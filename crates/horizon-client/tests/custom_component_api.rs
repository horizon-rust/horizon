//! Native component registration, value limits and content identity.
use horizon_client::content_identity;
use horizon_protocol::{
    buf::NBT_QUOTA,
    item::custom::{self, InstallError},
};
use horizon_world::nbt::Tag;

#[test]
fn registration_limits_and_defaults_are_bound_to_the_connection() {
    assert_eq!(
        custom::install(122, vec!["minecraft:no".into()]),
        Err(InstallError::InvalidName)
    );
    assert_eq!(
        custom::install(122, vec!["sample:x".into(), "sample:x".into()]),
        Err(InstallError::DuplicateName)
    );
    custom::install(122, vec!["sample:z".into(), "sample:a".into()]).unwrap();
    assert_eq!(custom::id("sample:a"), Some(122));
    assert_eq!(custom::id("sample:z"), Some(123));
    assert_eq!(
        custom::install(122, vec![]),
        Err(InstallError::AlreadyInstalled)
    );

    assert!(custom::validate(&Tag::Int(42)).is_ok());
    assert!(custom::validate(&Tag::End).is_err());
    assert!(custom::validate(&Tag::Double((-0.0_f64).to_bits())).is_err());
    assert!(custom::validate(&Tag::Float(0x7fc0_0001)).is_err());
    assert!(custom::validate(&Tag::Float(0x7fc0_0000)).is_ok());
    assert!(custom::validate(&Tag::ByteArray(vec![0; NBT_QUOTA])).is_err());
    let mut deep = Tag::Int(1);
    for _ in 0..600 {
        deep = Tag::List(vec![deep]);
    }
    assert!(custom::validate(&deep).is_err());
    assert!(custom::validate(&Tag::List(vec![Tag::Int(1), Tag::String("x".into())])).is_ok());

    let dir =
        std::env::temp_dir().join(format!("horizon-component-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, value) in [("a", 1), ("b", 2), ("c", 1)] {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("facts.json"), br#"{"states":[]}"#).unwrap();
        std::fs::write(
            root.join("items.json"),
            format!(r#"{{"sample:item":{{"sample:a":{{"value":{value}}}}}}}"#),
        )
        .unwrap();
    }
    let a = content_identity::identity(&dir.join("a")).unwrap();
    let b = content_identity::identity(&dir.join("b")).unwrap();
    let c = content_identity::identity(&dir.join("c")).unwrap();
    assert_ne!(a, b);
    assert_eq!(a, c);
    std::fs::remove_dir_all(dir).unwrap();
}
