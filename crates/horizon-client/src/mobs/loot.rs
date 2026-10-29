//! Items on the ground as a mob sees them and what it does with them: the
//! item entities within reach of a goal's `getEntitiesOfClass(ItemEntity.class,
//! ..)`, `Mob.wantsToPickUp` and `canHoldItem` with the overrides of the
//! mobs that pick up loot, and `Mob.equipItemIfPossible`, which `Mob.pickUpItem`
//! uses to put a picked up item in the slot it belongs to.
use super::{
    Mob, MobWorld, Species,
    doors::is_zombie,
    drops,
    finalize::{self, Attr},
    gear, raider,
};
use crate::{
    entities::ItemEntity,
    items::{ItemStack, Items},
};
use horizon_core::{geometry::Aabb, random::RandomSource};

impl MobWorld<'_> {
    /// The item entities whose box meets `area`, in the order the level lists
    /// them: what `getEntitiesOfClass(ItemEntity.class, area)` finds.
    pub fn items_in(&self, area: Aabb) -> impl Iterator<Item = &ItemEntity> {
        self.dropped
            .iter()
            .filter(move |e| e.stack.count > 0 && area.intersects(e.bounds()))
    }
    /// Whether a dropped item's box touches water (its `isInWater`).
    pub fn item_in_water(&self, item: &ItemEntity) -> bool {
        self.fluids(item.bounds(), item.position.y).in_water
    }
}

/// Whether the item is in an item tag.
pub fn in_tag(items: &Items, tag: &str, item: &str) -> bool {
    items.item_tags.get(tag).is_some_and(|t| t.contains(item))
}

/// `Mob.canHoldItem`: always true, but a zombie that is a baby passenger
/// does not hold eggs and a wither skeleton not the weapons it dislikes.
pub fn can_hold_item(m: &Mob, items: &Items, item: &str) -> bool {
    match m.species() {
        Species::WitherSkeleton => {
            !in_tag(items, "minecraft:wither_skeleton_disliked_weapons", item)
        }
        s if is_zombie(s) => {
            !(in_tag(items, "minecraft:eggs", item) && m.is_baby() && m.extra.mount.is_some())
        }
        _ => true,
    }
}

/// `Mob.wantsToPickUp`: what a mob that can pick up loot goes for, which is
/// what it can hold, except glow ink sacs for zombies and spears for
/// skeletons and drowned (a zombified piglin takes both).
pub fn wants_to_pick_up(m: &Mob, items: &Items, item: &str) -> bool {
    match m.species() {
        Species::ZombifiedPiglin => can_hold_item(m, items, item),
        Species::Drowned
        | Species::Skeleton
        | Species::Stray
        | Species::Bogged
        | Species::Parched
        | Species::WitherSkeleton
            if in_tag(items, "minecraft:spears", item) =>
        {
            false
        }
        Species::Zombie | Species::Husk | Species::ZombieVillager | Species::Drowned
            if item == "minecraft:glow_ink_sac" =>
        {
            false
        }
        _ => can_hold_item(m, items, item),
    }
}

/// `Mob.getPreferredWeaponType`: the item tag of the weapons a mob favours.
fn preferred_weapon_type(m: &Mob) -> Option<&'static str> {
    match m.species() {
        Species::Pillager => Some("minecraft:pillager_preferred_weapons"),
        Species::Skeleton | Species::Stray | Species::Bogged | Species::Parched => {
            Some("minecraft:skeleton_preferred_weapons")
        }
        Species::Drowned => Some("minecraft:drowned_preferred_weapons"),
        _ => None,
    }
}

/// Whether an `EquipmentSlotGroup` of an attribute modifier covers a slot.
fn slot_in_group(slot: &str, group: &str) -> bool {
    match group {
        "any" => true,
        "hand" => slot == "mainhand" || slot == "offhand",
        "armor" => matches!(slot, "feet" | "legs" | "chest" | "head" | "body"),
        _ => slot == group,
    }
}

/// `Mob.getApproximateAttributeWith`: the attribute's base value with the
/// modifiers of the item for the slot (`ItemAttributeModifiers.compute`).
fn approximate_attribute(
    m: &Mob,
    items: &Items,
    stack: &ItemStack,
    attribute: &str,
    slot: &str,
) -> f64 {
    let base = match attribute {
        "minecraft:attack_damage" => finalize::base_value(m, Attr::AttackDamage),
        "minecraft:armor" => finalize::base_value(m, Attr::Armor),
        other => m.kind.attributes.get(other).copied().unwrap_or(0.0),
    };
    let mut value = base;
    if let Some(info) = items.get(&stack.item) {
        for (attr, _, amount, operation, group) in &info.modifiers {
            if attr == attribute && slot_in_group(slot, group) {
                value += match operation.as_str() {
                    "add_multiplied_base" => amount * base,
                    "add_multiplied_total" => amount * value,
                    _ => *amount,
                };
            }
        }
    }
    value
}

