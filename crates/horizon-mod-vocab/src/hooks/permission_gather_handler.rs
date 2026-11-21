//! The `permission_gather_handler` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once per logical server start, after the operator levels load and before the server
    /// starts: name each permission handler this mod provides. A handler answers the permission
    /// node queries of the server it is selected for (`permission-handler` in server.properties).
    /// Push the handler's id, `<namespace>:<path>` style or any non-empty text; an empty id is
    /// refused. Two handlers with one id, from any mods, are a registration error and the server
    /// answers no permission query. Answers come from `permission_handler_query`. Side: server.
    #[hook(
        side = "server",
        contract = "collect",
        out = "handlers",
        answered_by = "mod",
        neoforge = "none (no NeoForge counterpart; permission nodes are a Horizon service)"
    )]
    fn permission_gather_handler();
}
