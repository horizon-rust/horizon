//! Mod command paths that ask a permission node (design note "Command paths").
//!
//! Only while a custom permission handler is selected does a usage's node decide. Then the
//! usage's operator level is set aside, the node's answer decides who sees the path in their
//! command tree and who may run it, and an error refuses the run instead of falling back to a
//! level. With the built-in handler a usage with a node keeps its level, as any command does. When
//! the permission setup failed or `permission-handler` names no handler a mod provided, a usage
//! with a node is left out of the tree: it never runs at a level.
//! Vanilla's commands and mod commands without a node never reach this module's decisions.

use super::{CommandArguments, runs_at, usage_nodes};
use crate::game::permission_service::NodePolicy;
use mod_api::{
    Command, CommandSource, NodeCommand, PermissionError, PermissionNode, PermissionValueType,
};
use std::collections::HashSet;

/// Something that answers a node for a player's UUID: the game's service in production.
pub(super) trait NodeAnswers {
    /// The node's answer for a player's `uuid`, online or offline.
    fn answer(&mut self, node: &PermissionNode, uuid: u128) -> Result<bool, PermissionError>;
}

/// The game as the answerer of nodes: each answer goes through its selected handler.
pub(super) struct GameNodes<'a, 'g>(pub(super) &'a mut crate::game::Game<'g>);

impl NodeAnswers for GameNodes<'_, '_> {
    fn answer(&mut self, node: &PermissionNode, uuid: u128) -> Result<bool, PermissionError> {
        self.0.permission_query_bool(node, uuid)
    }
}

/// The runnable paths a usage names, each as the names of the dispatcher nodes the command tree
/// builds for it (`pay <player:player> <amount:integer>` is `pay`, `player`, `amount`). The names
/// come from the parser that builds the tree (`usage_nodes`), not from the usage's text, so a
/// name that keeps a leading delimiter (`<<x:word>` names `<x`) is the name the dispatcher has.
/// A path ends where the next word is optional or the usage ends, so `balance [player:player]`
/// runs at `balance` and at `balance player`. `None` when the tree would refuse the usage.
pub(super) fn runnable_paths(usage: &str, custom: &CommandArguments) -> Option<Vec<Vec<String>>> {
    let nodes = usage_nodes(usage, custom).ok()?;
    let mut names = Vec::new();
    let mut paths = Vec::new();
    for (index, (name, _, _)) in nodes.iter().enumerate() {
        names.push(name.clone());
        if runs_at(&nodes, index) {
            paths.push(names.clone());
        }
    }
    Some(paths)
}

/// The usages a tree is built from, and the paths a node decides.
#[derive(Clone, Default)]
pub(super) struct NodeGate {
    /// Each gated runnable path, the node that answers it, and the indexes (in the usages
    /// [`split_usages`] returns) of the usages that name the path.
    paths: Vec<(Vec<String>, PermissionNode, Vec<usize>)>,
}

impl NodeGate {
    /// Whether no path asks a node.
    pub(super) fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The node that decides the path (dispatcher names, as [`path_names`] gives them).
    pub(super) fn node(&self, path: &[String]) -> Option<&PermissionNode> {
        self.paths
            .iter()
            .find(|(gated, _, _)| gated.as_slice() == path)
            .map(|(_, node, _)| node)
    }

    /// Drops the paths of usages the tree left out (`command_tree_accepting`'s rejected
    /// indexes), so a refused usage's path stays ungated and unhidden. A path another accepted
    /// usage also names stays.
    pub(super) fn without_usages(&mut self, rejected: &[usize]) {
        for (_, _, usages) in &mut self.paths {
            usages.retain(|usage| !rejected.contains(usage));
        }
        self.paths.retain(|(_, _, usages)| !usages.is_empty());
    }

    /// The gated paths the node refuses `uuid` for, which the player's tree leaves out. An
    /// error refuses the path too, and is logged. Each node is asked once for the player, however
    /// many paths it gates: a tree send makes one handler call per node, not per path.
    pub(super) fn denied_for<A: NodeAnswers>(
        &self,
        answers: &mut A,
        uuid: u128,
    ) -> Vec<Vec<String>> {
        let mut asked: Vec<(&str, bool)> = Vec::new();
        let mut denied = Vec::new();
        for (path, node, _) in &self.paths {
            let allowed = match asked.iter().find(|(name, _)| *name == node.name) {
                Some((_, allowed)) => *allowed,
                None => {
                    let allowed = allowed(answers, node, uuid);
                    asked.push((&node.name, allowed));
                    allowed
                }
            };
            if !allowed {
                denied.push(path.clone());
            }
        }
        denied
    }
}

