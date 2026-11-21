//! Typed permission nodes: the names a server mod asks about a player, the value types they
//! answer with, and the collector that declares them when a server starts. The service that
//! answers the queries is the game's (`horizon-client`, `game/permissions`).

use crate::{PermissionLevel, PlayerRef, ServerHost, Text};

/// The longest accepted node name, in bytes.
pub const MAX_NODE_NAME_LEN: usize = 256;
/// The most contexts one query accepts.
pub const MAX_CONTEXTS: usize = 32;
/// The longest accepted context key, in bytes.
pub const MAX_CONTEXT_KEY_LEN: usize = 128;

/// The value types a node answers with. A node's identity is its name together with this type.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, horizon_mod_api_macros::Wire,
)]
// exhaustive: these four types are the whole set a node can answer with.
pub enum PermissionValueType {
    /// A yes or no answer.
    Boolean,
    /// A signed 64-bit integer.
    Integer,
    /// A string.
    String,
    /// A text component.
    Text,
}

/// One typed answer. A query returns the value type its node was declared with, or an error.
// exhaustive: one variant per value type, so an answer always matches its node's type.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub enum PermissionValue {
    /// A Boolean answer.
    Boolean(bool),
    /// An integer answer.
    Integer(i64),
    /// A string answer.
    String(String),
    /// A text component answer.
    Text(Text),
}

impl PermissionValue {
    /// The value type this answer has.
    pub fn value_type(&self) -> PermissionValueType {
        match self {
            Self::Boolean(_) => PermissionValueType::Boolean,
            Self::Integer(_) => PermissionValueType::Integer,
            Self::String(_) => PermissionValueType::String,
            Self::Text(_) => PermissionValueType::Text,
        }
    }
}

/// A named, typed question a server mod asks about a player. Names are case-sensitive
/// dot-separated segments; two nodes with the same name and different types are different nodes.
// exhaustive: fields are public so a mod can name its node literally; use `PermissionNode::new`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, horizon_mod_api_macros::Wire)]
pub struct PermissionNode {
    /// The node's dot-separated name, for example `garden.harvest_rare`.
    pub name: String,
    /// The type of value this node answers with.
    pub value_type: PermissionValueType,
}

impl PermissionNode {
    /// A node named `name` that answers with `value_type`. Its name is checked when it is declared.
    pub fn new(name: impl Into<String>, value_type: PermissionValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
        }
    }
}

/// One context a caller supplies with a query: a key the mod chose and its value.
// exhaustive: fields are public so a caller can build a context literally; or use `PermissionContext::new`.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct PermissionContext {
    /// The key, a name the mod chose for what the context carries.
    pub key: String,
    /// The value of that key.
    pub value: PermissionValue,
}

impl PermissionContext {
    /// A context with the key `key` and the value `value`.
    pub fn new(key: impl Into<String>, value: PermissionValue) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }
}

/// Why a permission query or declaration was refused. A refusal never falls back to a grant.
// exhaustive: every refusal is one of these reasons, and a caller matches them all so none turns into a grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum PermissionError {
    /// The server has not gathered its nodes yet, or has closed. Side: server.
    NotInitialized,
    /// No node with this name and type is declared. Side: server.
    Unregistered,
    /// A node with this name and type is already declared. Side: server.
    DuplicateNode,
    /// A handler id is already gathered, or is the built-in default's. Side: server.
    DuplicateHandler,
    /// A value, a default, or an answer has the wrong type for its node. Side: server.
    WrongType,
    /// The player is not known to this server. Side: server.
    UnknownPlayer,
    /// The selected handler failed, trapped or panicked. Side: server.
    HandlerFailed,
    /// The query asked for itself while it was being answered. Side: server.
    Recursive,
    /// The input breaks a naming rule or a limit, or arrives after gathering closed. Side: server.
    InvalidInput,
}

impl std::fmt::Display for PermissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotInitialized => "permission service is not initialized",
            Self::Unregistered => "permission node is not registered",
            Self::DuplicateNode => "permission node is already registered",
            Self::DuplicateHandler => "permission handler id is already gathered",
            Self::WrongType => "permission value has the wrong type",
            Self::UnknownPlayer => "player is not known to this server",
            Self::HandlerFailed => "permission handler failed",
            Self::Recursive => "permission query is recursive",
            Self::InvalidInput => "permission input is invalid",
        })
    }
}

impl std::error::Error for PermissionError {}

/// What a node answers when no policy of the selected handler decides it: a Boolean node's
/// operator level or nobody, or the value of any other node.
// exhaustive: one variant per default shape; a Boolean node takes `Level` or `Nobody`, the others `Value`.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub enum NodeDefault {
    /// A Boolean node: granted to a player whose operator level is at least this.
    Level(PermissionLevel),
    /// Any other node: this value.
    Value(PermissionValue),
    /// A Boolean node: granted to no one, operators included, until a handler grants it. This is
    /// Bukkit's `default: false`.
    Nobody,
}

/// The collector `permission_gather_nodes` passes: it declares one typed node with its default.
/// Declarations are refused, never partly kept, when they break a rule.
pub trait PermissionNodes {
    /// Declare `node` with `default`. Side: server.
    fn add(&mut self, node: PermissionNode, default: NodeDefault) -> Result<(), PermissionError>;
}

/// Refuses a node name that is empty, longer than [`MAX_NODE_NAME_LEN`], or not made of
/// non-empty dot-separated segments of ASCII letters, digits, `_` and `-`. Side: server.
pub fn check_node_name(name: &str) -> Result<(), PermissionError> {
    let segment_ok = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    };
    if name.len() > MAX_NODE_NAME_LEN || !name.split('.').all(segment_ok) {
        return Err(PermissionError::InvalidInput);
    }
    Ok(())
}

/// Refuses more than [`MAX_CONTEXTS`] contexts, or a key that is empty or longer than
/// [`MAX_CONTEXT_KEY_LEN`]. Undeclared keys are not refused: handlers ignore what they do not read.
/// Side: server.
pub fn check_contexts(contexts: &[PermissionContext]) -> Result<(), PermissionError> {
    if contexts.len() > MAX_CONTEXTS {
        return Err(PermissionError::InvalidInput);
    }
    if contexts
        .iter()
        .any(|context| context.key.is_empty() || context.key.len() > MAX_CONTEXT_KEY_LEN)
    {
        return Err(PermissionError::InvalidInput);
    }
    Ok(())
}

impl PlayerRef {
    /// Asks the Boolean node `node` about this player through the selected handler. Refuses with
    /// `UnknownPlayer` for a handle that is no longer this server's player, and with `WrongType`
    /// for a node that does not answer Boolean. Side: server.
    pub fn permission_bool(
        self,
        host: &mut dyn ServerHost,
        node: &PermissionNode,
    ) -> Result<bool, PermissionError> {
        if node.value_type != PermissionValueType::Boolean {
            return Err(PermissionError::WrongType);
        }
        // The server's player of this UUID must be this very handle: a handle kept from an earlier
        // open of the world names the same UUID and is stale. A respawn keeps the handle equal.
        if host.player(self.uuid()) != Some(self) {
            return Err(PermissionError::UnknownPlayer);
        }
        match host.permission_query(node, self.uuid(), &[])? {
            PermissionValue::Boolean(answer) => Ok(answer),
            _ => Err(PermissionError::WrongType),
        }
    }
}
