//! Owned item values. Registry-dependent validation belongs to the game's write boundary.
use crate::ItemKey;
use horizon_core::nbt::{Compound, JavaString, Tag};
use std::num::NonZeroU32;
use std::sync::Arc;

/// A typed descriptor for one component in an item's component patch.
/// Implementations own the conversion between their value and Minecraft NBT.
pub trait Component {
    /// The value returned by this descriptor.
    type Value;
    /// Canonical component name, including its namespace.
    fn id(&self) -> &str;
    /// Decode a component value, refusing an incompatible NBT representation.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// Encode a component value.
    fn encode(&self, value: Self::Value) -> Tag;
}

/// An owned component patch, independent of the engine's component representation.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Components {
    patch: Compound,
}

impl Components {
    /// Borrow the raw component patch at an NBT boundary.
    pub fn as_nbt(&self) -> &Compound {
        &self.patch
    }
}
impl From<Compound> for Components {
    fn from(patch: Compound) -> Self {
        Self { patch }
    }
}
impl From<&Components> for Compound {
    fn from(value: &Components) -> Self {
        value.patch.clone()
    }
}

/// A nonempty, owned item value. Maxima are checked when the game accepts a write,
/// not by this record: item definitions and effective component overrides belong to the game.
/// The component patch is shared copy-on-write: `Clone` counts a reference instead of
/// copying it, the first write through a shared stack copies the patch once, and equality
/// of two stacks that still share it is a pointer comparison. An absent patch allocates nothing.
///
/// An untouched mutable stack hook argument retains its engine value without
/// write validation, even when the engine value exceeds effective maxima. Equality
/// with the snapshot passed to the listener determines whether it changed; a changed
/// argument is validated in full before committing any hook outputs.
/// A refused hook output retains its captured engine value. The clicked-slot bridge also
/// stages the slot, cursor and cancellation flags together. Independent host effects,
/// including storage writes, commands and messages, are not undone by output validation.
// no wire: decoding rejects zero counts through new and validates component patches through set_components; damage is applied through set_damage.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemStack {
    item: ItemKey,
    count: NonZeroU32,
    damage: Option<u32>,
    components: Option<Arc<Components>>,
}

/// A stack operation could not represent the requested count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StackError {
    /// Empty stacks are represented by `None`, not by a zero-count item.
    Empty,
    /// Adding items would overflow the record's count.
    CountOverflow,
    /// Damage must be a nonnegative integer representable by u32.
    InvalidDamage,
    /// A patch and the damage field (or a patch value and removal) disagree.
    ConflictingDamage,
}
impl std::fmt::Display for StackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("an item stack must be nonempty"),
            Self::CountOverflow => f.write_str("item stack count overflow"),
            Self::InvalidDamage => f.write_str("invalid item damage"),
            Self::ConflictingDamage => f.write_str("conflicting item damage"),
        }
    }
}
impl std::error::Error for StackError {}

/// Why the game refused to accept an owned stack at a write boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum StackWriteError {
    /// A text field failed component conversion before this write.
    Text(crate::text::TextFormError),
    /// The host has no game backend for this operation.
    Unavailable,
    /// The process-local key is not present in this game's item registry.
    UnknownItem,
    /// The count is above the effective stack limit.
    CountExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// The damage is above the effective durability limit.
    DamageExceeded {
        /// Effective maximum, including component overrides.
        maximum: u32,
    },
    /// A maximum is malformed or negative, or a stack limit lies outside 1..=99.
    InvalidMaximum,
    /// An engine result contained a zero-count stack instead of an empty slot.
    EmptyEngineStack,
    /// An engine snapshot's component patch was malformed or conflicted with its damage.
    InvalidComponents {
        /// The record-level normalization refusal.
        reason: StackError,
    },
    /// A consume or death-protection component could not decode its opaque effect payload.
    InvalidEffectPayload,
    /// A stack carries both maximum durability and a stack limit above one.
    StackableDamageable,
}

impl From<crate::text::TextFormError> for StackWriteError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for StackWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// The two nonempty portions of a consuming split. Either portion can be absent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct StackSplit {
    /// Up to the requested number of items.
    pub taken: Option<ItemStack>,
    /// Items left after taking that portion.
    pub remainder: Option<ItemStack>,
}

/// The most lore lines the item packet reader takes.
const MAX_LORE_LINES: usize = 256;
/// The most entries of a container or bundle `contained_stacks` reads.
const MAX_CONTAINED: usize = 256;
/// How deep `nesting_depth` looks, and how many stacks it visits.
const MAX_NESTING: u32 = 64;
const MAX_NESTING_VISITS: usize = 65_536;

