//! Acceptance rows for the permission service: typed nodes, duplicates, unregistered queries,
//! contexts, online and offline UUIDs, initialization, handler selection and failures.

use super::permission_service::{
    DEFAULT_HANDLER_ID, HandlerFactory, LevelSource, ModAnswers, NodePolicy, PermissionHandler,
    PermissionService,
};
use mod_api::{
    NodeDefault, PermissionContext, PermissionError, PermissionLevel, PermissionNode,
    PermissionNodes, PermissionValue, PermissionValueType, Text,
};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

const ALICE: u128 = 0x1111; // online, an administrator
const BOB: u128 = 0x2222; // offline, a moderator in the operator list
const CAROL: u128 = 0x3333; // unknown to the server

fn boolean(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Boolean)
}

fn integer(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Integer)
}

fn levels() -> LevelSource {
    Arc::new(|online, uuid| match (online, uuid) {
        (Some(ALICE), ALICE) => PermissionLevel::Admins,
        (None, BOB) => PermissionLevel::Moderators,
        _ => PermissionLevel::All,
    })
}

/// A handler that answers one fixed result and counts how often it is asked.
struct Fixed {
    answer: Result<PermissionValue, PermissionError>,
    calls: Arc<AtomicU32>,
}

impl PermissionHandler for Fixed {
    fn id(&self) -> &str {
        "fixed"
    }

    fn query(
        &mut self,
        _node: &PermissionNode,
        _default: &NodeDefault,
        _online: Option<u128>,
        _uuid: u128,
        _contexts: &[PermissionContext],
        _answers: &mut dyn ModAnswers,
    ) -> Result<PermissionValue, PermissionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}

fn fixed(
    answer: Result<PermissionValue, PermissionError>,
    calls: Arc<AtomicU32>,
) -> HandlerFactory {
    Box::new(
        move |_nodes: &[PermissionNode]| -> Box<dyn PermissionHandler> {
            Box::new(Fixed { answer, calls })
        },
    )
}

/// A service with the built-in handler answering, after gathering `nodes`.
fn ready(nodes: Vec<(PermissionNode, NodeDefault)>) -> PermissionService {
    let mut service = PermissionService::new();
    service.set_level_source(levels());
    for (node, default) in nodes {
        service.register_node(node, default).unwrap();
    }
    service.finish_gathering(None);
    service
}

#[test]
fn each_value_type_answers_its_declared_type() {
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_node(
            integer("a.count"),
            NodeDefault::Value(PermissionValue::Integer(7)),
        )
        .unwrap();
    service
        .register_node(
            PermissionNode::new("a.label", PermissionValueType::String),
            NodeDefault::Value(PermissionValue::String("gold".into())),
        )
        .unwrap();
    service
        .register_node(
            PermissionNode::new("a.title", PermissionValueType::Text),
            NodeDefault::Value(PermissionValue::Text(Text::literal("Gold"))),
        )
        .unwrap();
    service.finish_gathering(None);

    assert_eq!(
        service.query(&boolean("a.flag"), None, CAROL, &[]),
        Ok(PermissionValue::Boolean(true))
    );
    assert_eq!(
        service.query(&integer("a.count"), None, CAROL, &[]),
        Ok(PermissionValue::Integer(7))
    );
    assert_eq!(
        service.query(
            &PermissionNode::new("a.label", PermissionValueType::String),
            None,
            CAROL,
            &[]
        ),
        Ok(PermissionValue::String("gold".into()))
    );
    assert_eq!(
        service.query(
            &PermissionNode::new("a.title", PermissionValueType::Text),
            None,
            CAROL,
            &[]
        ),
        Ok(PermissionValue::Text(Text::literal("Gold")))
    );
}

#[test]
fn the_same_name_with_two_types_is_two_nodes() {
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.x"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_node(
            integer("a.x"),
            NodeDefault::Value(PermissionValue::Integer(3)),
        )
        .unwrap();
    service.finish_gathering(None);
    assert_eq!(
        service.query(&integer("a.x"), None, CAROL, &[]),
        Ok(PermissionValue::Integer(3))
    );
    assert_eq!(service.query_bool(&boolean("a.x"), None, CAROL), Ok(true));
}

#[test]
fn a_duplicate_typed_node_is_refused_and_the_first_default_stays() {
    let mut service = PermissionService::new();
    service.set_level_source(levels());
    service
        .register_node(
            boolean("a.flag"),
            NodeDefault::Level(PermissionLevel::Admins),
        )
        .unwrap();
    assert_eq!(
        service.register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All)),
        Err(PermissionError::DuplicateNode)
    );
    service.finish_gathering(None);
    // Carol is an unknown player at level 0: the kept default (Admins) denies her.
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(false)
    );
    // Alice is an administrator: she is granted.
    assert_eq!(
        service.query_bool(&boolean("a.flag"), Some(ALICE), ALICE),
        Ok(true)
    );
}

