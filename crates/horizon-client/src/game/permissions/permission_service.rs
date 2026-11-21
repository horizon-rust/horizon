//! The permission service: typed node declaration, handler selection and queries. It lives
//! with one open server and answers only server-side; every `PermissionError` it returns is an
//! explicit refusal, and no refusal falls back to a grant.

use mod_api::{
    NodeDefault, PermissionContext, PermissionError, PermissionLevel, PermissionNode,
    PermissionNodes, PermissionValue, PermissionValueType, check_contexts, check_node_name,
};
use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

/// The id of the built-in handler, which answers every node from its default. No mod can gather a
/// handler under this id.
pub(crate) const DEFAULT_HANDLER_ID: &str = "default";

/// The longest accepted handler id, in bytes.
const MAX_HANDLER_ID_LEN: usize = 128;

/// The operator level of a player, asked by UUID. `online` is the UUID when the player is on this
/// server. The game supplies it when the server starts; it reads an online player's live level.
pub(crate) type LevelSource = Arc<dyn Fn(Option<u128>, u128) -> PermissionLevel + Send + Sync>;

/// Builds a handler from the complete, validated set of declared nodes.
pub(crate) type HandlerFactory =
    Box<dyn FnOnce(&[PermissionNode]) -> Box<dyn PermissionHandler> + Send>;

/// What a mod answered to one handler query, and the mod that gave the answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModAnswer {
    /// The `[mod] id` of the mod that answered.
    pub(crate) mod_id: String,
    /// Its answer: the node's typed value, or the explicit error it chose.
    pub(crate) answer: Result<PermissionValue, PermissionError>,
}

/// Asks the mods for one query of a handler they provide. The answer names the mod that gave
/// it; `None` means no mod answered. The game implements it by firing `permission_handler_query`,
/// which hands the mod the game's own server host, so the service never holds the game.
/// Side: server.
pub(crate) trait ModAnswers {
    /// Asks the mod `provider`, which provided handler `handler`, for `node` (declared with
    /// `default`), for the player
    /// `uuid` (`online` is the same UUID when the player is on this server, `None` when offline)
    /// with the supplied `contexts`. No other mod is called, so none sees the UUID or the contexts.
    fn answer(
        &mut self,
        handler: &str,
        provider: &str,
        node: &PermissionNode,
        default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
    ) -> Option<ModAnswer>;
}

/// The one policy that answers every node of a server. It is `Send` because the service that
/// holds it lives in the game.
pub(crate) trait PermissionHandler: Send {
    /// The handler's id, as the server configuration names it.
    fn id(&self) -> &str;

    /// Answers `node`, declared with `default`, for `uuid` (and `online` when the player is on this
    /// server). Returns the node's declared type or an explicit error. A handler a mod provides
    /// asks `answers`, handing it `default`.
    /// Side: server.
    fn query(
        &mut self,
        node: &PermissionNode,
        default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
        answers: &mut dyn ModAnswers,
    ) -> Result<PermissionValue, PermissionError>;
}

type NodeKey = (String, PermissionValueType);

/// Who decides the command usages that ask a permission node, as the service stands after its
/// setup. Side: server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodePolicy {
    /// The built-in handler answers: a usage with a node keeps its operator level.
    Levels,
    /// A handler a mod provides answers: the node decides and its level is not asked.
    Handler,
    /// The setup failed (a node or a handler declared twice) or `permission-handler` names a
    /// handler no mod provides: a usage with a node is refused, never run at its level.
    Refused,
}

/// Declared nodes, gathered handler factories and the selected handler of one server. A server
/// owns one, created when it starts and dropped when it closes.
pub(crate) struct PermissionService {
    nodes: BTreeMap<NodeKey, (PermissionNode, NodeDefault)>,
    factories: BTreeMap<String, HandlerFactory>,
    levels: Option<LevelSource>,
    handler: Option<Box<dyn PermissionHandler>>,
    custom_handler: bool,
    /// A duplicate node or handler failed the setup: the service stays uninitialized.
    setup_failed: bool,
    /// `permission-handler` named a handler no mod gathered: a custom handler was configured,
    /// but none was selected.
    handler_missing: bool,
}

