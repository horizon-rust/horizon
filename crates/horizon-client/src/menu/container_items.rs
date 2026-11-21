//! `Item.canFitInsideContainerItems`: whether a stack may go inside an item that holds
//! items, a shulker box or a bundle. One rule and one entry point, which asks the parts of
//! the stack's item (`ItemPart::can_fit_inside_container_items`) after vanilla's answer.
use super::ItemStack;
use horizon_world::native_load_context::LoopContext;

/// Vanilla's `Item.canFitInsideContainerItems` for the block the stack's item places
/// (`ItemInfo.block`): a `BlockItem` answers `!(getBlock() instanceof ShulkerBoxBlock)`, so
/// the block decides and the item's id never does. `None` is an item that places no block,
/// which is no `BlockItem` and always fits. A `BlockItem` whose item id merely ends in a
/// shulker box's name fits, and a content item that places a shulker box's template does
/// not.
pub fn fits_inside_container_items_of_block(
    facts: &crate::blocks::BlockFacts,
    block: Option<horizon_world::registry::BlockStateId>,
) -> bool {
    block.is_none_or(|state| !facts.get(state).has_class("ShulkerBoxBlock"))
}

/// [`fits_inside_container_items_of_block`] for a caller that has the item table: the block
/// is the one the stack's item places. This is the exact entrance; the one below answers
/// only from the item's id.
pub fn vanilla_fits_inside_container_items_for_item(
    items: &crate::items::Items,
    facts: &crate::blocks::BlockFacts,
    stack: &ItemStack,
) -> bool {
    fits_inside_container_items_of_block(facts, items.get(&stack.item).and_then(|info| info.block))
}

/// Vanilla's `Item.canFitInsideContainerItems` where the caller has no item table and so
/// cannot ask the block the stack's item places: an item id carries only its block's name,
/// so the answer here is the block entity type that name holds —
/// `BlockEntityType.validBlocks` accepts `minecraft:shulker_box` for vanilla's 17 shulker
/// boxes and nothing else, and a content block answers with its template's. A mod item whose
/// block differs from its id, or a content block whose declared type the table does not
/// carry, is where this and vanilla can disagree; a caller with the item table
/// ([`vanilla_fits_inside_container_items_for_item`]) is exact, and the item's own part
/// (`can_fit_inside_container_items`) is where a mod says so itself.
pub fn vanilla_fits_inside_container_items(stack: &ItemStack) -> bool {
    crate::containers::block_entity_type(&stack.item) != Some("minecraft:shulker_box")
}

/// `ItemStack.canFitInsideContainerItems` (`ShulkerBoxSlot.mayPlace`): vanilla's answer,
/// then the parts of the stack's item. An item with no such part keeps vanilla's answer at
/// the cost of one table lookup. `context` is the loop the menu or the insert runs in.
pub fn fits_inside_container_items(context: &LoopContext, stack: &ItemStack) -> bool {
    fits_inside_container_items_from(context, stack, vanilla_fits_inside_container_items(stack))
}

/// [`fits_inside_container_items`] where the caller already knows vanilla's answer: a
/// caller with the item table asks the block the stack's item places
/// ([`vanilla_fits_inside_container_items_for_item`]) and the parts of the stack's item may
/// still change it.
pub fn fits_inside_container_items_from(
    context: &LoopContext,
    stack: &ItemStack,
    vanilla: bool,
) -> bool {
    let mut fits = vanilla;
    crate::native_client_hooks::can_fit_inside_container_items(context, stack, &mut fits);
    fits
}

/// `BundleContents.canItemBeInBundle`: a stack that isn't empty and fits inside container
/// items ([`fits_inside_container_items`]).
pub fn can_be_in_bundle(context: &LoopContext, stack: &ItemStack) -> bool {
    stack.count > 0 && fits_inside_container_items(context, stack)
}

/// Whether `stack` may go inside an item that holds items
/// (`IItemExtension.canFitInsideContainerItems`) is being decided: `fits` starts with
/// vanilla's answer and the parts of the stack's item may change it. Asked by a shulker
/// box's slots, a bundle and a mod's item slots, on both sides.
#[inline(never)]
pub fn native_can_fit_inside_container_items(
    context: &LoopContext,
    stack: &mod_api::ItemStack,
    fits: &mut bool,
) {
    mod_api::dispatch_can_fit_inside_container_items(
        &horizon_client_render::api_context::LoopReader(context).context(),
        stack,
        fits,
    );
}

#[cfg(test)]
#[path = "container_items_tests.rs"]
mod tests;
