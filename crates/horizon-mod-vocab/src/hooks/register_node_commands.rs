//! The `register_node_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server commands whose permission is a Boolean permission node: push
    /// `Command::new(usage).with_node(&node)`. Asked in the same places and as often as
    /// `register_commands`, which keeps its API 0.2 payload and takes no node; a mod built for
    /// 0.2 that registers commands loads as before. Both kinds of usage share one command tree and
    /// its rules (words, argument kinds, vanilla names left out).
    ///
    /// While a custom permission handler is selected (`permission-handler` in server.properties),
    /// the handler's answer for the node decides who sees and who runs the usage, and a handler
    /// error refuses the run: the usage's level is not asked. With the built-in handler the usage
    /// keeps its level, as a plain command does. A node that is not Boolean refuses the usage. Side: server.
    #[hook(side = "server", contract = "collect", out = "commands")]
    fn register_node_commands();
}