/// `Mob.canReplaceEqualItem`: more enchantments, then less damage, then a
/// custom name.
fn can_replace_equal_item(new: &ItemStack, current: &ItemStack) -> bool {
    let (new_enchantments, current_enchantments) =
        (new.enchantments().len(), current.enchantments().len());
    if new_enchantments != current_enchantments {
        return new_enchantments > current_enchantments;
    }
    if new.damage != current.damage {
        return new.damage < current.damage;
    }
    new.custom_name().is_some() && current.custom_name().is_none()
}

/// `Mob.canReplaceCurrentItem`: an empty slot is free; armour is compared by
/// defense, toughness, then `canReplaceEqualItem` (`compareArmor`); the
/// main hand by the preferred weapons, then attack damage (`compareWeapons`).
/// A drowned keeps its nautilus shell.
fn can_replace_current_item(
    m: &Mob,
    items: &Items,
    new: &ItemStack,
    current: Option<&ItemStack>,
    slot: &str,
) -> bool {
    let Some(current) = current else {
        return true;
    };
    if m.species() == Species::Drowned && &*current.item == "minecraft:nautilus_shell" {
        return false;
    }
    let attribute =
        |stack: &ItemStack, name: &str| approximate_attribute(m, items, stack, name, slot);
    match slot {
        "head" | "chest" | "legs" | "feet" | "body" => {
            // `PREVENT_ARMOR_CHANGE` is the binding curse's effect.
            if current.enchantment_level("minecraft:binding_curse") > 0 {
                return false;
            }
            let (new_defense, old_defense) = (
                attribute(new, "minecraft:armor"),
                attribute(current, "minecraft:armor"),
            );
            if new_defense != old_defense {
                return new_defense > old_defense;
            }
            let (new_toughness, old_toughness) = (
                attribute(new, "minecraft:armor_toughness"),
                attribute(current, "minecraft:armor_toughness"),
            );
            if new_toughness != old_toughness {
                new_toughness > old_toughness
            } else {
                can_replace_equal_item(new, current)
            }
        }
        "mainhand" => {
            if let Some(tag) = preferred_weapon_type(m) {
                let current_preferred = in_tag(items, tag, &current.item);
                let new_preferred = in_tag(items, tag, &new.item);
                if current_preferred && !new_preferred {
                    return false;
                }
                if !current_preferred && new_preferred {
                    return true;
                }
            }
            let (new_damage, old_damage) = (
                attribute(new, "minecraft:attack_damage"),
                attribute(current, "minecraft:attack_damage"),
            );
            if new_damage != old_damage {
                new_damage > old_damage
            } else {
                can_replace_equal_item(new, current)
            }
        }
        _ => false,
    }
}

/// The index of an `EquipmentSlot` name in `gear`.
fn gear_index(slot: &str) -> Option<usize> {
    Some(match slot {
        "mainhand" => gear::MAIN,
        "offhand" => gear::OFF,
        "head" => gear::HEAD,
        "chest" => gear::CHEST,
        "legs" => gear::LEGS,
        "feet" => gear::FEET,
        _ => return None,
    })
}

/// The stack in a slot a mob picks up into (`getItemBySlot`), with its
/// count (the main hand's), enchantments and trim.
pub fn slot_stack(m: &Mob, slot: &str) -> Option<ItemStack> {
    let e = &m.extra;
    let (item, worn) = match slot {
        "mainhand" => (e.main_hand.as_ref()?, None),
        "offhand" => (e.off_hand.as_ref()?, None),
        "head" => (e.head.as_ref()?, e.armor[0].as_ref()),
        "chest" => (e.chest.as_ref()?, e.armor[1].as_ref()),
        "legs" => (e.legs.as_ref()?, e.armor[2].as_ref()),
        "feet" => (e.feet.as_ref()?, e.armor[3].as_ref()),
        _ => return None,
    };
    // A raider's helmet slot holds the ominous banner.
    if slot == "head" && &**item == raider::BANNER && raider::is_raider(m.species()) {
        return Some(raider::ominous_banner());
    }
    let mut stack = match worn {
        Some(stack) if stack.item == *item => stack.clone(),
        _ => ItemStack::new(item, 1),
    };
    stack.count = if slot == "mainhand" {
        m.main_hand_count()
    } else {
        1
    };
    if let (Some(index), Some(g)) = (gear_index(slot), gear::gear(m)) {
        for (id, level) in &g.slots[index] {
            stack.enchant(id, *level);
        }
        if let Some(trim) = &g.trims[index] {
            stack.data_mut().trim = Some(trim.clone());
        }
    }
    Some(stack)
}

