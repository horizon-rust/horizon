//! Unit tests of the command paths a permission node decides (`permission_gate`).

use super::permission_gate::{NodeAnswers, NodeGate, refuses, runnable_paths, split_usages};
use super::{command_arguments, command_tree_accepting};
use crate::game::permission_service::NodePolicy;
use mod_api::{
    Command, CommandSource, PermissionError, PermissionLevel, PermissionNode, PermissionValueType,
};

fn boolean(name: &str) -> PermissionNode {
    PermissionNode {
        name: name.to_owned(),
        value_type: PermissionValueType::Boolean,
    }
}

fn integer(name: &str) -> PermissionNode {
    PermissionNode {
        name: name.to_owned(),
        value_type: PermissionValueType::Integer,
    }
}

/// Uuid 1 is allowed, uuid 2 refused, every other uuid fails.
struct Answers;

impl NodeAnswers for Answers {
    fn answer(&mut self, _node: &PermissionNode, uuid: u128) -> Result<bool, PermissionError> {
        match uuid {
            1 => Ok(true),
            2 => Ok(false),
            _ => Err(PermissionError::HandlerFailed),
        }
    }
}

fn words(path: &[&str]) -> Vec<String> {
    path.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn a_required_argument_path_runs_only_at_its_end() {
    assert_eq!(
        runnable_paths("pay <player:player> <amount:integer>", &command_arguments()).unwrap(),
        vec![words(&["pay", "player", "amount"])]
    );
}

#[test]
fn an_optional_argument_also_runs_at_the_prefix_before_it() {
    assert_eq!(
        runnable_paths("balance [player:player]", &command_arguments()).unwrap(),
        vec![words(&["balance"]), words(&["balance", "player"])]
    );
}

#[test]
fn without_a_custom_handler_usages_keep_their_levels_and_nothing_is_gated() {
    let plain = [Command::op("eco give <player:player> <amount:integer>", 3)];
    let with_nodes = [Command::new("balance").with_node(&boolean("economy.balance"))];
    let (out, gate, problems) = split_usages(
        &plain,
        &with_nodes,
        NodePolicy::Levels,
        &command_arguments(),
    );
    assert!(gate.is_empty());
    assert!(problems.is_empty());
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].usage, "eco give <player:player> <amount:integer>");
    assert_eq!(out[0].level, 3);
    assert_eq!(out[1].level, 0);
}

#[test]
fn with_a_custom_handler_a_gated_usage_has_no_level_and_its_path_is_gated() {
    let with_nodes = [Command::op("warp set <amount:integer>", 4).with_node(&boolean("warp.set"))];
    let (out, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(problems.is_empty());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].level, 0);
    let path = words(&["warp", "set", "amount"]);
    assert_eq!(
        gate.node(&path).map(|node| node.name.as_str()),
        Some("warp.set")
    );
    assert!(gate.node(&words(&["warp"])).is_none());
}

#[test]
fn a_node_that_is_not_boolean_leaves_its_usage_out_with_a_problem() {
    let with_nodes = [Command::new("limit <value:integer>").with_node(&integer("limit.value"))];
    let (out, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(out.is_empty());
    assert!(gate.is_empty());
    assert_eq!(problems.len(), 1);
}

#[test]
fn a_gated_path_shared_with_an_ungated_usage_is_left_out() {
    let plain = [Command::new("balance")];
    let with_nodes =
        [Command::new("balance [player:player]").with_node(&boolean("economy.others"))];
    let (out, gate, problems) = split_usages(
        &plain,
        &with_nodes,
        NodePolicy::Handler,
        &command_arguments(),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].usage, "balance");
    assert!(gate.is_empty());
    assert_eq!(problems.len(), 1);
}

