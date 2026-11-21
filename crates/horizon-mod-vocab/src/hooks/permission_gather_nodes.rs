//! The `permission_gather_nodes` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Declare the typed permission nodes this server's mods answer for. Asked once per server
    /// start, after the operator list is read and before the server starts; not again on a join,
    /// a resource reload or a data pack change. Add each node with its default: a Boolean node
    /// takes the operator level that grants it by default, any other node its value. A node is
    /// identified by its name and type, so the same name with two types is two nodes. A
    /// declaration that breaks a rule is refused with a `PermissionError` and keeps nothing of
    /// that declaration; a refused declaration is not a crash. Server-side.
    #[hook(side = "server", contract = "collect", out = "nodes")]
    fn permission_gather_nodes();
}
