//! Permission nodes for server mods: gathering them and the handler when a server starts, the
//! host's query with its recursion guard, and dropping the service when the server closes.

use super::Game;
use super::handler::gather_handlers;
use super::permission_service::{LevelSource, ModAnswer, ModAnswers, PermissionService};
use mod_api::{
    NodeDefault, PermissionContext, PermissionError, PermissionLevel, PermissionNode,
    PermissionNodes, PermissionValue, PermissionValueType,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// The live operator level (0 to 4) of each online player the game is asking about, by UUID. The
/// built-in handler reads it before the operator list: a dedicated server's level is in that list,
/// an integrated or LAN server has none, and its players' levels exist only on the player.
type LiveLevels = Arc<Mutex<HashMap<u128, u8>>>;

/// Whether a query is being answered, so one asked again meanwhile is refused as recursive.
/// Side: server.
#[derive(Default)]
pub(crate) struct QueryState {
    active: bool,
    tripped: bool,
    live_levels: LiveLevels,
}

impl QueryState {
    /// Starts a query. Refused with `Recursive` while another is being answered, and the
    /// enclosing query is then refused too when it ends (`leave`).
    fn enter(&mut self) -> Result<(), PermissionError> {
        if self.active {
            self.tripped = true;
            return Err(PermissionError::Recursive);
        }
        self.active = true;
        Ok(())
    }

    /// Ends the query `enter` started; a query asked again while it was answered fails it too.
    fn leave<T>(&mut self, result: Result<T, PermissionError>) -> Result<T, PermissionError> {
        self.active = false;
        if std::mem::take(&mut self.tripped) {
            return Err(PermissionError::Recursive);
        }
        result
    }

    /// The built-in handler's operator levels: an online player's live level first, then
    /// `listed` (the dedicated server's operator list by UUID), else level 0. An integrated or LAN
    /// server has no list, so its players' levels exist only as their live level.
    pub(crate) fn level_source(
        &self,
        listed: impl Fn(u128) -> Option<i32> + Send + Sync + 'static,
    ) -> LevelSource {
        let live = Arc::clone(&self.live_levels);
        Arc::new(move |online, uuid| {
            let live_level = online
                .and_then(|_| live.lock().ok())
                .and_then(|levels| levels.get(&uuid).copied())
                .map(i32::from);
            live_level
                .or_else(|| listed(uuid))
                .map_or(PermissionLevel::All, PermissionLevel::from_level)
        })
    }

    fn set_live_level(&self, uuid: u128, level: Option<u8>) {
        if let Ok(mut levels) = self.live_levels.lock() {
            match level {
                Some(level) => levels.insert(uuid, level),
                None => levels.remove(&uuid),
            };
        }
    }
}

/// What a query needs of the game: its service and guard, who is online, and the mods' answers.
/// The game implements it; the tests do with a fake that can ask again from inside an answer.
pub(crate) trait PermissionHost: ModAnswers {
    /// The service and the guard of the open server.
    fn permission_parts(&mut self) -> (&mut PermissionService, &mut QueryState);

    /// The live operator level of `uuid`, or `None` when the player is not on this server.
    fn online_level(&self, uuid: u128) -> Option<u8>;
}

/// Answers `node` for `uuid` through the server's selected handler. `online` is `uuid` when the
/// player is on this server. The service is taken out of the host while it answers, because a mod
/// handler is handed the host: a query it asks again finds the guard set and fails with
/// `Recursive`, and so does the enclosing query. Side: server.
pub(crate) fn run_query<H: PermissionHost>(
    host: &mut H,
    node: &PermissionNode,
    uuid: u128,
    contexts: &[PermissionContext],
) -> Result<PermissionValue, PermissionError> {
    let level = host.online_level(uuid);
    let (service, state) = host.permission_parts();
    state.enter()?;
    state.set_live_level(uuid, level);
    let mut taken = std::mem::take(service);
    let result = taken.query_with(node, level.map(|_| uuid), uuid, contexts, host);
    let (service, state) = host.permission_parts();
    *service = taken;
    state.leave(result)
}

/// `run_query` for a Boolean node. Refused with `WrongType` for any other node, before any handler
/// runs. Side: server.
pub(crate) fn run_query_bool<H: PermissionHost>(
    host: &mut H,
    node: &PermissionNode,
    uuid: u128,
) -> Result<bool, PermissionError> {
    if node.value_type != PermissionValueType::Boolean {
        return Err(PermissionError::WrongType);
    }
    match run_query(host, node, uuid, &[])? {
        PermissionValue::Boolean(answer) => Ok(answer),
        _ => Err(PermissionError::WrongType),
    }
}

/// The collector the mods' `permission_gather_nodes` fills. It declares into the service and
/// remembers a node declared twice, so the gather as a whole can fail, not only that `add`.
struct NodeCollector {
    service: PermissionService,
    duplicate: bool,
}

impl PermissionNodes for NodeCollector {
    fn add(&mut self, node: PermissionNode, default: NodeDefault) -> Result<(), PermissionError> {
        let result = self.service.register_node(node, default);
        if result == Err(PermissionError::DuplicateNode) {
            self.duplicate = true;
        }
        result
    }
}

/// Runs `gather` (the mods' hook) over `service` and returns the service with the declared
/// nodes, or `DuplicateNode` when any mod declared a node another had declared. A failed gather
/// returns no service, so nothing half-registered can be kept. Side: server.
pub(super) fn gathered_nodes(
    service: PermissionService,
    gather: impl FnOnce(&mut dyn PermissionNodes),
) -> Result<PermissionService, PermissionError> {
    let mut collector = NodeCollector {
        service,
        duplicate: false,
    };
    gather(&mut collector);
    if collector.duplicate {
        return Err(PermissionError::DuplicateNode);
    }
    Ok(collector.service)
}

impl ModAnswers for Game<'_> {
    fn answer(
        &mut self,
        handler: &str,
        provider: &str,
        node: &PermissionNode,
        default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
    ) -> Option<ModAnswer> {
        let mut answer = None;
        self.native_permission_handler_query(mod_api::__private::PermissionHandlerQueryCall {
            handler,
            provider,
            node,
            default,
            online,
            uuid,
            contexts,
            answer: &mut answer,
        });
        answer.map(|answered| ModAnswer {
            mod_id: answered.mod_id.to_owned(),
            answer: answered.answer,
        })
    }
}