impl ItemStack {
    /// Construct a nonempty stack. Registry maxima are validated by the game on writes.
    pub fn new(item: ItemKey, count: u32) -> Option<Self> {
        Some(Self {
            item,
            count: NonZeroU32::new(count)?,
            damage: None,
            components: None,
        })
    }
    /// Construct a single item without a component patch.
    pub fn of(item: ItemKey) -> Self {
        Self {
            item,
            count: NonZeroU32::MIN,
            damage: None,
            components: None,
        }
    }
    /// The process-local identity of this item.
    pub fn item(&self) -> ItemKey {
        self.item
    }
    /// The nonzero item count.
    pub fn count(&self) -> u32 {
        self.count.get()
    }
    /// Change the count without consulting the game. A refusal leaves the value unchanged.
    pub fn set_count(&mut self, count: u32) -> Result<(), StackError> {
        self.count = NonZeroU32::new(count).ok_or(StackError::Empty)?;
        Ok(())
    }
    /// Stored damage, or zero when no numeric damage override is present.
    /// A plain stack inherits its item's default. Some(0) is an explicit present zero;
    /// [`Self::clear_damage`] records removal of the default component instead.
    pub fn damage(&self) -> u32 {
        self.damage.unwrap_or(0)
    }
    /// The numeric damage override: None inherits the prototype unless the patch
    /// contains an explicit removal; Some(0) is a present zero-damage component.
    pub fn damage_component(&self) -> Option<u32> {
        self.damage
    }
    /// Set damage. The game checks the effective maximum when accepting this value.
    pub fn set_damage(&mut self, damage: u32) {
        self.damage = Some(damage);
        self.remove_patch_entries(&[JavaString::from("!minecraft:damage")]);
    }
    /// Remove the effective damage component, including an inherited default.
    /// The patch stores a real NBT removal, never a second numeric damage value.
    pub fn clear_damage(&mut self) {
        self.remove_component_value("minecraft:damage", true);
    }
    /// Whether a nonzero numeric damage override is stored on the stack.
    pub fn is_damaged(&self) -> bool {
        self.damage.is_some_and(|damage| damage != 0)
    }
    /// Add items. Overflow leaves the stack unchanged; registry maxima are checked on writes.
    pub fn grow(&mut self, count: u32) -> Result<(), StackError> {
        let count = self
            .count()
            .checked_add(count)
            .ok_or(StackError::CountOverflow)?;
        self.set_count(count)
    }
    /// Remove items, consuming the value so removing everything needs no empty sentinel.
    pub fn shrink(mut self, count: u32) -> Option<Self> {
        self.count = NonZeroU32::new(self.count().saturating_sub(count))?;
        Some(self)
    }
    /// Take up to `count` items. A full split moves the patch without cloning it.
    pub fn split(mut self, count: u32) -> StackSplit {
        if count == 0 {
            return StackSplit {
                taken: None,
                remainder: Some(self),
            };
        }
        if count >= self.count() {
            return StackSplit {
                taken: Some(self),
                remainder: None,
            };
        }
        let mut taken = self.clone();
        taken.count = NonZeroU32::new(count).expect("partial split is nonzero");
        self.count = NonZeroU32::new(self.count() - count).expect("partial split leaves items");
        StackSplit {
            taken: Some(taken),
            remainder: Some(self),
        }
    }
    /// Compare item identities, independent of count and components.
    pub fn same_item(&self, other: &Self) -> bool {
        self.item == other.item
    }
    /// Compare component patches and damage, independent of count and item identity.
    pub fn same_components(&self, other: &Self) -> bool {
        self.damage == other.damage && self.components == other.components
    }
    /// Whether these values have the same item and component patch.
    /// Capacity is a registry-dependent write-boundary check, not a promise of this predicate.
    pub fn is_stackable_with(&self, other: &Self) -> bool {
        self.same_item(other) && self.same_components(other)
    }
    /// Read a component with its typed descriptor.
    pub fn get<C: Component>(&self, component: &C) -> Option<C::Value> {
        if component.id() == "minecraft:damage" {
            return self
                .damage
                .and_then(|damage| component.decode(&unsigned_tag(damage)));
        }
        component.decode(
            self.components
                .as_ref()?
                .patch
                .get(&JavaString::from(component.id()))?,
        )
    }
    /// Replace a component value. Only stacks with a patch allocate component storage.
    pub fn set<C: Component>(&mut self, component: &C, value: C::Value) -> Result<(), StackError> {
        self.set_component_value(component.id(), component.encode(value))
    }
    pub(crate) fn set_component_value(&mut self, id: &str, value: Tag) -> Result<(), StackError> {
        if id == "minecraft:damage" {
            self.set_damage(damage_value(&value)?);
            return Ok(());
        }
        if id == "!minecraft:damage" {
            if !matches!(&value, Tag::Compound(value) if value.is_empty()) {
                return Err(StackError::InvalidDamage);
            }
            if self.damage.is_some() {
                return Err(StackError::ConflictingDamage);
            }
        }
        let patch = &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
        patch.remove(&JavaString::from(format!("!{id}").as_str()));
        patch.insert(JavaString::from(id), value);
        Ok(())
    }
    /// Remove an override, releasing the patch allocation when its last entry is removed.
    pub fn remove<C: Component>(&mut self, component: &C) {
        self.remove_component_value(component.id(), false);
    }
    pub(crate) fn remove_component_value(&mut self, id: &str, removal: bool) {
        if id == "minecraft:damage" {
            self.damage = None;
        }
        let removed = JavaString::from(format!("!{id}").as_str());
        if removal {
            let patch =
                &mut Arc::make_mut(self.components.get_or_insert_with(Default::default)).patch;
            patch.remove(&JavaString::from(id));
            patch.insert(removed, Tag::Compound(Compound::new()));
            return;
        }
        self.remove_patch_entries(&[JavaString::from(id), removed]);
    }
    /// Remove entries from an existing patch. A shared patch is copied only when one of
    /// the keys is present; the patch is released when its last entry is removed.
    fn remove_patch_entries(&mut self, keys: &[JavaString]) {
        let Some(components) = &mut self.components else {
            return;
        };
        if !keys.iter().any(|key| components.patch.contains_key(key)) {
            return;
        }
        let patch = &mut Arc::make_mut(components).patch;
        for key in keys {
            patch.remove(key);
        }
        if patch.is_empty() {
            self.components = None;
        }
    }
    /// The component patch, if this stack has overrides.
    pub fn components(&self) -> Option<&Components> {
        self.components.as_deref()
    }
    /// `ItemStack.applyComponents`: merge a patch without discarding other overrides.
    /// Incoming values replace removals and incoming removals replace values. If both
    /// are supplied for one component, removal wins (conflicting damage is refused).
    /// Damage is normalized before any write; a malformed patch leaves this stack unchanged.
    pub fn apply_components(&mut self, patch: &Compound) -> Result<(), StackError> {
        let damage = patch_damage(patch)?;
        for (id, value) in patch {
            let id = id.to_string_lossy();
            if id == "minecraft:damage" {
                continue;
            }
            if let Some(id) = id.strip_prefix('!') {
                self.remove_component_value(id, true);
            } else if !patch.contains_key(&JavaString::from(format!("!{id}").as_str())) {
                self.set_component_value(&id, value.clone())?;
            }
        }
        if let Some(damage) = damage {
            self.set_damage(damage);
        }
        Ok(())
    }
    /// Copy this component's own value or explicit removal from `other`, without
    /// consulting item defaults. If `other` inherits it, clear this stack's override too.
    /// An `id` that is not a namespaced component id (`Damage`, or a removal marker such as
    /// `!minecraft:damage`) changes nothing.
    pub fn copy_component_from(&mut self, other: &Self, id: &str) {
        if crate::ComponentId::new(id).is_none() {
            return;
        }
        if id == "minecraft:damage"
            && let Some(damage) = other.damage
        {
            self.set_damage(damage);
            return;
        }
        let patch = other.components().map(Components::as_nbt);
        let removed = JavaString::from(format!("!{id}").as_str());
        if patch.is_some_and(|patch| patch.contains_key(&removed)) {
            self.remove_component_value(id, true);
        } else if let Some(value) = patch.and_then(|patch| patch.get(&JavaString::from(id))) {
            self.set_component_value(id, value.clone())
                .expect("stored component is normalized");
        } else {
            self.remove_component_value(id, false);
        }
    }
    /// Replace the component patch at an NBT conversion boundary.
    /// Damage is normalized into the scalar field. Malformed or conflicting values
    /// refuse the entire replacement and leave the stack unchanged.
    pub fn set_components(&mut self, mut patch: Compound) -> Result<(), StackError> {
        let damage = patch_damage(&patch)?;
        let removed = patch.contains_key(&JavaString::from("!minecraft:damage"));
        if damage.is_some_and(|value| self.damage.is_some_and(|damage| damage != value))
            || (removed && self.damage.is_some())
        {
            return Err(StackError::ConflictingDamage);
        }
        let next = damage.or(self.damage);
        patch.remove(&JavaString::from("minecraft:damage"));
        self.damage = next;
        self.components = (!patch.is_empty()).then(|| Arc::new(Components::from(patch)));
        Ok(())
    }
    /// Decode the item NBT escape using this host's load-resolved item ids.
    /// Unknown ids, zero counts and malformed/conflicting damage return None.
    /// Effective maxima remain the game's write-boundary responsibility.
    pub fn from_nbt(host: &dyn crate::Host, value: &Compound) -> Option<Self> {
        let Tag::String(id) = value.get(&JavaString::from("id"))? else {
            return None;
        };
        let id = crate::Identifier::parse(&id.to_string_lossy()).ok()?;
        let item = host.item_key(&id)?;
        let count = match value.get(&JavaString::from("count")) {
            None => 1,
            Some(value) => damage_value(value).ok()?,
        };
        let mut stack = Self::new(item, count)?;
        match value.get(&JavaString::from("components")) {
            Some(Tag::Compound(patch)) => stack.set_components(patch.clone()).ok()?,
            None => {}
            _ => return None,
        }
        Some(stack)
    }
    /// Sets the stack's shown name and lore (`minecraft:custom_name`, `minecraft:lore`), as
    /// Bukkit's `ItemMeta.displayName` and `lore` do: None removes the name, an empty `lore`
    /// removes the lore. The texts become component NBT through the host
    /// (`TextHost::text_to_nbt`); vanilla draws a custom name in italics unless its style says
    /// `italic: false`. Nothing changes when a text is refused, or when `lore` has more than 256
    /// lines (`Malformed`: the item packet reader takes at most 256). Clearing the name or the lore
    /// records a removal when the item's prototype supplies that component, so its default does not
    /// come back.
    pub fn set_display(
        &mut self,
        host: &dyn crate::Host,
        name: Option<&crate::Text>,
        lore: &[crate::Text],
    ) -> Result<(), crate::TextFormError> {
        if lore.len() > MAX_LORE_LINES {
            return Err(crate::TextFormError::Malformed);
        }
        let name = name.map(|name| host.text_to_nbt(name)).transpose()?;
        let lore = lore
            .iter()
            .map(|line| host.text_to_nbt(line))
            .collect::<Result<Vec<_>, _>>()?;
        match name {
            Some(name) => {
                let _ = self.set_component_value("minecraft:custom_name", name);
            }
            None => {
                let default = host.item_default_component(self.item, "minecraft:custom_name");
                self.remove_component_value("minecraft:custom_name", default.is_some());
            }
        }
        if lore.is_empty() {
            let default = host.item_default_component(self.item, "minecraft:lore");
            self.remove_component_value("minecraft:lore", default.is_some());
        } else {
            let _ = self.set_component_value("minecraft:lore", Tag::List(lore));
        }
        Ok(())
    }
    /// The enchantments the stack carries itself (`minecraft:enchantments`, Bukkit's
    /// `ItemMeta.getEnchants`): each registry id with its level, in id order. A prototype's
    /// enchantments are not the stack's own, and an entry whose level is not a nonnegative
    /// integer is left out.
    pub fn enchantments(&self) -> Vec<(String, u32)> {
        self.enchantment_levels("minecraft:enchantments")
    }
    /// The enchantments an enchanted book stores (`minecraft:stored_enchantments`, Bukkit's
    /// `EnchantmentStorageMeta.getStoredEnchants`), read as [`ItemStack::enchantments`] reads them.
    pub fn stored_enchantments(&self) -> Vec<(String, u32)> {
        self.enchantment_levels("minecraft:stored_enchantments")
    }
    /// The `id: level` compound of an enchantment component, as (id, level) pairs.
    fn enchantment_levels(&self, id: &str) -> Vec<(String, u32)> {
        let Some(Tag::Compound(levels)) = self
            .components()
            .and_then(|c| c.as_nbt().get(&JavaString::from(id)))
        else {
            return Vec::new();
        };
        levels
            .iter()
            .filter_map(|(key, level)| match level {
                Tag::Int(level) => u32::try_from(*level)
                    .ok()
                    .map(|level| (key.to_string_lossy(), level)),
                _ => None,
            })
            .collect()
    }

