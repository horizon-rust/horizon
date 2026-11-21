#[test]
fn only_the_providing_mod_is_asked_and_the_others_never_see_the_query() {
    use std::sync::atomic::{AtomicU32, Ordering};
    static EARLY: AtomicU32 = AtomicU32::new(0);
    static PROVIDER: AtomicU32 = AtomicU32::new(0);
    static LATE: AtomicU32 = AtomicU32::new(0);
    // An earlier mod that answers everything would end the query before the provider.
    struct Early;
    #[mod_api::__subscriptions]
    impl Hooks for Early {
        fn permission_handler_query(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PermissionHandlerQueryEvent<'_>,
        ) -> Option<Result<generated::mod_api::PermissionValue, generated::mod_api::PermissionError>>
        {
            EARLY.fetch_add(1, Ordering::SeqCst);
            Some(Err(generated::mod_api::PermissionError))
        }
    }
    struct Provider;
    #[mod_api::__subscriptions]
    impl Hooks for Provider {
        fn permission_handler_query(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PermissionHandlerQueryEvent<'_>,
        ) -> Option<Result<generated::mod_api::PermissionValue, generated::mod_api::PermissionError>>
        {
            assert_eq!(
                (event.handler, event.provider, event.uuid),
                ("garden.policy", "garden", 7)
            );
            PROVIDER.fetch_add(1, Ordering::SeqCst);
            Some(Ok(generated::mod_api::PermissionValue))
        }
    }
    struct Late;
    #[mod_api::__subscriptions]
    impl Hooks for Late {
        fn permission_handler_query(
            &self,
            _: &mut dyn ServerHost,
            _: &mut PermissionHandlerQueryEvent<'_>,
        ) -> Option<Result<generated::mod_api::PermissionValue, generated::mod_api::PermissionError>>
        {
            LATE.fetch_add(1, Ordering::SeqCst);
            None
        }
    }
    install(
        vec![
            ModEntry::new("early", Box::new(Early)),
            ModEntry::new("garden", Box::new(Provider)),
            ModEntry::new("late", Box::new(Late)),
        ]
        .into(),
    );
    let node = generated::mod_api::PermissionNode;
    let default = generated::mod_api::NodeDefault;
    let mut answer = None;
    dispatch_permission_handler_query(
        &mut TestHost,
        calls::PermissionHandlerQueryCall {
            handler: "garden.policy",
            provider: "garden",
            node: &node,
            default: &default,
            online: Some(7),
            uuid: 7,
            contexts: &[],
            answer: &mut answer,
        },
    );
    assert_eq!(answer.map(|answered| answered.mod_id), Some("garden"));
    assert_eq!(
        [
            EARLY.load(Ordering::SeqCst),
            PROVIDER.load(Ordering::SeqCst),
            LATE.load(Ordering::SeqCst)
        ],
        [0, 1, 0],
        "only the provider is called"
    );
    // A provider that is not installed is not answered by anyone else.
    let mut none = None;
    dispatch_permission_handler_query(
        &mut TestHost,
        calls::PermissionHandlerQueryCall {
            handler: "garden.policy",
            provider: "absent",
            node: &node,
            default: &default,
            online: None,
            uuid: 7,
            contexts: &[],
            answer: &mut none,
        },
    );
    assert!(none.is_none());
    assert_eq!(
        EARLY.load(Ordering::SeqCst) + LATE.load(Ordering::SeqCst),
        0
    );
}
