//! The `permission_handler_query` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The selected permission handler of this server is asked to answer one node query. Only the
    /// mod `provider`, which provided the handler `handler` (through `permission_gather_handler`),
    /// is asked: no other mod is called, so no other mod sees the player's UUID or the contexts,
    /// and none can end the query before the provider answers. Answer with the node's typed value,
    /// or with the explicit error the handler chooses. The game refuses an answer from a mod other than the one that
    /// provided the handler, and treats an unanswered query as a handler failure: the query never
    /// falls back to a grant. `online` is the player's UUID when the player is on this server (ask
    /// `host.player(uuid)` for the handle) and `None` for an offline UUID, and `contexts` holds the
    /// contexts the caller supplied. `default` is the default the node was declared with: a handler
    /// with no rule of its own for the node answers from it (a `Level` against the player's operator
    /// level, `Nobody` as false, a `Value` as it is). A query this handler asks again from inside its answer fails
    /// both queries with `Recursive`. Side: server.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        answered_by = "mod",
        owner = "provider",
        neoforge = "none (no NeoForge counterpart; permission nodes are a Horizon service)"
    )]
    fn permission_handler_query(
        handler: &str,
        provider: &str,
        node: &mod_api::PermissionNode,
        default: &mod_api::NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[mod_api::PermissionContext],
        answer: &mut Option<
            mod_api::Answered<Result<mod_api::PermissionValue, mod_api::PermissionError>>,
        >,
    );
}