    /// Force the enchantment glint on or off (`minecraft:enchantment_glint_override`, Bukkit's
    /// `ItemMeta.setEnchantmentGlintOverride`); `None` returns the stack to the item's own rule.
    /// Clearing records a removal when the item's prototype sets the component, so its default does
    /// not come back.
    pub fn set_glint_override(&mut self, host: &dyn crate::Host, glint: Option<bool>) {
        const ID: &str = "minecraft:enchantment_glint_override";
        match glint {
            Some(on) => {
                let _ = self.set_component_value(ID, Tag::Byte(i8::from(on)));
            }
            None => {
                let default = host.item_default_component(self.item, ID);
                self.remove_component_value(ID, default.is_some());
            }
        }
    }
    /// Hide or show the whole tooltip (`minecraft:tooltip_display.hide_tooltip`, Bukkit's
    /// `ItemMeta.setHideTooltip`). The other keys of the display, such as `hidden_components`, are kept.
    /// The stack's own display, else the item's prototype display, is the base; a display left with no
    /// key is removed, recording a removal when the prototype sets it.
    pub fn set_tooltip_hidden(&mut self, host: &dyn crate::Host, hidden: bool) {
        const ID: &str = "minecraft:tooltip_display";
        let own = self
            .components()
            .and_then(|c| c.as_nbt().get(&JavaString::from(ID)).cloned());
        let mut display = match own.or_else(|| host.item_default_component(self.item, ID)) {
            Some(Tag::Compound(display)) => display,
            _ => Compound::new(),
        };
        if hidden {
            display.insert(JavaString::from("hide_tooltip"), Tag::Byte(1));
        } else {
            display.remove(&JavaString::from("hide_tooltip"));
        }
        if display.is_empty() {
            let default = host.item_default_component(self.item, ID);
            self.remove_component_value(ID, default.is_some());
        } else {
            let _ = self.set_component_value(ID, Tag::Compound(display));
        }
    }
    /// Dye the stack's colour (`minecraft:dyed_color`, Bukkit's `LeatherArmorMeta.setColor`): `Some(rgb)`
    /// sets the 24-bit colour (bits above 24 are dropped) and keeps the component's other keys;
    /// `None` returns the stack to the item's own colour, recording a removal when the prototype sets it.
    pub fn set_leather_color(&mut self, host: &dyn crate::Host, rgb: Option<u32>) {
        const ID: &str = "minecraft:dyed_color";
        match rgb {
            Some(rgb) => {
                let own = self
                    .components()
                    .and_then(|c| c.as_nbt().get(&JavaString::from(ID)).cloned());
                let mut color = match own {
                    Some(Tag::Compound(color)) => color,
                    _ => Compound::new(),
                };
                color.insert(
                    JavaString::from("rgb"),
                    Tag::Int((rgb & 0x00FF_FFFF) as i32),
                );
                let _ = self.set_component_value(ID, Tag::Compound(color));
            }
            None => {
                let default = host.item_default_component(self.item, ID);
                self.remove_component_value(ID, default.is_some());
            }
        }
    }

