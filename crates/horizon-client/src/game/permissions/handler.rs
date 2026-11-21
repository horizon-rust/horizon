//! Permission handlers the mods provide: their ids as gathered, the selection from
//! `permission-handler`, and the handler that asks the providing mod through
//! `permission_handler_query`. A failed answer is an explicit error, never a grant. Side: server.
//!
//! The permission service (`permission_service.rs`) provides `PermissionHandler`,
//! `register_handler_factory`, `finish_gathering` and `query`.

use super::permission_service::{
    DEFAULT_HANDLER_ID, ModAnswers, PermissionHandler, PermissionService,
};
use mod_api::{
    NodeDefault, PermissionContext, PermissionError, PermissionNode, PermissionValue, PermissionValueType,
};

/// A handler a mod provided. Every query goes to that mod alone: the other mods are not called
/// (`permission_handler_query` is asked of its provider only). Side: server.
pub(crate) struct ModPermissionHandler {
    id: String,
    owner: String,
}

impl ModPermissionHandler {
    /// The handler `id` that the mod `owner` provides.
    pub(crate) fn new(id: &str, owner: &str) -> Self {
        Self {
            id: id.to_owned(),
            owner: owner.to_owned(),
        }
    }
}

impl PermissionHandler for ModPermissionHandler {
    fn id(&self) -> &str {
        &self.id
    }

    /// Refuses without a grant: no answer, an answer from another mod, a value of the wrong type
    /// and a failed answer are all errors. Side: server.
    fn query(
        &mut self,
        node: &PermissionNode,
        default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
        answers: &mut dyn ModAnswers,
    ) -> Result<PermissionValue, PermissionError> {
        let Some(answered) = answers.answer(&self.id, &self.owner, node, default, online, uuid, contexts)
        else {
            log::warn!(
                "permission node \"{}\": handler \"{}\" got no answer from mod \"{}\"",
                node.name,
                self.id,
                self.owner
            );
            return Err(PermissionError::HandlerFailed);
        };
        if answered.mod_id != self.owner {
            log::warn!(
                "permission node \"{}\": handler \"{}\" was answered by mod \"{}\", not by its provider \"{}\"",
                node.name,
                self.id,
                answered.mod_id,
                self.owner
            );
            return Err(PermissionError::HandlerFailed);
        }
        match answered.answer {
            Ok(value) if value_matches(&value, &node.value_type) => Ok(value),
            Ok(_) => {
                log::warn!(
                    "permission node \"{}\": handler \"{}\" answered a value of another type",
                    node.name,
                    self.id
                );
                Err(PermissionError::WrongType)
            }
            Err(error) => {
                log::warn!(
                    "permission node \"{}\": handler \"{}\" failed in mod \"{}\"",
                    node.name,
                    self.id,
                    self.owner
                );
                Err(error)
            }
        }
    }
}

/// Whether `value` has the declared type `ty`. Side: server.
fn value_matches(value: &PermissionValue, ty: &PermissionValueType) -> bool {
    matches!(
        (value, ty),
        (PermissionValue::Boolean(_), PermissionValueType::Boolean)
            | (PermissionValue::Integer(_), PermissionValueType::Integer)
            | (PermissionValue::String(_), PermissionValueType::String)
            | (PermissionValue::Text(_), PermissionValueType::Text)
    )
}

/// What `permission-handler` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfiguredHandler {
    /// An empty setting, or `default`: the built-in handler answers.
    BuiltIn,
    /// A handler some mod gathered.
    Gathered(String),
    /// A handler id no mod gathered (a typo, or the handler mod missing or disabled). It is not
    /// the built-in handler: a custom handler was configured, none could be selected, and command
    /// usages with a node are refused.
    Missing(String),
}

/// The handler `permission-handler` asks for. A name no mod gathered is logged, naming the id,
/// and is `Missing`, not the built-in handler. Side: server.
pub(crate) fn configured_handler(configured: &str, gathered: &[(&str, &str)]) -> ConfiguredHandler {
    if configured.is_empty() || configured == DEFAULT_HANDLER_ID {
        return ConfiguredHandler::BuiltIn;
    }
    if gathered.iter().any(|(id, _)| *id == configured) {
        return ConfiguredHandler::Gathered(configured.to_owned());
    }
    log::warn!(
        "permission-handler \"{configured}\" names no gathered handler; command usages with a permission node are refused"
    );
    ConfiguredHandler::Missing(configured.to_owned())
}

/// Registers each gathered handler factory (`(id, providing mod)` pairs), selects one and publishes the service through
/// `finish_gathering`. A duplicate or empty id returns its error before anything is selected, so
/// the service stays uninitialized, refuses every query and is marked failed (`node_policy` is
/// `Refused`). A configured id no mod gathered selects the built-in handler for queries and
/// marks the service `Refused` for command usages with a node. The node set must already be
/// gathered. Side: server.
pub(crate) fn gather_handlers(
    service: &mut PermissionService,
    gathered: &[(&str, &str)],
    configured: &str,
) -> Result<(), PermissionError> {
    let registered = register_gathered(service, gathered);
    if registered.is_err() {
        service.mark_setup_failed();
        return registered;
    }
    let selected = match configured_handler(configured, gathered) {
        ConfiguredHandler::BuiltIn => None,
        ConfiguredHandler::Gathered(id) | ConfiguredHandler::Missing(id) => Some(id),
    };
    service.finish_gathering(selected.as_deref());
    Ok(())
}

fn register_gathered(
    service: &mut PermissionService,
    gathered: &[(&str, &str)],
) -> Result<(), PermissionError> {
    for (id, owner) in gathered {
        if id.is_empty() {
            return Err(PermissionError::InvalidInput);
        }
        let handler = ModPermissionHandler::new(id, owner);
        service.register_handler_factory(
            id,
            Box::new(
                move |_nodes: &[PermissionNode]| -> Box<dyn PermissionHandler> {
                    Box::new(handler)
                },
            ),
        )?;
    }
    Ok(())
}
