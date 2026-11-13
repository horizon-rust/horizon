use super::*;
use horizon_command::component::json::{Error, context::UnavailableContext};

#[test]
fn dimension_lifecycle_reads_only_admitted_metadata_and_preserves_unavailable_errors() {
    let world = std::env::temp_dir().join(format!(
        "h146-lifecycle-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let pack = world.join("datapacks/dimension");
    std::fs::create_dir_all(pack.join("data/example/dimension")).unwrap();
    std::fs::write(pack.join("data/example/dimension/extra.json"),r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{"layers":[{"height":1,"block":"minecraft:stone"}],"biome":"minecraft:plains","features":false,"lakes":false}}}"#).unwrap();
    let metadata = pack.join("pack.mcmeta");
    let context = UnavailableContext;
    std::fs::write(&metadata,r#"{"pack":{"description":{"text":"valid","click_event":{"action":"show_dialog","dialog":{"type":"notice","title":"Inline"}}}}}"#).unwrap();
    assert_eq!(world_gen_unstable(&world, &context), Ok(true));
    std::fs::write(
        &metadata,
        r#"{"pack":{"description":{"text":"partial","bold":1}}}"#,
    )
    .unwrap();
    assert_eq!(world_gen_unstable(&world, &context), Ok(false));
    std::fs::write(&metadata,r#"{"pack":{"description":{"text":"needs catalog","hover_event":{"action":"show_item","id":"minecraft:stone"}}}}"#).unwrap();
    let error = world_gen_unstable(&world, &context).unwrap_err();
    assert_eq!(error.pack, "file/dimension");
    assert_eq!(error.reason, Error::Unavailable);
}
