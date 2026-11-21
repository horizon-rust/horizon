//! The game's permission query: the recursion guard, the live operator levels and the arguments a
//! mod handler is asked with. A fake host stands in for the game; expected values are literals.

use super::handler::gather_handlers;
use super::native_permissions::{
    PermissionHost, QueryState, gathered_nodes, run_query, run_query_bool,
};
use super::permission_service::{ModAnswer, ModAnswers, PermissionService};
use mod_api::{
    NodeDefault, PermissionContext, PermissionError, PermissionLevel, PermissionNode,
    PermissionValue, PermissionValueType,
};

const ALICE: u128 = 0xa11ce; // online
const BOB: u128 = 0xb0b; // offline, level 3 in the operator list
const CAROL: u128 = 0xca401; // offline, not listed

fn boolean(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Boolean)
}

/// What a mod handler was asked: its id, `online`, the UUID and the number of contexts.
type Asked = (String, Option<u128>, u128, usize);

struct FakeHost {
    service: PermissionService,
    state: QueryState,
    /// The live operator level of ALICE; nobody else is online.
    alice_level: u8,
    asked: Vec<Asked>,
    /// Whether the mod handler asks the same query again from inside its answer.
    reenter: bool,
    /// What that inner query answered.
    inner: Option<Result<PermissionValue, PermissionError>>,
    /// What the mod `garden` answers.
    reply: Result<PermissionValue, PermissionError>,
}

impl ModAnswers for FakeHost {
    fn answer(
        &mut self,
        handler: &str,
        _provider: &str,
        node: &PermissionNode,
        _default: &NodeDefault,
        online: Option<u128>,
        uuid: u128,
        contexts: &[PermissionContext],
    ) -> Option<ModAnswer> {
        self.asked
            .push((handler.to_owned(), online, uuid, contexts.len()));
        if self.reenter {
            self.reenter = false;
            self.inner = Some(run_query(self, node, uuid, &[]));
        }
        Some(ModAnswer {
            mod_id: "garden".to_owned(),
            answer: self.reply.clone(),
        })
    }
}

impl PermissionHost for FakeHost {
    fn permission_parts(&mut self) -> (&mut PermissionService, &mut QueryState) {
        (&mut self.service, &mut self.state)
    }

    fn online_level(&self, uuid: u128) -> Option<u8> {
        (uuid == ALICE).then_some(self.alice_level)
    }
}

/// A host whose server has the Boolean nodes `a.staff` (level 2 grants it) and `a.flag`
/// (everyone), and the operator list `BOB` at level 3. `configured` selects a handler.
fn host(configured: &str) -> FakeHost {
    let state = QueryState::default();
    let mut service = PermissionService::new();
    service.set_level_source(state.level_source(|uuid| (uuid == BOB).then_some(3)));
    service
        .register_node(
            boolean("a.staff"),
            NodeDefault::Level(PermissionLevel::Gamemasters),
        )
        .unwrap();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    gather_handlers(&mut service, &[("garden.policy", "garden")], configured).unwrap();
    FakeHost {
        service,
        state,
        alice_level: 0,
        asked: Vec::new(),
        reenter: false,
        inner: None,
        reply: Ok(PermissionValue::Boolean(true)),
    }
}

#[test]
fn a_query_asked_again_from_a_handler_fails_both_queries_as_recursive() {
    let mut host = host("garden.policy");
    host.reenter = true;
    assert_eq!(
        run_query(&mut host, &boolean("a.flag"), ALICE, &[]),
        Err(PermissionError::Recursive)
    );
    assert_eq!(host.inner, Some(Err(PermissionError::Recursive)));
    // Only the outer query reached the mod; the inner one was refused before it.
    assert_eq!(host.asked.len(), 1);
}

#[test]
fn the_service_and_the_guard_are_back_after_a_recursive_query() {
    let mut host = host("garden.policy");
    host.reenter = true;
    let _ = run_query(&mut host, &boolean("a.flag"), ALICE, &[]);
    assert!(host.service.is_initialized());
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.flag"), ALICE),
        Ok(true)
    );
}

