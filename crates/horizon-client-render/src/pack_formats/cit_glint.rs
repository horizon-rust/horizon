//! OptiFine's Custom Item Textures' enchantment glints (`type=enchantment`): which of a stack's
//! matching rules draw, in which order and how strongly, and whether vanilla's glint still draws
//! under them ([`crate::cit_glint::StackGlint`]). The pack's `cit.properties` (`Global`) says how
//! layers share an item.
//!
//! Written from OptiFine's `cit.properties` doc and CIT Resewn's (MIT) `TypeEnchantment` of
//! before its 1.21.11 port, which implemented `method`, `cap` and `fade`: the sums and the
//! cycle's fading follow it, except where noted.

use super::cit::read_properties;
use crate::cit_glint::{GlintLayer, StackGlint};
use horizon_optifine::cit::{Global, Method, Rule};
use std::path::Path;

/// The pack's global `cit.properties` under the overlay resource root: the first of
/// `Global::PATHS` that exists, else the defaults.
pub fn load_global(root: &Path) -> Global {
    Global::PATHS
        .iter()
        .find_map(|path| read_properties(&root.join(path)))
        .map(|props| Global::parse(&props))
        .unwrap_or_default()
}

/// A stack's glint: the layers among `matched` (its matching enchantment rules, in priority
/// order, as `CitRules::enchantments` gives them) that draw, for a stack with `enchantments`
/// (`ItemStack::crafting_enchantments`).
pub fn stack_glint(
    rules: &[Rule],
    matched: &[usize],
    enchantments: &[(impl AsRef<str>, u32)],
    global: &Global,
) -> StackGlint {
    // The highest priority rule of each layer.
    let mut layers: Vec<usize> = Vec::new();
    for &i in matched {
        let layer = rules[i].glint.layer;
        if !layers.iter().any(|&j| rules[j].glint.layer == layer) {
            layers.push(i);
        }
    }
    layers.sort_by_key(|&i| rules[i].glint.layer);
    let cycle = global.method == Method::Cycle;
    // OptiFine's doc: `cap` is for `average` and `layered`, and keeps the topmost layers.
    // `none`, which OptiFine lacks, is capped too, as CIT Resewn caps every method (it keeps
    // the bottom layers, though, and caps cycles as well).
    if let Some(cap) = global.cap.filter(|_| !cycle) {
        let cap = usize::try_from(cap).unwrap_or(usize::MAX);
        layers = layers.split_off(layers.len().saturating_sub(cap));
    }
    // CIT Resewn: any layer left asking for vanilla's glint keeps it, and the global
    // `useGlint=false` removes it everywhere, as OptiFine's doc says too. `cap=0` leaves no
    // custom layer, and vanilla's glint only if the pack's `useGlint` allows it.
    let vanilla =
        global.use_glint && (layers.is_empty() || layers.iter().any(|&i| rules[i].glint.use_glint));
    // CIT Resewn: shares are against every enchantment of the stack, not only those the
    // layers name.
    let levels = enchantments.iter().map(|(_, level)| *level as f32);
    let total = match global.method {
        Method::Average => levels.sum::<f32>(),
        Method::Layered => levels.fold(0.0, f32::max),
        Method::None | Method::Cycle => 0.0,
    };
    let shared = matches!(global.method, Method::Average | Method::Layered);
    let layers = layers
        .into_iter()
        .map(|i| {
            let rule = &rules[i];
            let named = &rule.enchantments;
            let weight = if !shared || named.is_empty() {
                1.0
            } else {
                // The highest level among the enchantments the rule names whose level also
                // passes its `enchantmentLevels` (CIT Resewn takes the first named one the
                // stack has, whatever its level).
                let in_range = |level: u32| {
                    let level = i32::try_from(level).unwrap_or(i32::MAX);
                    rule.enchantment_levels
                        .as_ref()
                        .is_none_or(|r| r.contains(level))
                };
                let level = enchantments
                    .iter()
                    .filter(|(id, level)| {
                        let id: &str = id.as_ref();
                        named.iter().any(|n| n == id) && in_range(*level)
                    })
                    .map(|(_, level)| *level as f32)
                    .fold(0.0, f32::max);
                if total > 0.0 { level / total } else { 0.0 }
            };
            GlintLayer::new(i, weight, rule.glint.duration)
        })
        .collect();
    StackGlint::from_parts(layers, vanilla, cycle.then_some(global.fade))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_optifine::Properties;

    const DIR: &str = "assets/minecraft/optifine/cit";

    /// An enchantment rule named `name` with `text` after `type` and `texture`.
    fn rule(name: &str, text: &str) -> Rule {
        let text = format!("type=enchantment\ntexture=glint\n{text}");
        let path = format!("{DIR}/{name}.properties");
        Rule::parse(&path, &Properties::parse_equals_first(&text), |_| true).unwrap()
    }

    fn global(text: &str) -> Global {
        Global::parse(&Properties::parse_equals_first(text))
    }

    fn rules_of(layers: &StackGlint) -> Vec<usize> {
        layers.layers.iter().map(|l| l.rule).collect()
    }

    const SWORD: [(&str, u32); 2] = [("minecraft:sharpness", 3), ("minecraft:unbreaking", 1)];

    #[test]
    fn one_rule_per_layer_bottom_first() {
        let rules = [
            rule("a", "layer=2"),
            rule("b", "layer=0"),
            rule("c", "layer=2"),
            rule("d", "layer=-1"),
        ];
        // Priority order a, b, c, d: c loses layer 2 to a.
        let g = stack_glint(&rules, &[0, 1, 2, 3], &SWORD, &Global::default());
        assert_eq!(rules_of(&g), [3, 1, 0]);
        let g = stack_glint(&rules, &[2, 0], &SWORD, &Global::default());
        assert_eq!(rules_of(&g), [2]);
    }

    #[test]
    fn average_and_layered_shares() {
        let rules = [
            rule("sharp", "enchantments=sharpness smite"),
            rule("unb", "layer=1\nenchantments=unbreaking"),
            rule("any", "layer=2"),
            rule("fire", "layer=3\nenchantments=fire_aspect"),
        ];
        let weights = |method: &str| {
            let g = stack_glint(&rules, &[0, 1, 2, 3], &SWORD, &global(method));
            g.layers.iter().map(|l| l.weight).collect::<Vec<_>>()
        };
        // Sharpness III and Unbreaking I: 3 / 4 and 1 / 4; against the highest, 3 / 3 and 1 / 3.
        // A rule naming no enchantment is at full strength, one the stack lacks at none.
        assert_eq!(weights("method=average"), [0.75, 0.25, 1.0, 0.0]);
        assert_eq!(weights("method=layered"), [1.0, 1.0 / 3.0, 1.0, 0.0]);
        assert_eq!(weights("method=cycle"), [1.0; 4]);
        assert_eq!(weights("method=none"), [1.0; 4]);
    }

    #[test]
    fn the_level_is_one_the_rules_levels_allow() {
        let rules = [rule(
            "holy",
            "enchantments=sharpness smite\nenchantmentLevels=1-2",
        )];
        // Sharpness V is out of range, so Smite I sets the share: 1 / 5 of the highest level.
        let stack = [("minecraft:sharpness", 5), ("minecraft:smite", 1)];
        let g = stack_glint(&rules, &[0], &stack, &global("method=layered"));
        assert_eq!(g.layers[0].weight, 1.0 / 5.0);
    }

    #[test]
    fn cap_keeps_the_topmost_layers() {
        let rules = [
            rule("a", "layer=0"),
            rule("b", "layer=1"),
            rule("c", "layer=2"),
        ];
        let g = stack_glint(&rules, &[0, 1, 2], &SWORD, &global("cap=2"));
        assert_eq!(rules_of(&g), [1, 2]);
        let g = stack_glint(&rules, &[0, 1, 2], &SWORD, &global("cap=0"));
        assert!(g.is_empty());
        let g = stack_glint(&rules, &[0, 1, 2], &SWORD, &global("cap=1\nmethod=none"));
        assert_eq!(rules_of(&g), [2]);
        // Cycles show every layer in turn.
        let g = stack_glint(&rules, &[0, 1, 2], &SWORD, &global("cap=1\nmethod=cycle"));
        assert_eq!(rules_of(&g), [0, 1, 2]);
    }

    #[test]
    fn vanilla_glint() {
        let plain = rule("plain", "");
        let keep = rule("keep", "layer=1\nuseGlint=true");
        let rules = [plain, keep];
        let vanilla = |matched: &[usize], text: &str| {
            stack_glint(&rules, matched, &SWORD, &global(text)).vanilla
        };
        assert!(vanilla(&[], ""), "no rule: vanilla's glint");
        assert!(!vanilla(&[0], ""), "a rule replaces it");
        assert!(vanilla(&[0, 1], ""), "unless one keeps it");
        assert!(!vanilla(&[], "useGlint=false"));
        assert!(!vanilla(&[1], "useGlint=false"), "the pack's word is final");
        // Only the layers `cap` keeps count.
        let rules = [rule("keep", "useGlint=true"), rule("top", "layer=1")];
        let g = stack_glint(&rules, &[0, 1], &SWORD, &global("cap=1"));
        assert!(!g.vanilla);
        let g = stack_glint(&rules, &[0, 1], &SWORD, &global("cap=0"));
        assert!(g.is_empty() && g.vanilla);
        let g = stack_glint(&rules, &[0, 1], &SWORD, &global("cap=0\nuseGlint=false"));
        assert!(!g.vanilla);
    }

    #[test]
    fn cycles_take_turns_and_fade() {
        let rules = [
            rule("a", "duration=2"),
            rule("b", "layer=1\nduration=1"),
            rule("c", "layer=2\nduration=1"),
        ];
        let g = stack_glint(
            &rules,
            &[0, 1, 2],
            &SWORD,
            &global("method=cycle\nfade=0.5"),
        );
        // a 0-2, a→b 2-2.5, b 2.5-3.5, b→c 3.5-4, c 4-5, c→a 5-5.5, then again.
        let at = |t: f64| [0, 1, 2].map(|i| g.intensity(i, t));
        assert_eq!(at(1.0), [1.0, 0.0, 0.0]);
        assert_eq!(at(2.125), [0.75, 0.25, 0.0]);
        assert_eq!(at(3.0), [0.0, 1.0, 0.0]);
        assert_eq!(at(3.75), [0.0, 0.5, 0.5]);
        assert_eq!(at(4.5), [0.0, 0.0, 1.0]);
        assert_eq!(at(5.25), [0.5, 0.0, 0.5]);
        assert_eq!(at(5.5 + 3.0), [0.0, 1.0, 0.0]);
        // One layer always shows.
        let one = stack_glint(&rules, &[1], &SWORD, &global("method=cycle"));
        assert_eq!(one.intensity(0, 0.7), 1.0);
    }

    #[test]
    fn a_cycle_always_shows_a_whole_layer() {
        let rules = [
            rule("a", "duration=0.1"),
            rule("b", "layer=1\nduration=0.7"),
            rule("c", "layer=2\nduration=0.2"),
        ];
        let g = stack_glint(
            &rules,
            &[0, 1, 2],
            &SWORD,
            &global("method=cycle\nfade=0.3"),
        );
        // Every millisecond of the first two seconds, and just under a period, the last fade's
        // end. (Rounding has not been seen to leave time past it; the last fade's clamp is
        // defensive.)
        let period: f64 = [0.1f32, 0.7, 0.2]
            .map(|d| f64::from(d) + f64::from(0.3f32))
            .iter()
            .sum();
        let last = f64::from_bits(period.to_bits() - 1);
        for t in (0..2000_i32)
            .map(|i| f64::from(i) * 0.001)
            .chain([last, period + last])
        {
            let sum: f32 = (0..3).map(|i| g.intensity(i, t)).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{sum} at {t}");
        }
        // No time at all: the bottom layer alone, never NaN.
        let still = [rule("a", "duration=0"), rule("b", "layer=1\nduration=0")];
        let g = stack_glint(&still, &[0, 1], &SWORD, &global("method=cycle\nfade=0"));
        for t in [0.0, 0.5, 7.25] {
            assert_eq!([0, 1].map(|i| g.intensity(i, t)), [1.0, 0.0]);
        }
    }

    #[test]
    fn the_first_global_file_wins() {
        let root = std::env::temp_dir().join(format!("horizon-cit-glint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(load_global(&root), Global::default());
        let write = |path: &str, text: &[u8]| {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        };
        // Saved as Latin-1 (`é` is not UTF-8 alone), in the OptiFine doc's `cit/` folder.
        write(
            "assets/minecraft/optifine/cit/cit.properties",
            b"# caf\xE9\nmethod=cycle",
        );
        assert_eq!(load_global(&root).method, Method::Cycle);
        write("assets/minecraft/mcpatcher/cit.properties", b"method=none");
        assert_eq!(load_global(&root).method, Method::Cycle);
        write(
            "assets/minecraft/optifine/cit.properties",
            b"method=layered",
        );
        assert_eq!(load_global(&root).method, Method::Layered);
        write("assets/minecraft/citresewn/cit.properties", b"cap=1");
        let g = load_global(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!((g.method, g.cap), (Method::Average, Some(1)));
    }
}
