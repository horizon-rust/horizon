use super::*;

impl Hub {
    /// Publish the frozen server token inventory before sending any command trees.
    pub fn set_selector_tokens(
        &mut self,
        tokens: Vec<horizon_client_core::registries::SelectorToken>,
    ) {
        self.selector_tokens = tokens;
    }

    /// Commands.sendCommands: the command tree for this player. Public because `Hub::set_mod_commands`, a function
    /// replacement's target (`tools/ci/fn-replace-check/lower`), calls it from a copy in another crate.
    pub fn send_commands(&self, i: usize) {
        let g = &self.guests[i];
        let mut packet = self.commands_packet(g.permission);
        // What the mods took out is not in the tree the client gets.
        if !g.hidden_commands.is_empty() {
            let hidden: Vec<i32> = packet
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.kind == 1 && g.hidden_commands.contains(&n.name))
                .map(|(i, _)| i as i32)
                .collect();
            let root = packet.root as usize;
            if let Some(root) = packet.nodes.get_mut(root) {
                root.children.retain(|c| !hidden.contains(c));
            }
        }
        // The paths a permission node refuses this player: their end is not runnable and
        // a branch left with nothing under it goes too.
        hide_command_paths(&mut packet, &g.hidden_command_paths);
        // A modified client reads every node in the array: only what the root still reaches is sent.
        keep_reachable_nodes(&mut packet);
        if g.brand == "horizon" {
            g.send(&C::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload {
                    channel: crate::lan::commands::selector_sync::CHANNEL.into(),
                    data: crate::lan::commands::selector_sync::encode(&self.selector_tokens),
                },
            ));
        }
        g.send(&C::Commands(packet));
    }
}

/// Takes each path's executable end out of a command tree: the node stops being runnable, and
/// it leaves its parent once nothing is below it, walking back up the same way. A node that
/// other usages still reach stays. Paths that the tree lacks are skipped.
fn hide_command_paths(packet: &mut play::Commands, paths: &[Vec<String>]) {
    for path in paths {
        let mut chain: Vec<(usize, usize)> = Vec::new();
        let mut current = packet.root as usize;
        for name in path {
            let next = packet.nodes.get(current).and_then(|node| {
                node.children.iter().copied().find(|&child| {
                    packet
                        .nodes
                        .get(child as usize)
                        .is_some_and(|candidate| candidate.name == *name)
                })
            });
            let Some(next) = next else {
                chain.clear();
                break;
            };
            chain.push((current, next as usize));
            current = next as usize;
        }
        if chain.is_empty() {
            continue;
        }
        packet.nodes[current].executable = false;
        for &(parent, child) in chain.iter().rev() {
            if !packet.nodes[child].children.is_empty() || packet.nodes[child].executable {
                break;
            }
            packet.nodes[parent]
                .children
                .retain(|&c| c as usize != child);
        }
    }
}

/// Drops every node the root no longer reaches through children or redirects, and renumbers the
/// rest, so a node taken out of the tree is not in the packet at all.
fn keep_reachable_nodes(packet: &mut play::Commands) {
    let count = packet.nodes.len();
    let mut reached = vec![false; count];
    let mut stack = vec![packet.root as usize];
    while let Some(index) = stack.pop() {
        if index >= count || reached[index] {
            continue;
        }
        reached[index] = true;
        let node = &packet.nodes[index];
        stack.extend(node.children.iter().map(|&child| child as usize));
        stack.extend(node.redirect.map(|redirect| redirect as usize));
    }
    if reached.iter().all(|&kept| kept) {
        return;
    }
    let mut renumbered = vec![None; count];
    let mut next = 0;
    for (index, kept) in reached.iter().enumerate() {
        if *kept {
            renumbered[index] = Some(next);
            next += 1;
        }
    }
    let new_index = |old: i32| renumbered.get(old as usize).copied().flatten();
    packet.root = new_index(packet.root).unwrap_or(0);
    let nodes = std::mem::take(&mut packet.nodes);
    packet.nodes = nodes
        .into_iter()
        .zip(&reached)
        .filter(|(_, kept)| **kept)
        .map(|(mut node, _)| {
            node.children = node.children.iter().filter_map(|&c| new_index(c)).collect();
            node.redirect = node.redirect.and_then(new_index);
            node
        })
        .collect();
}

#[cfg(test)]
mod hide_command_paths_tests {
    use super::*;
    use horizon_protocol::packets::play::CommandNode;

    fn node(name: &str, executable: bool, children: Vec<i32>) -> CommandNode {
        CommandNode {
            kind: 1,
            executable,
            restricted: false,
            children,
            redirect: None,
            name: name.to_owned(),
            parser: None,
            suggestions: None,
        }
    }

