//! The `horizon-ctl` commands a crate answers in the game ([`Hooks::control`](crate::Hooks::control)),
//! described as plain data. The game's `hello` sends these descriptions, so `horizon-ctl` offers a
//! crate's commands (in its help and as MCP tools) only when the running game has that crate.

/// The commands one crate answers: what [`Hooks::control_commands`](crate::Hooks::control_commands)
/// returns.
#[derive(Clone, Copy, Debug)]
pub struct Commands {
    /// The command group, as `hello` lists it in `features`.
    pub group: &'static str,
    /// The lines the game's own `help` adds for them.
    pub help: &'static [&'static str],
    /// Each command, as `horizon-ctl` describes it.
    pub commands: &'static [Command],
}

/// One command, as `horizon-ctl help COMMAND` shows it and its MCP tool takes it.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub name: &'static str,
    /// `name` and its arguments, such as `friends host|stop|status`.
    pub usage: &'static str,
    /// One line, lowercase, no final full stop.
    pub summary: &'static str,
    /// More, shown by `--help` and in the MCP tool's description; may be empty.
    pub details: &'static str,
    /// Whole command lines, starting with `horizon-ctl`.
    pub examples: &'static [&'static str],
    pub params: &'static [Param],
    /// Offered as an MCP tool.
    pub mcp: bool,
    /// Reaches other players or the network: MCP's `openWorldHint`, used only when the player asks.
    pub open_world: bool,
}

/// One argument of a [`Command`].
#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    /// Sent as `--name value` (`--name` alone for booleans), not by position.
    pub flag: bool,
    pub help: &'static str,
}

/// What a [`Param`] takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Int,
    Num,
    Str,
    Bool,
    /// One of these words.
    Enum(&'static [&'static str]),
    /// The rest of the line, spaces included.
    Rest,
}

impl Kind {
    /// The name `hello` gives it: `int`, `num`, `str`, `bool`, `enum` or `rest`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Int => "int",
            Kind::Num => "num",
            Kind::Str => "str",
            Kind::Bool => "bool",
            Kind::Enum(_) => "enum",
            Kind::Rest => "rest",
        }
    }
}