impl PermissionHost for Game<'_> {
    fn permission_parts(&mut self) -> (&mut PermissionService, &mut QueryState) {
        (
            &mut self.permission_service,
            &mut self.permission_query_state,
        )
    }

    /// A guest still joining counts: its first command tree is decided while it is not placed yet,
    /// and must see the same online state as the command it runs right after.
    fn online_level(&self, uuid: u128) -> Option<u8> {
        self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| guest.uuid == uuid && guest.participates_online())
                .map(|guest| guest.permission)
        })
    }
}

impl Game<'_> {
    /// Gathers this server's permission nodes and handlers and selects the handler
    /// `configured_handler` names (`permission-handler`; empty selects the built-in one). Asked
    /// once per server start, after the operator list is read and before `server_starting`. The
    /// built-in handler reads an online player's live operator level, else the operator list by
    /// UUID. Side: server.
    pub(super) fn gather_permission_nodes(&mut self, configured_handler: &str) {
        let lists = self.server_lists.clone();
        let levels = self.permission_query_state.level_source(move |uuid| {
            let lists = lists.as_ref()?.lock().ok()?;
            lists
                .ops
                .entries()
                .into_iter()
                .find(|entry| entry.user.id == uuid)
                .map(|entry| entry.level)
        });
        let mut service = PermissionService::new();
        service.set_level_source(levels);
        let mut service = match gathered_nodes(service, Self::native_permission_gather_nodes) {
            Ok(service) => service,
            Err(error) => {
                // Two mods declared the same node: the gather failed as a whole, so no handler is
                // selected and the service stays empty and uninitialized, refusing every query. It
                // is marked failed, so command usages with a node are refused too, not run at
                // their operator levels.
                log::error!(
                    "permission nodes were not gathered ({error}): no permission query is answered and command usages with a permission node are refused"
                );
                self.permission_service = PermissionService::failed();
                return;
            }
        };
        let mut handlers = Vec::new();
        self.native_permission_gather_handler(&mut handlers);
        let gathered: Vec<(&str, &str)> = handlers
            .iter()
            .map(|handler| (handler.answer.as_str(), handler.mod_id))
            .collect();
        if let Err(error) = gather_handlers(&mut service, &gathered, configured_handler) {
            log::error!(
                "permission handlers were not gathered ({error}): no permission query is answered and command usages with a permission node are refused"
            );
        }
        self.permission_service = service;
    }

    /// Drops this server's permission service when the server closes; a reopened server gathers
    /// afresh. Side: server.
    pub(super) fn reset_permission_service(&mut self) {
        self.permission_service = PermissionService::new();
        self.permission_query_state = QueryState::default();
    }

    /// Answers `node` for `uuid` through this server's selected handler. Online and offline UUIDs
    /// are answered. Side: server.
    pub(super) fn permission_query(
        &mut self,
        node: &PermissionNode,
        uuid: u128,
        contexts: &[PermissionContext],
    ) -> Result<PermissionValue, PermissionError> {
        run_query(self, node, uuid, contexts)
    }

    /// `permission_query` for a Boolean node. Side: server.
    pub(super) fn permission_query_bool(
        &mut self,
        node: &PermissionNode,
        uuid: u128,
    ) -> Result<bool, PermissionError> {
        run_query_bool(self, node, uuid)
    }
}