    /// `root` with `warp` (executable, with `set` below it, executable with `amount`) and `pay`.
    fn tree() -> play::Commands {
        // 0 root, 1 warp, 2 set, 3 amount, 4 pay
        play::Commands {
            nodes: vec![
                node("", false, vec![1, 4]),
                node("warp", true, vec![2]),
                node("set", true, vec![3]),
                node("amount", true, vec![]),
                node("pay", true, vec![]),
            ],
            root: 0,
        }
    }

    fn children_of(packet: &play::Commands, index: usize) -> Vec<i32> {
        packet.nodes[index].children.clone()
    }

    #[test]
    fn a_leaf_path_leaves_its_branch_and_root_keeps_the_rest() {
        let mut packet = tree();
        hide_command_paths(&mut packet, &[vec!["pay".to_owned()]]);
        assert_eq!(children_of(&packet, 0), vec![1]);
        assert!(!packet.nodes[4].executable);
    }

    #[test]
    fn a_shared_prefix_stays_runnable_when_only_its_argument_path_is_hidden() {
        let mut packet = tree();
        hide_command_paths(
            &mut packet,
            &[vec![
                "warp".to_owned(),
                "set".to_owned(),
                "amount".to_owned(),
            ]],
        );
        assert!(packet.nodes[1].executable);
        assert!(packet.nodes[2].executable);
        assert_eq!(children_of(&packet, 2), Vec::<i32>::new());
        assert_eq!(children_of(&packet, 1), vec![2]);
    }

    #[test]
    fn hiding_a_prefix_end_keeps_the_arguments_below_it() {
        let mut packet = tree();
        hide_command_paths(&mut packet, &[vec!["warp".to_owned(), "set".to_owned()]]);
        assert!(packet.nodes[1].executable);
        assert!(!packet.nodes[2].executable);
        // `set` stays as the parent of `amount`, so the branch is kept but not runnable at `set`.
        assert_eq!(children_of(&packet, 1), vec![2]);
        assert_eq!(children_of(&packet, 2), vec![3]);
    }

    fn encoded(packet: &play::Commands) -> Vec<u8> {
        use horizon_protocol::buf::{Codec, Writer};
        let mut out = Writer::new();
        <play::Commands as Codec<play::Commands>>::write(packet, &mut out);
        out.data
    }

    #[test]
    fn a_hidden_literal_is_not_in_the_encoded_packet() {
        let mut packet = tree();
        hide_command_paths(&mut packet, &[vec!["pay".to_owned()]]);
        keep_reachable_nodes(&mut packet);
        let bytes = encoded(&packet);
        assert!(!bytes.windows(3).any(|w| w == b"pay"));
        assert!(bytes.windows(4).any(|w| w == b"warp"));
        assert_eq!(packet.nodes.len(), 4);
        assert_eq!(packet.nodes[packet.root as usize].children, vec![1]);
    }

    #[test]
    fn a_hidden_argument_branch_leaves_with_its_nodes_and_indices_are_renumbered() {
        let mut packet = tree();
        hide_command_paths(&mut packet, &[vec!["warp".to_owned()]]);
        // `warp` stays (its `set` branch is still reachable) but is not runnable.
        hide_command_paths(
            &mut packet,
            &[vec![
                "warp".to_owned(),
                "set".to_owned(),
                "amount".to_owned(),
            ]],
        );
        keep_reachable_nodes(&mut packet);
        let names: Vec<&str> = packet.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["", "warp", "set", "pay"]);
        assert_eq!(packet.nodes[0].children, vec![1, 3]);
        assert_eq!(packet.nodes[2].children, Vec::<i32>::new());
        assert!(!encoded(&packet).windows(6).any(|w| w == b"amount"));
    }

    #[test]
    fn a_redirect_target_stays_and_follows_its_new_index() {
        // 0 root, 1 unreachable, 2 tp (redirects to 3), 3 teleport
        let mut packet = play::Commands {
            nodes: vec![
                node("", false, vec![2, 3]),
                node("gone", true, vec![]),
                node("tp", true, vec![]),
                node("teleport", true, vec![]),
            ],
            root: 0,
        };
        packet.nodes[2].redirect = Some(3);
        keep_reachable_nodes(&mut packet);
        let names: Vec<&str> = packet.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["", "tp", "teleport"]);
        assert_eq!(packet.nodes[0].children, vec![1, 2]);
        assert_eq!(packet.nodes[1].redirect, Some(2));
    }

    #[test]
    fn a_tree_with_nothing_hidden_is_left_as_it_is() {
        let mut packet = tree();
        let before = packet.clone();
        keep_reachable_nodes(&mut packet);
        assert_eq!(packet, before);
    }

    #[test]
    fn a_missing_path_changes_nothing() {
        let mut packet = tree();
        let before = packet.clone();
        hide_command_paths(&mut packet, &[vec!["nothing".to_owned()]]);
        assert_eq!(packet, before);
    }
}