    /// The stacks inside this stack, one level down: its `minecraft:container` (a shulker box, a
    /// chest or barrel item) in the order stored, then its `minecraft:bundle_contents` (a bundle).
    /// Read from the stack's own components, as `from_nbt` reads an item: an entry whose item the
    /// host does not know, whose count is zero or that is malformed is left out, and at most
    /// 256 entries of each are read. Empty for a stack with neither component. Call it again on
    /// a result to go further down, or use `nesting_depth`.
    pub fn contained_stacks(&self, host: &dyn crate::Host) -> Vec<ItemStack> {
        let mut stacks = Vec::new();
        if let Some(Tag::List(entries)) = self.component_value("minecraft:container") {
            for entry in entries.iter().take(MAX_CONTAINED) {
                let Tag::Compound(entry) = entry else {
                    continue;
                };
                if let Some(Tag::Compound(item)) = entry.get(&JavaString::from("item"))
                    && let Some(stack) = Self::from_nbt(host, item)
                {
                    stacks.push(stack);
                }
            }
        }
        if let Some(Tag::List(entries)) = self.component_value("minecraft:bundle_contents") {
            for entry in entries.iter().take(MAX_CONTAINED) {
                if let Tag::Compound(item) = entry
                    && let Some(stack) = Self::from_nbt(host, item)
                {
                    stacks.push(stack);
                }
            }
        }
        stacks
    }
    /// How many containers deep this stack nests: 0 for a stack that holds no stack, 1 for a
    /// shulker box of plain items, 2 for one with a bundle inside, and so on. The walk stops at
    /// 64 levels and answers 64, so a hostile item cannot make it deep or slow; with at most
    /// 256 entries per level it also stops after 65,536 stacks in all.
    pub fn nesting_depth(&self, host: &dyn crate::Host) -> u32 {
        let mut budget = MAX_NESTING_VISITS;
        self.nesting_depth_within(host, MAX_NESTING, &mut budget)
            .min(MAX_NESTING)
    }
    fn nesting_depth_within(&self, host: &dyn crate::Host, levels: u32, budget: &mut usize) -> u32 {
        if levels == 0 {
            return 0;
        }
        let mut deepest = 0;
        for inner in self.contained_stacks(host) {
            if *budget == 0 {
                return MAX_NESTING;
            }
            *budget -= 1;
            deepest = deepest.max(1 + inner.nesting_depth_within(host, levels - 1, budget));
        }
        deepest
    }
    /// This stack's own value of component `id` as raw NBT, None when its patch has none.
    fn component_value(&self, id: &str) -> Option<&Tag> {
        self.components()?.as_nbt().get(&JavaString::from(id))
    }
    /// Encode the item NBT escape using this host's canonical id, never a process-local key.
    /// Returns None when id metadata is unavailable. Only components present on the
    /// stack are written; effective defaults and maxima belong to the items host area.
    pub fn to_nbt(&self, host: &dyn crate::Host) -> Option<Compound> {
        let id = self.item.id(host);
        if id.is_empty() {
            return None;
        }
        let mut value = Compound::new();
        value.insert(JavaString::from("id"), Tag::String(id.as_str().into()));
        value.insert(JavaString::from("count"), unsigned_tag(self.count()));
        let mut patch = self
            .components()
            .map(|patch| patch.as_nbt().clone())
            .unwrap_or_default();
        if let Some(damage) = self.damage {
            patch.insert(JavaString::from("minecraft:damage"), unsigned_tag(damage));
        }
        if !patch.is_empty() {
            value.insert(JavaString::from("components"), Tag::Compound(patch));
        }
        Some(value)
    }
}