#[test]
fn an_unregistered_node_is_refused_before_the_handler_runs() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Boolean(true)), calls.clone()),
        )
        .unwrap();
    service.finish_gathering(Some("policy"));
    assert_eq!(
        service.query_bool(&boolean("a.other"), None, CAROL),
        Err(PermissionError::Unregistered)
    );
    assert_eq!(
        service.query(&integer("a.flag"), None, CAROL, &[]),
        Err(PermissionError::Unregistered)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn undeclared_contexts_are_passed_on_and_ignored_by_the_default() {
    let mut service = ready(vec![(
        boolean("a.flag"),
        NodeDefault::Level(PermissionLevel::All),
    )]);
    let contexts = [PermissionContext {
        key: "undeclared".into(),
        value: PermissionValue::Integer(9),
    }];
    assert_eq!(
        service.query(&boolean("a.flag"), None, CAROL, &contexts),
        Ok(PermissionValue::Boolean(true))
    );
}

#[test]
fn online_and_offline_uuids_are_answered_from_the_operator_levels() {
    let mut service = ready(vec![(
        boolean("a.staff"),
        NodeDefault::Level(PermissionLevel::Gamemasters),
    )]);
    assert_eq!(
        service.query_bool(&boolean("a.staff"), Some(ALICE), ALICE),
        Ok(true)
    );
    assert_eq!(
        service.query_bool(&boolean("a.staff"), None, BOB),
        Ok(false)
    );
    assert_eq!(
        service.query_bool(&boolean("a.staff"), None, CAROL),
        Ok(false)
    );
    let mut service = ready(vec![(
        boolean("a.mod"),
        NodeDefault::Level(PermissionLevel::Moderators),
    )]);
    assert_eq!(service.query_bool(&boolean("a.mod"), None, BOB), Ok(true));
}

#[test]
fn queries_and_late_declarations_are_refused_before_initialization() {
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::NotInitialized)
    );
    service.finish_gathering(None);
    assert_eq!(
        service.register_node(boolean("a.late"), NodeDefault::Level(PermissionLevel::All)),
        Err(PermissionError::InvalidInput)
    );
    assert_eq!(
        service.register_handler_factory(
            "late",
            fixed(
                Ok(PermissionValue::Boolean(true)),
                Arc::new(AtomicU32::new(0))
            )
        ),
        Err(PermissionError::InvalidInput)
    );
}

#[test]
fn duplicate_handler_ids_and_the_reserved_default_id_are_refused() {
    let mut service = PermissionService::new();
    let calls = Arc::new(AtomicU32::new(0));
    service
        .register_handler_factory(
            "garden",
            fixed(Ok(PermissionValue::Boolean(true)), calls.clone()),
        )
        .unwrap();
    assert_eq!(
        service.register_handler_factory(
            "garden",
            fixed(Ok(PermissionValue::Boolean(true)), calls.clone())
        ),
        Err(PermissionError::DuplicateHandler)
    );
    assert_eq!(
        service.register_handler_factory(
            "default",
            fixed(Ok(PermissionValue::Boolean(true)), calls.clone())
        ),
        Err(PermissionError::DuplicateHandler)
    );
    assert_eq!(
        service.register_handler_factory("", fixed(Ok(PermissionValue::Boolean(true)), calls)),
        Err(PermissionError::InvalidInput)
    );
}

#[test]
fn a_configured_handler_answers_and_an_unknown_one_selects_the_default() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut configured = PermissionService::new();
    configured
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    configured
        .register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Boolean(false)), calls.clone()),
        )
        .unwrap();
    configured.finish_gathering(Some("policy"));
    assert_eq!(
        configured.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(false)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let mut unknown = PermissionService::new();
    unknown.set_level_source(levels());
    unknown
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    unknown
        .register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Boolean(false)), calls.clone()),
        )
        .unwrap();
    unknown.finish_gathering(Some("missing"));
    assert_eq!(
        unknown.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(true)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_failing_handler_denies_explicitly_and_never_falls_back_to_the_default() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = PermissionService::new();
    service.set_level_source(levels());
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_handler_factory("policy", fixed(Err(PermissionError::HandlerFailed), calls))
        .unwrap();
    service.finish_gathering(Some("policy"));
    // The default would grant this node to everyone.
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::HandlerFailed)
    );
    assert_eq!(
        service.query(&boolean("a.flag"), None, CAROL, &[]),
        Err(PermissionError::HandlerFailed)
    );
}

