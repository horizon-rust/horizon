//! Custom item textures: rules a pack format vanilla doesn't read (OptiFine's and CIT Resewn's
//! `cit/` folders) gives, which change a stack's texture, model, worn armour layers or
//! enchantment glint. The platform's pack formats read and match them ([`crate::pack_hooks`]);
//! the engine draws what the matching rule says ([`Rule`]).

use std::path::Path;
use std::sync::Arc;

use horizon_items::items::ItemStack;

use crate::cit_glint::StackGlint;

/// What a rule changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Item,
    Armor,
    Elytra,
    Enchantment,
}

/// The hand a stack is drawn in; GUI items count as the main hand. `Any` is a rule's "either".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Any,
    Main,
    Off,
}

/// An OpenGL blend factor (`GL_ONE`, `GL_SRC_ALPHA`...), as `glBlendFuncSeparate` takes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    SrcColor,
    OneMinusSrcColor,
    SrcAlpha,
    OneMinusSrcAlpha,
    DstAlpha,
    OneMinusDstAlpha,
    DstColor,
    OneMinusDstColor,
    SrcAlphaSaturate,
}

/// A glint layer's blend factors for colour and alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blend {
    pub src: Factor,
    pub dst: Factor,
    pub src_alpha: Factor,
    pub dst_alpha: Factor,
}

/// An enchantment rule's glint layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Glint {
    pub layer: i32,
    /// `None` draws without blending (replace).
    pub blend: Option<Blend>,
    /// Scroll speed: the texture runs through vanilla's glint periods at `speed` times the
    /// clock in milliseconds.
    pub speed: f32,
    /// Degrees the texture turns by.
    pub rotation: f32,
    /// Seconds on this layer when the layers take turns.
    pub duration: f32,
    /// Multipliers of the texture's colour, never negative.
    pub rgba: [f32; 4],
    /// Whether the default glint still draws under this layer.
    pub use_glint: bool,
    /// Linear filtering, else nearest.
    pub blur: bool,
}

/// A rule as the engine draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// The rule's file, as a resource path.
    pub path: String,
    pub kind: Kind,
    /// Its texture, as a resource path to a `.png`.
    pub texture: Option<String>,
    /// Its named textures: (name, resource path).
    pub textures: Vec<(String, String)>,
    /// Its model, as a resource path to a `.json`.
    pub model: Option<String>,
    /// Its named models.
    pub models: Vec<(String, String)>,
    /// What an Enchantment rule draws.
    pub glint: Glint,
}

/// Which rules a stack matches: the pack format's own conditions.
pub trait Matcher: Send + Sync {
    /// The winning rule of `kind` (Item, Armor or Elytra) for this stack, as an index into
    /// [`CitRules::rules`].
    fn select(&self, kind: Kind, stack: &ItemStack, hand: Hand, max_damage: u32) -> Option<usize>;
    /// Every matching Enchantment rule for the stack drawn in `hand`, in priority order.
    fn enchantments(&self, stack: &ItemStack, hand: Hand, max_damage: u32) -> Vec<usize>;
    /// The glint layers this stack draws in `hand`; `None` when the pack changes no glint.
    /// `foil` is whether the stack draws a glint at all.
    fn glint(
        &self,
        stack: &ItemStack,
        hand: Hand,
        max_damage: u32,
        foil: bool,
    ) -> Option<Arc<StackGlint>>;
    /// Leave out the Enchantment rules `keep` refuses: they match no stack.
    fn keep_glints(&mut self, keep: &dyn Fn(usize) -> bool);
}

/// The packs' custom item texture rules, the winner first, and their matcher.
pub struct CitRules {
    pub rules: Vec<Rule>,
    /// Whether the default glint draws on items no enchantment rule matches.
    pub use_glint: bool,
    matcher: Option<Box<dyn Matcher>>,
}

impl Default for CitRules {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            use_glint: true,
            matcher: None,
        }
    }
}

impl std::fmt::Debug for CitRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CitRules")
            .field("rules", &self.rules)
            .field("use_glint", &self.use_glint)
            .finish_non_exhaustive()
    }
}

impl CitRules {
    /// The rules of the overlay at `root`, from the installed pack formats.
    pub fn load(root: &Path) -> Self {
        crate::pack_hooks::formats()
            .item_textures(root)
            .unwrap_or_default()
    }

    /// `rules`, in priority order, matched by `matcher`.
    pub fn new(rules: Vec<Rule>, use_glint: bool, matcher: Box<dyn Matcher>) -> Self {
        Self {
            rules,
            use_glint,
            matcher: Some(matcher),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Leave out the Enchantment rules `keep` refuses (those without a drawable texture): they
    /// match no stack, so vanilla's glint and the other rules of their layer stay.
    pub fn keep_glints(&mut self, keep: impl Fn(usize) -> bool) {
        if let Some(matcher) = &mut self.matcher {
            matcher.keep_glints(&keep);
        }
    }

    /// [`Matcher::select`].
    pub fn select(
        &self,
        kind: Kind,
        stack: &ItemStack,
        hand: Hand,
        max_damage: u32,
    ) -> Option<usize> {
        self.matcher.as_ref()?.select(kind, stack, hand, max_damage)
    }

    /// [`Matcher::enchantments`].
    pub fn enchantments(&self, stack: &ItemStack, hand: Hand, max_damage: u32) -> Vec<usize> {
        self.matcher
            .as_ref()
            .map_or_else(Vec::new, |m| m.enchantments(stack, hand, max_damage))
    }

    /// [`Matcher::glint`].
    pub fn glint(
        &self,
        stack: &ItemStack,
        hand: Hand,
        max_damage: u32,
        foil: bool,
    ) -> Option<Arc<StackGlint>> {
        self.matcher.as_ref()?.glint(stack, hand, max_damage, foil)
    }
}

/// Rules and blends as a pack format gives them, for the engine's tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// `src dst`, the alpha factors GL_ZERO and GL_ONE (the target's alpha stays).
    pub(crate) fn pair(src: Factor, dst: Factor) -> Option<Blend> {
        Some(Blend {
            src,
            dst,
            src_alpha: Factor::Zero,
            dst_alpha: Factor::One,
        })
    }

    /// The glint layer a rule draws when it sets nothing: added, still, drawn blurred.
    pub(crate) fn glint() -> Glint {
        Glint {
            layer: 0,
            blend: pair(Factor::SrcAlpha, Factor::One),
            speed: 0.0,
            rotation: 0.0,
            duration: 1.0,
            rgba: [1.0; 4],
            use_glint: false,
            blur: true,
        }
    }

    /// A rule of `kind` in `dir` (`assets/minecraft/optifine/cit`) named `name`, drawing
    /// `texture` and the named `textures` (names of `.png`s beside it).
    pub(crate) fn rule(
        kind: Kind,
        dir: &str,
        name: &str,
        texture: Option<&str>,
        textures: &[(&str, &str)],
    ) -> Rule {
        let png = |t: &str| format!("{dir}/{t}.png");
        Rule {
            path: format!("{dir}/{name}.properties"),
            kind,
            texture: texture.map(png),
            textures: textures
                .iter()
                .map(|(n, t)| ((*n).to_owned(), png(t)))
                .collect(),
            model: None,
            models: Vec::new(),
            glint: glint(),
        }
    }
}