/// Whether the node allows `uuid`; an error does not, and is logged with the node's name.
fn allowed<A: NodeAnswers>(answers: &mut A, node: &PermissionNode, uuid: u128) -> bool {
    match answers.answer(node, uuid) {
        Ok(answer) => answer,
        Err(error) => {
            log::warn!(
                "Horizon: permission node `{}` failed for a command ({error:?}); the path is refused",
                node.name
            );
            false
        }
    }
}

/// Whether a run of a gated path is refused for `origin`. Only a player has a UUID to ask about;
/// any other origin (console, command block, function) is refused, and the refusal is logged.
pub(super) fn refuses<A: NodeAnswers>(
    answers: &mut A,
    node: &PermissionNode,
    origin: &CommandSource,
) -> bool {
    let CommandSource::Player { player, .. } = origin else {
        log::warn!(
            "Horizon: permission node `{}` needs a player; a command from this source is refused",
            node.name
        );
        return true;
    };
    !allowed(answers, node, player.uuid())
}

/// The dispatcher's names along a parsed command (`Context::nodes`), without the root.
pub(super) fn path_names<R>(
    dispatcher: &horizon_client::command::Dispatcher,
    parsed: &[(usize, R)],
) -> Vec<String> {
    parsed
        .iter()
        .filter_map(|(id, _)| dispatcher.nodes.get(*id))
        .map(|node| node.name().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Splits the mods' usages (`plain` from `register_commands`, `with_nodes` from
/// `register_node_commands`) into the ones the tree is built from and the gate. The tree's
/// usages are the plain ones, then the gated ones that stay. With the built-in handler every
/// usage goes on unchanged (its level decides) and the gate is empty. With a mod's handler, a
/// gated usage goes on at level 0 and its runnable paths are gated. When the policy is
/// `Refused` (the permission setup failed, or `permission-handler` names no gathered handler)
/// every gated usage is left out and named in the problems: it never runs at an operator level.
/// Usages are also left out for a node that is not Boolean, a path shared with an ungated
/// usage, or a path another node gates.
pub(super) fn split_usages(
    plain: &[Command],
    with_nodes: &[NodeCommand],
    policy: NodePolicy,
    custom: &CommandArguments,
) -> (Vec<Command>, NodeGate, Vec<String>) {
    let mut out = Vec::with_capacity(plain.len() + with_nodes.len());
    let mut gate = NodeGate::default();
    let mut problems = Vec::new();
    let ungated: HashSet<Vec<String>> = plain
        .iter()
        .filter_map(|command| runnable_paths(&command.usage, custom))
        .flatten()
        .collect();
    out.extend(plain.iter().cloned());
    for entry in with_nodes {
        let (command, node) = (&entry.command, &entry.node);
        if !matches!(node.value_type, PermissionValueType::Boolean) {
            problems.push(format!(
                "`{}`: permission node `{}` is not a Boolean node",
                command.usage, node.name
            ));
            continue;
        }
        match policy {
            NodePolicy::Levels => {
                out.push(command.clone());
                continue;
            }
            NodePolicy::Refused => {
                problems.push(format!(
                    "`{}`: the permission setup failed or its handler is missing, so the usage with node `{}` is refused",
                    command.usage, node.name
                ));
                continue;
            }
            NodePolicy::Handler => {}
        }
        // A usage the tree's parser refuses is left out: it has no dispatcher node to gate.
        let Some(paths) = runnable_paths(&command.usage, custom) else {
            problems.push(format!(
                "`{}`: the command tree cannot read this usage, so it is left out",
                command.usage
            ));
            continue;
        };
        if let Some(path) = paths.iter().find(|path| ungated.contains(*path)) {
            problems.push(format!(
                "`{}`: its path `{}` is also an ungated usage's, so it is left out",
                command.usage,
                path.join(" ")
            ));
            continue;
        }
        if let Some(path) = paths.iter().find(|path| {
            gate.paths
                .iter()
                .any(|(gated, other, _)| gated == *path && other.name != node.name)
        }) {
            problems.push(format!(
                "`{}`: its path `{}` is gated by another permission node, so it is left out",
                command.usage,
                path.join(" ")
            ));
            continue;
        }
        let index = out.len();
        for path in paths {
            match gate.paths.iter_mut().find(|(gated, _, _)| *gated == path) {
                Some((_, _, usages)) => usages.push(index),
                None => gate.paths.push((path, node.clone(), vec![index])),
            }
        }
        out.push(Command::new(&command.usage));
    }
    (out, gate, problems)
}