#[test]
fn a_handler_answer_of_another_type_is_wrong_type() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Integer(1)), calls.clone()),
        )
        .unwrap();
    service.finish_gathering(Some("policy"));
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::WrongType)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_panicking_handler_is_a_handler_failure_and_the_service_stays_usable() {
    struct Once {
        calls: Arc<AtomicU32>,
    }
    impl PermissionHandler for Once {
        fn id(&self) -> &str {
            "once"
        }
        fn query(
            &mut self,
            _node: &PermissionNode,
            _default: &NodeDefault,
            _online: Option<u128>,
            _uuid: u128,
            _contexts: &[PermissionContext],
            _answers: &mut dyn ModAnswers,
        ) -> Result<PermissionValue, PermissionError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("first answer traps");
            }
            Ok(PermissionValue::Boolean(true))
        }
    }
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    let factory_calls = calls.clone();
    service
        .register_handler_factory(
            "once",
            Box::new(
                move |_nodes: &[PermissionNode]| -> Box<dyn PermissionHandler> {
                    Box::new(Once {
                        calls: factory_calls,
                    })
                },
            ),
        )
        .unwrap();
    service.finish_gathering(Some("once"));
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::HandlerFailed)
    );
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(true)
    );
}

#[test]
fn closing_and_reopening_gathers_afresh() {
    let first = ready(vec![(
        boolean("a.flag"),
        NodeDefault::Level(PermissionLevel::All),
    )]);
    drop(first);
    let mut reopened = PermissionService::new();
    assert_eq!(
        reopened.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::NotInitialized)
    );
    reopened.finish_gathering(None);
    assert_eq!(
        reopened.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::Unregistered)
    );
}

#[test]
fn bad_names_and_mismatched_defaults_are_refused_and_keep_nothing() {
    let mut service = PermissionService::new();
    for name in ["", "a..b", "a b", ".a", "a.", &"x".repeat(257)] {
        assert_eq!(
            service.register_node(boolean(name), NodeDefault::Level(PermissionLevel::All)),
            Err(PermissionError::InvalidInput),
            "name {name:?}"
        );
    }
    assert_eq!(
        service.register_node(
            boolean("a.flag"),
            NodeDefault::Value(PermissionValue::Boolean(true))
        ),
        Err(PermissionError::WrongType)
    );
    assert_eq!(
        service.register_node(integer("a.count"), NodeDefault::Level(PermissionLevel::All)),
        Err(PermissionError::WrongType)
    );
    assert_eq!(
        service.register_node(
            integer("a.count"),
            NodeDefault::Value(PermissionValue::String("x".into()))
        ),
        Err(PermissionError::WrongType)
    );
    service.finish_gathering(None);
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::Unregistered)
    );
}

