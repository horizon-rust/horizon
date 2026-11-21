# Foliage placer types

Register a `FoliagePlacerType` in `Hooks::register` to grow a tree's canopy by your own code. A tree
feature whose `foliage_placer` is `{"type": "mymod:fruit", ...}` then uses it, in world generation
and when a sapling grows that tree:

```rust,ignore
fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
    let apple = registrar.block_key("mymod:apple");
    registrar.register_foliage_placer_type("fruit", FruitType { apple });
}
```

Use an ID in your mod's namespace. A duplicate ID, an ID in another mod's namespace, or an empty
name stops startup with a message naming your mod. Vanilla's types keep their IDs.

## Decoding

`FoliagePlacerType::decode(&self, json: &str)` is the type's codec: it gets the whole
`foliage_placer` object as JSON text, its `type` included, once when the feature loads, and returns
an `Arc<dyn FoliagePlacer>`. Vanilla's `radius` and `offset` fields belong to vanilla's placers; your
codec reads only the fields it defines. An `Err` refuses the feature with a warning naming the type
and your message, as a data pack's bad field does.

## Placing a canopy

`FoliagePlacer` (`Debug + Send + Sync`) answers vanilla's three calls, in vanilla's order, with the
tree's one random:

| Method | Vanilla | When |
|---|---|---|
| `foliage_height(random, tree_height)` | `foliageHeight` | Before the trunk |
| `foliage_radius(random, trunk_height)` | `foliageRadius` | Before the trunk |
| `create_foliage(foliage, tree_height, attachment, foliage_height, leaf_radius)` | `createFoliage` | Once per `FoliageAttachment` the trunk leaves |

`create_foliage` is vanilla's public `createFoliage`: sample your offset first, as vanilla's samples
its `offset`, then place. `FoliageAttachment` carries `pos`, `radius_offset`, `height_offset` and
`size`; `double_trunk()` is true over a two-by-two trunk.

| `FoliageContext` access | Vanilla |
|---|---|
| `random` | The tree's `RandomSource`, as a `StructureRandom` |
| `block`, `metadata` | `WorldGenLevel.getBlockState`, and block metadata to read its properties |
| `valid_tree_pos` | `TreeFeature.validTreePos`: air or `#minecraft:replaceable_by_trees` |
| `try_place_leaf` | `FoliagePlacer.tryPlaceLeaf`: the tree's foliage provider, waterlogged in water, where no persistent leaves are |
| `set`, `is_set` | The `FoliageSetter`: write any state as foliage, or ask whether this tree's foliage holds a position |

`place_leaves_row(foliage, origin, radius, y, double_trunk, skip)` is vanilla's `placeLeavesRow`:
it passes `skip` the distances `shouldSkipLocationSigned` computes, so `skip` is your
`shouldSkipLocation`. What `set` writes counts as foliage for the tree's decorators and its leaves'
distances. The trunk, the roots and the decorators stay the game's.

Trees place on the worldgen worker threads and on the server thread, with no `Host`: keep a placer
stateless and deterministic. A `next_int_bound` bound below 1, which vanilla refuses with an
exception, answers 0 and fails the tree after your call returns. These traits also work in
`mod.wasm`; its calls run under the dependency group's store lock and call budget, as structure
types' do.