/// A stack put into a slot a mob picks up into (`setItemSlot`): the item, its
/// count (the main hand's), enchantments and trim.
pub fn set_slot_stack(m: &mut Mob, slot: &str, stack: &ItemStack) {
    let item = Some(stack.item.clone());
    let worn = match slot {
        "mainhand" => {
            m.extra.main_hand = item;
            m.extra.main_hand_count = stack.count;
            None
        }
        "offhand" => {
            m.extra.off_hand = item;
            None
        }
        "head" => {
            m.extra.head = item;
            Some(0)
        }
        "chest" => {
            m.extra.chest = item;
            Some(1)
        }
        "legs" => {
            m.extra.legs = item;
            Some(2)
        }
        "feet" => {
            m.extra.feet = item;
            Some(3)
        }
        _ => return,
    };
    if let Some(index) = gear_index(slot) {
        gear::set(m, index, stack.enchantments().to_vec());
        gear::set_trim(m, index, stack.data().and_then(|d| d.trim.clone()));
    }
    if let Some(piece) = worn {
        m.extra.armor[piece] = stack.data().is_some().then(|| stack.clone());
    }
}

/// `Mob.setGuaranteedDrop`: the slot's item is always dropped.
pub fn set_guaranteed_drop(m: &mut Mob, slot: &str) {
    if let Some(index) = finalize::SLOT_NAMES.iter().position(|s| *s == slot) {
        finalize::set_drop_chance(m, index, 2.0);
    }
}

/// What `Mob.equipItemIfPossible` put on the mob and what it pushed out.
pub struct Equipped {
    /// The part of the stack that is now in the slot (`EquipmentSlot.limit`).
    pub taken: ItemStack,
    /// What was in the slot and falls out of it (`spawnAtLocation`).
    pub replaced: Option<ItemStack>,
    /// The slot is the one the item's equippable names, so
    /// `LivingEntity.onEquipItem` plays its equip sound.
    pub worn: bool,
}

/// `Mob.equipItemIfPossible`: the slot the stack goes to
/// (`getEquipmentSlotForItem`, the armour that does not beat the worn piece
/// goes to the hand), the check that it can replace what is there and that the
/// mob can hold it, the replaced item that falls with its drop chance, then
/// the equipping with a guaranteed drop. Horizon keeps neither a body nor a
/// saddle slot for the mobs that pick up loot, which `allowed_entities` of
/// those items never allows, so only the hands and the armour are tried.
pub fn equip_item_if_possible(m: &mut Mob, items: &Items, stack: &ItemStack) -> Option<Equipped> {
    let info = items.get(&stack.item);
    // getEquipmentSlotForItem and isEquippableInSlot: `canUseSlot` is true
    // for every slot of these mobs; the item's allowed entities decide.
    let mut slot = match info.and_then(|i| i.equip_slot.as_deref()) {
        Some(slot) => {
            let allowed = info
                .and_then(|i| i.equip_allowed.as_ref())
                .is_none_or(|a| a.contains(&*m.kind.id));
            if !allowed {
                return None;
            }
            slot
        }
        None => "mainhand",
    };
    gear_index(slot)?;
    let armor = matches!(slot, "head" | "chest" | "legs" | "feet");
    let mut current = slot_stack(m, slot);
    let mut can_replace = can_replace_current_item(m, items, stack, current.as_ref(), slot);
    if armor && !can_replace {
        slot = "mainhand";
        current = slot_stack(m, slot);
        can_replace = current.is_none();
    }
    if !(can_replace && can_hold_item(m, items, &stack.item)) {
        return None;
    }
    let chance = drops::drop_chance(m, slot);
    let mut replaced = None;
    if let Some(old) = current
        && f64::from((m.random.next_float() - 0.1).max(0.0)) < f64::from(chance)
    {
        replaced = Some(old);
    }
    // EquipmentSlot.limit: one of an armour piece, the whole stack in a hand.
    let mut taken = stack.clone();
    if armor {
        taken.count = taken.count.min(1);
    }
    let worn = info.and_then(|i| i.equip_slot.as_deref()) == Some(slot);
    set_slot_stack(m, slot, &taken);
    set_guaranteed_drop(m, slot);
    m.persistent = true;
    Some(Equipped {
        taken,
        replaced,
        worn,
    })
}