#[test]
fn too_many_or_badly_keyed_contexts_are_refused_before_the_handler() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Boolean(true)), calls.clone()),
        )
        .unwrap();
    service.finish_gathering(Some("policy"));
    let context = |key: &str| PermissionContext {
        key: key.into(),
        value: PermissionValue::Integer(1),
    };
    let too_many: Vec<_> = (0..33).map(|i| context(&format!("k{i}"))).collect();
    assert_eq!(
        service.query(&boolean("a.flag"), None, CAROL, &too_many),
        Err(PermissionError::InvalidInput)
    );
    assert_eq!(
        service.query(&boolean("a.flag"), None, CAROL, &[context("")]),
        Err(PermissionError::InvalidInput)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn query_bool_refuses_a_non_boolean_node_before_the_handler() {
    let mut service = PermissionService::new();
    service
        .register_node(
            integer("a.count"),
            NodeDefault::Value(PermissionValue::Integer(2)),
        )
        .unwrap();
    service.finish_gathering(None);
    assert_eq!(
        service.query_bool(&integer("a.count"), None, CAROL),
        Err(PermissionError::WrongType)
    );
}

#[test]
fn the_selected_factory_is_given_every_declared_node() {
    let seen = Arc::new(AtomicUsize::new(0));
    let mut service = PermissionService::new();
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service
        .register_node(
            integer("a.count"),
            NodeDefault::Value(PermissionValue::Integer(2)),
        )
        .unwrap();
    let seen_in_factory = seen.clone();
    service
        .register_handler_factory(
            "counting",
            Box::new(
                move |nodes: &[PermissionNode]| -> Box<dyn PermissionHandler> {
                    seen_in_factory.store(nodes.len(), Ordering::SeqCst);
                    Box::new(Fixed {
                        answer: Ok(PermissionValue::Boolean(true)),
                        calls: Arc::new(AtomicU32::new(0)),
                    })
                },
            ),
        )
        .unwrap();
    service.finish_gathering(Some("counting"));
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[test]
fn the_collector_declares_nodes_like_register_node() {
    let mut service = PermissionService::new();
    let nodes: &mut dyn PermissionNodes = &mut service;
    nodes
        .add(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    assert_eq!(
        nodes.add(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All)),
        Err(PermissionError::DuplicateNode)
    );
    service.finish_gathering(None);
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(true)
    );
}

#[test]
fn the_node_policy_follows_the_selection_and_a_missing_handler_is_its_own_state() {
    let built_in = ready(vec![(
        boolean("a.flag"),
        NodeDefault::Level(PermissionLevel::All),
    )]);
    assert_eq!(built_in.node_policy(), NodePolicy::Levels);

    let mut unknown = PermissionService::new();
    unknown
        .register_handler_factory(
            "policy",
            fixed(
                Ok(PermissionValue::Boolean(true)),
                Arc::new(AtomicU32::new(0)),
            ),
        )
        .unwrap();
    unknown.finish_gathering(Some("missing"));
    // A handler id no mod gathered is not the built-in handler: node usages are refused.
    assert_eq!(unknown.node_policy(), NodePolicy::Refused);

    let mut selected = PermissionService::new();
    selected
        .register_handler_factory(
            "policy",
            fixed(
                Ok(PermissionValue::Boolean(true)),
                Arc::new(AtomicU32::new(0)),
            ),
        )
        .unwrap();
    selected.finish_gathering(Some("policy"));
    assert_eq!(selected.node_policy(), NodePolicy::Handler);
}

#[test]
fn a_second_gathering_changes_nothing() {
    let calls = Arc::new(AtomicU32::new(0));
    let mut service = ready(vec![(
        boolean("a.flag"),
        NodeDefault::Level(PermissionLevel::All),
    )]);
    // A join or a resource reload must not select a handler or declare a node afterwards.
    service.finish_gathering(Some("policy"));
    assert_eq!(service.node_policy(), NodePolicy::Levels);
    assert_eq!(
        service.register_handler_factory(
            "policy",
            fixed(Ok(PermissionValue::Boolean(false)), calls.clone()),
        ),
        Err(PermissionError::InvalidInput)
    );
    assert_eq!(
        service.register_node(boolean("a.late"), NodeDefault::Level(PermissionLevel::All)),
        Err(PermissionError::InvalidInput)
    );
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(true)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_setup_is_refused_and_a_service_not_yet_gathered_is_not() {
    let pending = PermissionService::new();
    assert_eq!(pending.node_policy(), NodePolicy::Levels);

    let mut failed = PermissionService::failed();
    assert_eq!(failed.node_policy(), NodePolicy::Refused);
    // It stays uninitialized: every query is refused too.
    assert!(!failed.is_initialized());
    assert_eq!(
        failed.query_bool(&boolean("a.flag"), None, CAROL),
        Err(PermissionError::NotInitialized)
    );
}

#[test]
fn a_missing_configured_handler_refuses_node_usages_but_the_default_still_answers_queries() {
    let mut service = PermissionService::new();
    service.set_level_source(levels());
    service
        .register_node(boolean("a.flag"), NodeDefault::Level(PermissionLevel::All))
        .unwrap();
    service.finish_gathering(Some("missing"));
    assert_eq!(service.node_policy(), NodePolicy::Refused);
    assert_eq!(
        service.query_bool(&boolean("a.flag"), None, CAROL),
        Ok(true)
    );
    // Naming the built-in handler is not a missing handler.
    let mut named = PermissionService::new();
    named.finish_gathering(Some(DEFAULT_HANDLER_ID));
    assert_eq!(named.node_policy(), NodePolicy::Levels);
}

#[test]
fn a_nobody_node_is_false_for_everyone_including_administrators() {
    let mut service = ready(vec![(boolean("a.exempt"), NodeDefault::Nobody)]);
    // ALICE is online at level 3 (Admins), BOB is offline at level 1, CAROL is unknown at level 0.
    assert_eq!(
        service.query_bool(&boolean("a.exempt"), Some(ALICE), ALICE),
        Ok(false)
    );
    assert_eq!(
        service.query_bool(&boolean("a.exempt"), None, BOB),
        Ok(false)
    );
    assert_eq!(
        service.query_bool(&boolean("a.exempt"), None, CAROL),
        Ok(false)
    );
}

#[test]
fn nobody_is_only_a_default_for_boolean_nodes() {
    let mut service = PermissionService::new();
    assert_eq!(
        service.register_node(integer("a.count"), NodeDefault::Nobody),
        Err(PermissionError::WrongType)
    );
    assert_eq!(
        service.register_node(boolean("a.exempt"), NodeDefault::Nobody),
        Ok(())
    );
}