pub(crate) fn unsigned_tag(value: u32) -> Tag {
    i32::try_from(value).map_or_else(|_| Tag::Long(i64::from(value)), Tag::Int)
}

fn damage_value(value: &Tag) -> Result<u32, StackError> {
    let value = match value {
        Tag::Byte(value) => i64::from(*value),
        Tag::Short(value) => i64::from(*value),
        Tag::Int(value) => i64::from(*value),
        Tag::Long(value) => *value,
        _ => return Err(StackError::InvalidDamage),
    };
    u32::try_from(value).map_err(|_| StackError::InvalidDamage)
}

/// Validate the raw damage entries before either a replacement or a merge mutates a stack.
fn patch_damage(patch: &Compound) -> Result<Option<u32>, StackError> {
    let damage = patch.get(&JavaString::from("minecraft:damage"));
    let removal = patch.get(&JavaString::from("!minecraft:damage"));
    if removal.is_some_and(|value| !matches!(value, Tag::Compound(value) if value.is_empty())) {
        return Err(StackError::InvalidDamage);
    }
    let damage = damage.map(damage_value).transpose()?;
    if damage.is_some() && removal.is_some() {
        return Err(StackError::ConflictingDamage);
    }
    Ok(damage)
}
#[cfg(test)]
mod tests {
    use super::*;

    include!("item_stack_damage_tests.rs");
    include!("item_stack_component_tests.rs");