impl Default for PermissionService {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionService {
    /// An empty service that answers nothing until its gathering finishes.
    pub(crate) fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            factories: BTreeMap::new(),
            levels: None,
            handler: None,
            custom_handler: false,
            setup_failed: false,
            handler_missing: false,
        }
    }

    /// A service whose setup failed. It stays uninitialized, so it refuses every query, and
    /// `node_policy` is `Refused`: an uninitialized service that merely has not gathered yet is
    /// a different state.
    pub(crate) fn failed() -> Self {
        let mut service = Self::new();
        service.mark_setup_failed();
        service
    }

    /// Records that gathering failed (a node or a handler declared twice).
    pub(crate) fn mark_setup_failed(&mut self) {
        self.setup_failed = true;
    }

    /// Whether gathering has finished, so queries are answered.
    pub(crate) fn is_initialized(&self) -> bool {
        self.handler.is_some()
    }

    /// The operator level source the built-in handler reads. Set it before `finish_gathering`.
    pub(crate) fn set_level_source(&mut self, levels: LevelSource) {
        self.levels = Some(levels);
    }

    /// Declares `node` with `default`. Refused after gathering has finished, for a bad name, for
    /// a default of the wrong shape, and for a node already declared with the same name and type.
    /// Side: server.
    pub(crate) fn register_node(
        &mut self,
        node: PermissionNode,
        default: NodeDefault,
    ) -> Result<(), PermissionError> {
        if self.is_initialized() {
            return Err(PermissionError::InvalidInput);
        }
        check_node_name(&node.name)?;
        check_default(node.value_type, &default)?;
        let key = (node.name.clone(), node.value_type);
        if self.nodes.contains_key(&key) {
            return Err(PermissionError::DuplicateNode);
        }
        self.nodes.insert(key, (node, default));
        Ok(())
    }

    /// Gathers a handler factory under `id`. Refused after gathering has finished, for an empty or
    /// over-long id, and for an id already gathered or reserved for the built-in handler.
    /// Side: server.
    pub(crate) fn register_handler_factory(
        &mut self,
        id: &str,
        factory: HandlerFactory,
    ) -> Result<(), PermissionError> {
        if self.is_initialized() {
            return Err(PermissionError::InvalidInput);
        }
        if id.is_empty() || id.len() > MAX_HANDLER_ID_LEN {
            return Err(PermissionError::InvalidInput);
        }
        if id == DEFAULT_HANDLER_ID || self.factories.contains_key(id) {
            return Err(PermissionError::DuplicateHandler);
        }
        self.factories.insert(id.to_owned(), factory);
        Ok(())
    }

    /// Ends gathering: selects the configured factory, or the built-in handler when none is
    /// configured, builds the handler from the declared nodes and starts answering. A configured
    /// id no mod gathered (logged by name) keeps the built-in handler for queries, but is its own
    /// state: `node_policy` is `Refused`, so a command usage with a node does not run at an
    /// operator level. A second call does nothing.
    pub(crate) fn finish_gathering(&mut self, configured_handler: Option<&str>) {
        if self.is_initialized() {
            return;
        }
        let nodes: Vec<PermissionNode> = self.nodes.values().map(|(n, _)| n.clone()).collect();
        let selected = match configured_handler {
            None | Some(DEFAULT_HANDLER_ID) => None,
            Some(id) => match self.factories.remove(id) {
                Some(factory) => Some(factory),
                None => {
                    log::warn!(
                        "permission-handler \"{id}\" is not gathered: the built-in default handler answers queries, and command usages with a permission node are refused"
                    );
                    self.handler_missing = true;
                    None
                }
            },
        };
        self.factories.clear();
        let selected_custom = selected.is_some();
        let handler: Box<dyn PermissionHandler> = match selected {
            Some(factory) => factory(&nodes),
            None => Box::new(DefaultHandler {
                defaults: self
                    .nodes
                    .iter()
                    .map(|(key, (_, default))| (key.clone(), default.clone()))
                    .collect(),
                levels: self.levels.clone(),
            }),
        };
        self.custom_handler = selected_custom;
        self.handler = Some(handler);
    }

    /// Who decides the command usages that ask a node: the built-in handler's levels, a mod's
    /// handler, or nobody (`Refused`). A failed setup and a configured handler no mod provided
    /// are `Refused`; a service that has not gathered yet is `Levels`, as before there was a
    /// service. Side: server.
    pub(crate) fn node_policy(&self) -> NodePolicy {
        if self.setup_failed || self.handler_missing {
            NodePolicy::Refused
        } else if self.custom_handler {
            NodePolicy::Handler
        } else {
            NodePolicy::Levels
        }
    }

    /// Answers `node` for `uuid`. `online` is the UUID when the player is on this server; an
    /// offline UUID is answered too. Refused before initialization, for unregistered nodes
    /// (before the handler runs), for bad contexts, and for an answer of another type than the
    /// node's. A handler's failure or panic is `HandlerFailed`. A query asked again while one is
    /// answered never reaches this method: the game refuses it (`native_permissions::run_query`).
    /// Side: server.
    pub(crate) fn query_with(
        &mut self,
        node: &PermissionNode,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
        answers: &mut dyn ModAnswers,
    ) -> Result<PermissionValue, PermissionError> {
        if !self.is_initialized() {
            return Err(PermissionError::NotInitialized);
        }
        check_contexts(contexts)?;
        let Some((_, default)) = self.nodes.get(&(node.name.clone(), node.value_type)) else {
            return Err(PermissionError::Unregistered);
        };
        let Some(handler) = self.handler.as_mut() else {
            return Err(PermissionError::NotInitialized);
        };
        let answer = catch_unwind(AssertUnwindSafe(|| {
            handler.query(node, default, online, uuid, contexts, answers)
        }))
        .unwrap_or(Err(PermissionError::HandlerFailed));
        let value = match answer {
            Ok(value) => value,
            Err(error) => {
                log::warn!(
                    "permission node {} (handler {}): {error}",
                    node.name,
                    handler.id()
                );
                return Err(error);
            }
        };
        if value.value_type() != node.value_type {
            log::warn!(
                "permission node {} (handler {}): answered with another value type",
                node.name,
                handler.id()
            );
            return Err(PermissionError::WrongType);
        }
        Ok(value)
    }

    /// `query_with` for a service whose handler asks no mod, as in the tests.
    #[cfg(test)]
    pub(crate) fn query(
        &mut self,
        node: &PermissionNode,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
    ) -> Result<PermissionValue, PermissionError> {
        self.query_with(node, online, uuid, contexts, &mut NoMods)
    }

    /// `query` for a node that answers Boolean, refused with `WrongType` for any other node before
    /// the handler runs, as in the tests.
    #[cfg(test)]
    pub(crate) fn query_bool(
        &mut self,
        node: &PermissionNode,
        online: Option<u128>,
        uuid: u128,
    ) -> Result<bool, PermissionError> {
        if node.value_type != PermissionValueType::Boolean {
            return Err(PermissionError::WrongType);
        }
        match self.query(node, online, uuid, &[])? {
            PermissionValue::Boolean(answer) => Ok(answer),
            _ => Err(PermissionError::WrongType),
        }
    }
}