#[test]
fn a_refused_query_leaves_the_guard_open() {
    let mut host = host("garden.policy");
    assert_eq!(
        run_query(&mut host, &boolean("a.unknown"), ALICE, &[]),
        Err(PermissionError::Unregistered)
    );
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.flag"), ALICE),
        Ok(true)
    );
}

#[test]
fn the_mod_handler_is_asked_with_the_uuid_the_online_flag_and_the_contexts() {
    let mut host = host("garden.policy");
    let contexts = [PermissionContext {
        key: "region".to_owned(),
        value: PermissionValue::Integer(7),
    }];
    run_query(&mut host, &boolean("a.flag"), ALICE, &contexts).unwrap();
    run_query(&mut host, &boolean("a.flag"), CAROL, &[]).unwrap();
    assert_eq!(
        host.asked,
        vec![
            ("garden.policy".to_owned(), Some(ALICE), ALICE, 1),
            ("garden.policy".to_owned(), None, CAROL, 0),
        ]
    );
}

#[test]
fn a_mod_handlers_error_is_the_query_error_and_never_a_grant() {
    let mut host = host("garden.policy");
    host.reply = Err(PermissionError::UnknownPlayer);
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.flag"), ALICE),
        Err(PermissionError::UnknownPlayer)
    );
}

#[test]
fn a_non_boolean_node_is_refused_before_any_handler_runs() {
    let mut host = host("garden.policy");
    let count = PermissionNode::new("a.count", PermissionValueType::Integer);
    assert_eq!(
        run_query_bool(&mut host, &count, ALICE),
        Err(PermissionError::WrongType)
    );
    assert!(host.asked.is_empty());
}

#[test]
fn the_built_in_handler_reads_an_online_players_live_level() {
    let mut host = host("");
    host.alice_level = 2;
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), ALICE),
        Ok(true)
    );
    host.alice_level = 1;
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), ALICE),
        Ok(false)
    );
    assert!(host.asked.is_empty());
}

#[test]
fn an_offline_uuid_is_answered_from_the_operator_list_or_as_level_zero() {
    let mut host = host("");
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), BOB),
        Ok(true)
    );
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), CAROL),
        Ok(false)
    );
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.flag"), CAROL),
        Ok(true)
    );
}

#[test]
fn a_live_level_wins_over_a_listed_one() {
    // Listed at level 3 in the operator list, but online with level 0 (an integrated host).
    let state = QueryState::default();
    let mut service = PermissionService::new();
    service.set_level_source(state.level_source(|_| Some(3)));
    service
        .register_node(
            boolean("a.staff"),
            NodeDefault::Level(PermissionLevel::Gamemasters),
        )
        .unwrap();
    service.finish_gathering(None);
    let mut host = FakeHost {
        service,
        state,
        alice_level: 0,
        asked: Vec::new(),
        reenter: false,
        inner: None,
        reply: Ok(PermissionValue::Boolean(true)),
    };
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), ALICE),
        Ok(false)
    );
    // A UUID that is not on the server has no live level: the list answers.
    assert_eq!(
        run_query_bool(&mut host, &boolean("a.staff"), BOB),
        Ok(true)
    );
}

#[test]
fn a_node_two_mods_declare_fails_the_whole_gather_and_keeps_no_service() {
    let result = gathered_nodes(PermissionService::new(), |nodes| {
        assert_eq!(
            nodes.add(
                boolean("a.flag"),
                NodeDefault::Level(PermissionLevel::Admins)
            ),
            Ok(())
        );
        // The second mod's own call still gets the refusal.
        assert_eq!(
            nodes.add(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All)),
            Err(PermissionError::DuplicateNode)
        );
    });
    assert!(matches!(result, Err(PermissionError::DuplicateNode)));
}

#[test]
fn distinct_nodes_gather_into_an_uninitialized_service() {
    let service = gathered_nodes(PermissionService::new(), |nodes| {
        for name in ["a.one", "a.two"] {
            assert_eq!(
                nodes.add(boolean(name), NodeDefault::Level(PermissionLevel::All)),
                Ok(())
            );
        }
    })
    .unwrap();
    // Not published until `finish_gathering`: it answers nothing yet.
    assert!(!service.is_initialized());
}