    /// A host whose text NBT is the literal string, and which refuses "bad".
    struct TextNbt;
    impl crate::prelude::TextHost for TextNbt {
        fn text_to_nbt(&self, text: &crate::Text) -> Result<Tag, crate::TextFormError> {
            match &text.content {
                crate::text::TextContent::Literal(s) if s == "bad" => {
                    Err(crate::TextFormError::Malformed)
                }
                crate::text::TextContent::Literal(s) => Ok(Tag::String(s.as_str().into())),
                _ => Err(crate::TextFormError::Malformed),
            }
        }
    }
    impl crate::Host for TextNbt {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            None
        }
    }
    /// Its items have no prototype defaults.
    impl crate::prelude::ItemsHost for TextNbt {
        fn item_key(&self, _: &crate::Identifier) -> Option<ItemKey> {
            None
        }
        fn item_default_component(&self, _: ItemKey, _: &str) -> Option<Tag> {
            None
        }
    }
    crate::impl_host_areas!(TextNbt, except(text, items));

    #[test]
    fn set_display_writes_and_clears_the_name_and_lore_components() {
        let name = |stack: &ItemStack| {
            stack.components().and_then(|c| {
                c.as_nbt()
                    .get(&JavaString::from("minecraft:custom_name"))
                    .cloned()
            })
        };
        let lore = |stack: &ItemStack| {
            stack
                .components()
                .and_then(|c| c.as_nbt().get(&JavaString::from("minecraft:lore")).cloned())
        };
        let mut stack = ItemStack::of(ItemKey(0));
        let lines = [crate::Text::literal("one"), crate::Text::literal("two")];
        stack
            .set_display(&TextNbt, Some(&crate::Text::literal("Sword")), &lines)
            .unwrap();
        assert_eq!(name(&stack), Some(Tag::String("Sword".into())));
        assert_eq!(
            lore(&stack),
            Some(Tag::List(vec![
                Tag::String("one".into()),
                Tag::String("two".into())
            ]))
        );
        // A refused text changes nothing.
        let before = stack.clone();
        assert_eq!(
            stack.set_display(&TextNbt, None, &[crate::Text::literal("bad")]),
            Err(crate::TextFormError::Malformed)
        );
        assert_eq!(stack, before);
        stack.set_display(&TextNbt, None, &[]).unwrap();
        assert!(stack.components().is_none(), "both components removed");
    }

    /// A host that knows three items by id.
    struct Shelf;
    impl crate::prelude::ItemsHost for Shelf {
        fn item_key(&self, id: &crate::Identifier) -> Option<ItemKey> {
            match id.to_string().as_str() {
                "minecraft:stone" => Some(ItemKey(1)),
                "minecraft:shulker_box" => Some(ItemKey(2)),
                "minecraft:bundle" => Some(ItemKey(3)),
                _ => None,
            }
        }
        fn item_default_component(&self, _: ItemKey, _: &str) -> Option<Tag> {
            None
        }
    }
    impl crate::Host for Shelf {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            None
        }
    }
    crate::impl_host_areas!(Shelf, except(items));

    fn item_tag(id: &str, count: i32, components: Option<(&str, Tag)>) -> Compound {
        let mut item = Compound::new();
        item.insert(JavaString::from("id"), Tag::String(id.into()));
        item.insert(JavaString::from("count"), Tag::Int(count));
        if let Some((name, value)) = components {
            let mut patch = Compound::new();
            patch.insert(JavaString::from(name), value);
            item.insert(JavaString::from("components"), Tag::Compound(patch));
        }
        item
    }

    /// A shulker box's `container` entries and a bundle's `bundle_contents` read in order, an
    /// unknown item or a zero count is left out, and the depth counts the boxes inside boxes.
    #[test]
    fn contained_stacks_and_nesting_depth_read_containers_and_bundles() {
        let plain = ItemStack::of(ItemKey(0));
        assert!(plain.contained_stacks(&Shelf).is_empty());
        assert_eq!(plain.nesting_depth(&Shelf), 0);
        let slot = |n: i32, item: Compound| {
            let mut entry = Compound::new();
            entry.insert(JavaString::from("slot"), Tag::Int(n));
            entry.insert(JavaString::from("item"), Tag::Compound(item));
            Tag::Compound(entry)
        };
        let bundle = item_tag(
            "minecraft:bundle",
            1,
            Some((
                "minecraft:bundle_contents",
                Tag::List(vec![Tag::Compound(item_tag("minecraft:stone", 7, None))]),
            )),
        );
        let shulker = item_tag(
            "minecraft:shulker_box",
            1,
            Some((
                "minecraft:container",
                Tag::List(vec![
                    slot(0, item_tag("minecraft:stone", 3, None)),
                    slot(1, item_tag("minecraft:unknown", 1, None)),
                    slot(2, item_tag("minecraft:stone", 0, None)),
                    slot(4, bundle),
                ]),
            )),
        );
        let outer = ItemStack::from_nbt(&Shelf, &shulker).unwrap();
        let inside = outer.contained_stacks(&Shelf);
        assert_eq!(inside.len(), 2);
        assert_eq!((inside[0].item(), inside[0].count()), (ItemKey(1), 3));
        assert_eq!((inside[1].item(), inside[1].count()), (ItemKey(3), 1));
        let in_bundle = inside[1].contained_stacks(&Shelf);
        assert_eq!((in_bundle[0].item(), in_bundle[0].count()), (ItemKey(1), 7));
        assert_eq!(inside[0].nesting_depth(&Shelf), 0);
        assert_eq!(inside[1].nesting_depth(&Shelf), 1);
        assert_eq!(outer.nesting_depth(&Shelf), 2);
        // A chain of 100 bundles stops at the 64 levels the walk takes.
        let mut chain = item_tag("minecraft:stone", 1, None);
        for _ in 0..100 {
            chain = item_tag(
                "minecraft:bundle",
                1,
                Some((
                    "minecraft:bundle_contents",
                    Tag::List(vec![Tag::Compound(chain)]),
                )),
            );
        }
        assert_eq!(
            ItemStack::from_nbt(&Shelf, &chain).unwrap().nesting_depth(&Shelf),
            64
        );
    }

    /// [`TextNbt`] over an item whose prototype supplies a name and a lore.
    struct Prototyped;
    impl crate::prelude::TextHost for Prototyped {
        fn text_to_nbt(&self, text: &crate::Text) -> Result<Tag, crate::TextFormError> {
            TextNbt.text_to_nbt(text)
        }
    }
    impl crate::prelude::ItemsHost for Prototyped {
        fn item_key(&self, _: &crate::Identifier) -> Option<ItemKey> {
            None
        }
        fn item_default_component(&self, _: ItemKey, id: &str) -> Option<Tag> {
            matches!(id, "minecraft:custom_name" | "minecraft:lore")
                .then(|| Tag::String("default".into()))
        }
    }
    impl crate::Host for Prototyped {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            None
        }
    }
    crate::impl_host_areas!(Prototyped, except(text, items));

    /// More than 256 lore lines is refused before either component changes, and clearing the name
    /// or the lore of an item whose prototype supplies one records a removal.
    #[test]
    fn set_display_limits_the_lore_and_records_the_removal_of_a_prototype_default() {
        let mut stack = ItemStack::of(ItemKey(0));
        let lines = vec![crate::Text::literal("l"); 257];
        let before = stack.clone();
        assert_eq!(
            stack.set_display(&TextNbt, Some(&crate::Text::literal("Sword")), &lines),
            Err(crate::TextFormError::Malformed)
        );
        assert_eq!(stack, before, "neither component changed");
        stack
            .set_display(&TextNbt, None, &vec![crate::Text::literal("l"); 256])
            .unwrap();
        assert!(stack.components().is_some());
        // no prototype default: clearing leaves no trace; with one, the removal is recorded
        let mut plain = ItemStack::of(ItemKey(0));
        plain.set_display(&TextNbt, None, &[]).unwrap();
        assert!(plain.components().is_none());
        let mut named = ItemStack::of(ItemKey(0));
        named.set_display(&Prototyped, None, &[]).unwrap();
        let patch = named.components().unwrap().as_nbt();
        assert!(patch.contains_key(&JavaString::from("!minecraft:custom_name")));
        assert!(patch.contains_key(&JavaString::from("!minecraft:lore")));
    }

    /// The enchantments read back in id order with their levels, a book's stored ones from their own
    /// component, a negative or non-integer level left out, and a stack with none reads empty.
    #[test]
    fn enchantments_read_the_stacks_own_levels() {
        let levels = |entries: &[(&str, Tag)]| {
            Tag::Compound(
                entries
                    .iter()
                    .map(|(id, level)| (JavaString::from(*id), level.clone()))
                    .collect(),
            )
        };
        let mut stack = ItemStack::of(ItemKey(0));
        assert!(stack.enchantments().is_empty());
        assert!(stack.stored_enchantments().is_empty());
        stack
            .set_component_value(
                "minecraft:enchantments",
                levels(&[
                    ("minecraft:sharpness", Tag::Int(5)),
                    ("minecraft:mending", Tag::Int(1)),
                    ("minecraft:unbreaking", Tag::Int(-2)),
                    ("minecraft:efficiency", Tag::String("3".into())),
                ]),
            )
            .unwrap();
        assert_eq!(
            stack.enchantments(),
            [
                ("minecraft:mending".to_owned(), 1),
                ("minecraft:sharpness".to_owned(), 5),
            ]
        );
        assert!(stack.stored_enchantments().is_empty());
        stack
            .set_component_value(
                "minecraft:stored_enchantments",
                levels(&[("minecraft:silk_touch", Tag::Int(1))]),
            )
            .unwrap();
        assert_eq!(
            stack.stored_enchantments(),
            [("minecraft:silk_touch".to_owned(), 1)]
        );
        // A removal marker reads empty.
        stack.remove_component_value("minecraft:enchantments", true);
        assert!(stack.enchantments().is_empty());
    }

    /// [`TextNbt`] over items whose prototype sets a tooltip display that hides a component, and a glint
    /// override and a dyed colour.
    struct Dressed;
    impl crate::prelude::TextHost for Dressed {
        fn text_to_nbt(&self, text: &crate::Text) -> Result<Tag, crate::TextFormError> {
            TextNbt.text_to_nbt(text)
        }
    }
    impl crate::prelude::ItemsHost for Dressed {
        fn item_key(&self, _: &crate::Identifier) -> Option<ItemKey> {
            None
        }
        fn item_default_component(&self, _: ItemKey, id: &str) -> Option<Tag> {
            match id {
                "minecraft:tooltip_display" => Some(Tag::compound([(
                    "hidden_components",
                    Tag::List(vec![Tag::String("minecraft:dyed_color".into())]),
                )])),
                "minecraft:enchantment_glint_override" => Some(Tag::Byte(1)),
                "minecraft:dyed_color" => Some(Tag::compound([("rgb", Tag::Int(0x0000_00FF))])),
                _ => None,
            }
        }
    }
    impl crate::Host for Dressed {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            None
        }
    }
    crate::impl_host_areas!(Dressed, except(text, items));

    /// The glint override, the tooltip's hidden flag and the leather colour are written, keep the other
    /// keys they share a component with, and clearing one leaves no trace when the item has no prototype
    /// value for it (and records a removal when it has one).
    #[test]
    fn glint_tooltip_and_leather_colour_are_written_and_cleared() {
        let patch = |stack: &ItemStack, id: &str| {
            stack
                .components()
                .and_then(|c| c.as_nbt().get(&JavaString::from(id)).cloned())
        };
        let mut stack = ItemStack::of(ItemKey(0));
        stack.set_glint_override(&TextNbt, Some(true));
        assert_eq!(
            patch(&stack, "minecraft:enchantment_glint_override"),
            Some(Tag::Byte(1))
        );
        stack.set_glint_override(&TextNbt, Some(false));
        assert_eq!(
            patch(&stack, "minecraft:enchantment_glint_override"),
            Some(Tag::Byte(0))
        );
        stack.set_glint_override(&TextNbt, None);
        assert!(stack.components().is_none(), "no prototype: cleared");

        stack.set_tooltip_hidden(&TextNbt, true);
        assert_eq!(
            patch(&stack, "minecraft:tooltip_display"),
            Some(Tag::compound([("hide_tooltip", Tag::Byte(1))]))
        );
        stack.set_tooltip_hidden(&TextNbt, false);
        assert!(stack.components().is_none(), "an empty display is removed");

        stack.set_leather_color(&TextNbt, Some(0xFF_AA_00_12));
        assert_eq!(
            patch(&stack, "minecraft:dyed_color"),
            Some(Tag::compound([("rgb", Tag::Int(0x00AA_0012))]))
        );
        stack.set_leather_color(&TextNbt, None);
        assert!(stack.components().is_none(), "no prototype: cleared");
    }

    /// Over a prototype's display, hiding keeps its `hidden_components`, showing again keeps them too,
    /// and clearing the glint or the colour records the removal of the prototype's own value.
    #[test]
    fn prototype_values_are_kept_and_their_removals_recorded() {
        let mut stack = ItemStack::of(ItemKey(0));
        stack.set_tooltip_hidden(&Dressed, true);
        assert_eq!(
            stack
                .components()
                .and_then(|c| c.as_nbt().get(&JavaString::from("minecraft:tooltip_display")).cloned()),
            Some(Tag::compound([
                (
                    "hidden_components",
                    Tag::List(vec![Tag::String("minecraft:dyed_color".into())]),
                ),
                ("hide_tooltip", Tag::Byte(1)),
            ]))
        );
        stack.set_tooltip_hidden(&Dressed, false);
        let patch = stack.components().unwrap().as_nbt();
        assert_eq!(
            patch.get(&JavaString::from("minecraft:tooltip_display")),
            Some(&Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:dyed_color".into())]),
            )]))
        );
        let mut plain = ItemStack::of(ItemKey(0));
        plain.set_glint_override(&Dressed, None);
        plain.set_leather_color(&Dressed, None);
        let patch = plain.components().unwrap().as_nbt();
        assert!(patch.contains_key(&JavaString::from("!minecraft:enchantment_glint_override")));
        assert!(patch.contains_key(&JavaString::from("!minecraft:dyed_color")));
    }

    struct Number;
    impl Component for Number {
        type Value = i32;
        fn id(&self) -> &str {
            "test:number"
        }
        fn decode(&self, value: &Tag) -> Option<i32> {
            match value {
                Tag::Int(value) => Some(*value),
                _ => None,
            }
        }
        fn encode(&self, value: i32) -> Tag {
            Tag::Int(value)
        }
    }

    #[test]
    fn count_validation_is_atomic_and_registry_independent() {
        let key = ItemKey(0);
        assert!(ItemStack::new(key, 0).is_none());
        let mut stack = ItemStack::new(key, u32::MAX).unwrap();
        assert_eq!(stack.grow(1), Err(StackError::CountOverflow));
        assert_eq!(stack.set_count(0), Err(StackError::Empty));
        assert_eq!(stack.count(), u32::MAX);
        stack.set_damage(u32::MAX);
        assert_eq!(stack.damage(), u32::MAX);
    }

    #[test]
    fn splits_conserve_items_without_an_empty_sentinel() {
        for n in [0, 1, 4, 5, u32::MAX] {
            let mut stack = ItemStack::new(ItemKey(0), 4).unwrap();
            stack.set(&Number, 17).unwrap();
            let split = stack.split(n);
            assert_eq!(split.taken.as_ref().map_or(0, ItemStack::count), n.min(4));
            assert_eq!(
                split.remainder.as_ref().map_or(0, ItemStack::count),
                4 - n.min(4)
            );
            for portion in [split.taken, split.remainder].into_iter().flatten() {
                assert_eq!(portion.get(&Number), Some(17));
            }
        }
        assert!(ItemStack::of(ItemKey(0)).shrink(1).is_none());
    }

    #[test]
    fn patches_are_lazy_and_comparisons_ignore_counts() {
        let mut stack = ItemStack::of(ItemKey(0));
        assert!(stack.components().is_none());
        stack.set(&Number, 19).unwrap();
        let mut other = stack.clone();
        other.grow(3).unwrap();
        assert!(stack.is_stackable_with(&other));
        other.remove(&Number);
        assert!(!stack.same_components(&other));
        assert!(other.components().is_none());
        stack.remove(&Number);
        assert!(stack.is_stackable_with(&other));
    }
}