/// Answers nothing, for tests of handlers that never ask a mod.
#[cfg(test)]
pub(crate) struct NoMods;

#[cfg(test)]
impl ModAnswers for NoMods {
    fn answer(
        &mut self,
        _handler: &str,
        _provider: &str,
        _node: &PermissionNode,
        _default: &NodeDefault,
        _online: Option<u128>,
        _uuid: u128,
        _contexts: &[PermissionContext],
    ) -> Option<ModAnswer> {
        None
    }
}

impl PermissionNodes for PermissionService {
    fn add(&mut self, node: PermissionNode, default: NodeDefault) -> Result<(), PermissionError> {
        self.register_node(node, default)
    }
}

/// A Boolean node takes a level default or `Nobody`, any other node a value of its own type.
fn check_default(
    value_type: PermissionValueType,
    default: &NodeDefault,
) -> Result<(), PermissionError> {
    let fits = match (value_type, default) {
        (PermissionValueType::Boolean, NodeDefault::Level(_) | NodeDefault::Nobody) => true,
        (PermissionValueType::Boolean, NodeDefault::Value(_)) => false,
        (_, NodeDefault::Level(_) | NodeDefault::Nobody) => false,
        (expected, NodeDefault::Value(value)) => value.value_type() == expected,
    };
    if fits {
        Ok(())
    } else {
        Err(PermissionError::WrongType)
    }
}

/// The built-in handler: a Boolean node is granted when the player's operator level is at least
/// its default level, and a `Nobody` node is never granted; any other node answers its default value. Nothing here reads the world.
struct DefaultHandler {
    defaults: BTreeMap<NodeKey, NodeDefault>,
    levels: Option<LevelSource>,
}

impl PermissionHandler for DefaultHandler {
    fn id(&self) -> &str {
        DEFAULT_HANDLER_ID
    }

    fn query(
        &mut self,
        node: &PermissionNode,
        _default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        _contexts: &[PermissionContext],
        _answers: &mut dyn ModAnswers,
    ) -> Result<PermissionValue, PermissionError> {
        let default = self
            .defaults
            .get(&(node.name.clone(), node.value_type))
            .ok_or(PermissionError::Unregistered)?;
        Ok(match default {
            NodeDefault::Level(required) => {
                let level = self
                    .levels
                    .as_ref()
                    .map_or(PermissionLevel::All, |levels| levels(online, uuid));
                PermissionValue::Boolean(level >= *required)
            }
            NodeDefault::Nobody => PermissionValue::Boolean(false),
            NodeDefault::Value(value) => value.clone(),
        })
    }
}