#[test]
fn a_gated_path_already_gated_by_another_node_is_left_out() {
    let with_nodes = [
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.first")),
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.second")),
    ];
    let (out, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert_eq!(out.len(), 1);
    assert_eq!(
        gate.node(&words(&["warp", "set", "amount"]))
            .map(|node| node.name.as_str()),
        Some("warp.first")
    );
    assert_eq!(problems.len(), 1);
}

#[test]
fn denied_paths_are_the_ones_the_answer_refuses_or_fails() {
    let with_nodes = [Command::new("warp set <amount:integer>").with_node(&boolean("warp.set"))];
    let (_, gate, _) = split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    // Uuid 1 is allowed and keeps the path; uuid 2 and an uuid that fails both lose it.
    assert_eq!(gate.denied_for(&mut Answers, 1), Vec::<Vec<String>>::new());
    assert_eq!(
        gate.denied_for(&mut Answers, 2),
        vec![words(&["warp", "set", "amount"])]
    );
    assert_eq!(
        gate.denied_for(&mut Answers, 3),
        vec![words(&["warp", "set", "amount"])]
    );
}

#[test]
fn a_gated_run_from_a_console_is_refused() {
    let console = CommandSource::Console {
        permission: PermissionLevel::Owners,
    };
    assert!(refuses(&mut Answers, &boolean("warp.set"), &console));
}

#[test]
fn an_empty_gate_denies_nothing() {
    let gate = NodeGate::default();
    assert!(gate.denied_for(&mut Answers, 2).is_empty());
    assert!(gate.node(&words(&["warp"])).is_none());
}

#[test]
fn a_usage_the_tree_leaves_out_leaves_no_gated_path() {
    // `give` is a vanilla command, so the tree refuses the first usage after the gate took it.
    let with_nodes = [
        Command::new("give <amount:integer>").with_node(&boolean("shop.give")),
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.set")),
    ];
    let (out, mut gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(problems.is_empty());
    assert!(gate.node(&words(&["give", "amount"])).is_some());
    let (_, _, tree_problems, rejected) =
        command_tree_accepting(&out, |name| name == "give", &command_arguments());
    assert_eq!(rejected, vec![0]);
    assert_eq!(tree_problems.len(), 1);
    gate.without_usages(&rejected);
    assert!(gate.node(&words(&["give", "amount"])).is_none());
    assert_eq!(
        gate.node(&words(&["warp", "set", "amount"]))
            .map(|node| node.name.as_str()),
        Some("warp.set")
    );
}

#[test]
fn a_path_two_usages_name_stays_gated_until_both_are_left_out() {
    let with_nodes = [
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.set")),
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.set")),
    ];
    let (_, mut gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(problems.is_empty());
    let path = words(&["warp", "set", "amount"]);
    gate.without_usages(&[0]);
    assert!(gate.node(&path).is_some());
    gate.without_usages(&[1]);
    assert!(gate.node(&path).is_none());
    assert!(gate.is_empty());
}

#[test]
fn a_refused_policy_leaves_every_node_usage_out_and_keeps_the_plain_ones() {
    // A failed setup, or a configured handler no mod provides, must not open a node usage at its
    // level: it is left out of the tree and nothing is gated, so nothing runs.
    let plain = [Command::op("eco give <player:player> <amount:integer>", 3)];
    let with_nodes = [
        Command::new("balance").with_node(&boolean("economy.balance")),
        Command::op("warp set <amount:integer>", 4).with_node(&boolean("warp.set")),
    ];
    let (out, gate, problems) = split_usages(
        &plain,
        &with_nodes,
        NodePolicy::Refused,
        &command_arguments(),
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].usage, "eco give <player:player> <amount:integer>");
    assert!(gate.is_empty());
    assert_eq!(problems.len(), 2);
}

/// The paths of the tree's executable nodes, by node name from the root.
fn executable_paths(tree: &serde_json::Value) -> Vec<Vec<String>> {
    fn walk(node: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        let Some(children) = node["children"].as_object() else {
            return;
        };
        for (name, child) in children {
            path.push(name.clone());
            if child["executable"] == true {
                out.push(path.clone());
            }
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut Vec::new(), &mut out);
    out
}

#[test]
fn a_name_that_keeps_its_leading_delimiter_is_gated_under_that_name() {
    // The parser takes one `<` off the word, so `<<amount:integer>` is the argument `<amount`.
    // Stripping every leading delimiter would gate `amount`, a path the tree never has, and
    // leave the executable `<amount` open at level 0.
    let with_nodes = [
        Command::new("warp <<amount:integer>").with_node(&boolean("warp.set")),
        Command::new("pay [[who:word]").with_node(&boolean("economy.pay")),
    ];
    let (out, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(problems.is_empty());
    assert_eq!(
        runnable_paths("pay [[who:word]", &command_arguments()),
        Some(vec![words(&["pay"]), words(&["pay", "[who"])])
    );
    let (tree, _, tree_problems, rejected) =
        command_tree_accepting(&out, |_| false, &command_arguments());
    assert!(tree_problems.is_empty() && rejected.is_empty());
    let executable = executable_paths(&tree);
    assert_eq!(executable.len(), 3);
    for path in &executable {
        assert!(
            gate.node(path).is_some(),
            "the executable path {path:?} has no gate"
        );
    }
}

#[test]
fn every_executable_path_of_the_tree_is_gated_whatever_the_usage_looks_like() {
    let usages = [
        "balance",
        "balance [player:player]",
        "/set <block:word>",
        "eco give <player:player> <amount:integer> [reason:text]",
        "odd <<a:word> <[b:word>",
        "odd2 [<c:word>",
        "tag <name:word>>",
    ];
    let with_nodes: Vec<_> = usages
        .iter()
        .map(|usage| Command::new(usage).with_node(&boolean("any.node")))
        .collect();
    let arguments = command_arguments();
    let (out, gate, _) = split_usages(&[], &with_nodes, NodePolicy::Handler, &arguments);
    let (tree, _, _, _) = command_tree_accepting(&out, |_| false, &arguments);
    let executable = executable_paths(&tree);
    assert!(!executable.is_empty());
    for path in executable {
        assert!(
            gate.node(&path).is_some(),
            "the executable path {path:?} has no gate"
        );
    }
}

#[test]
fn a_usage_the_tree_cannot_read_is_left_out_of_a_gated_split() {
    let with_nodes = [Command::new("tag <name:nokind>").with_node(&boolean("tag.name"))];
    let (out, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(out.is_empty());
    assert!(gate.is_empty());
    assert_eq!(problems.len(), 1);
}

/// Counts how often each node is asked; allows everybody.
#[derive(Default)]
struct Counting(Vec<String>);

impl NodeAnswers for Counting {
    fn answer(&mut self, node: &PermissionNode, _uuid: u128) -> Result<bool, PermissionError> {
        self.0.push(node.name.clone());
        Ok(node.name != "warp.set")
    }
}

#[test]
fn a_node_that_gates_many_paths_is_asked_once_for_a_player() {
    // `balance [player:player]` gates two paths and `balance other` one more, all by one node;
    // `warp set` has a node of its own.
    let with_nodes = [
        Command::new("balance [player:player]").with_node(&boolean("economy.balance")),
        Command::new("balance other <player:player>").with_node(&boolean("economy.balance")),
        Command::new("warp set <amount:integer>").with_node(&boolean("warp.set")),
    ];
    let (_, gate, problems) =
        split_usages(&[], &with_nodes, NodePolicy::Handler, &command_arguments());
    assert!(problems.is_empty());
    let mut answers = Counting::default();
    let denied = gate.denied_for(&mut answers, 1);
    assert_eq!(answers.0, ["economy.balance", "warp.set"]);
    // The answer of a node still reaches every one of its paths.
    assert_eq!(denied, vec![words(&["warp", "set", "amount"])]);
}
