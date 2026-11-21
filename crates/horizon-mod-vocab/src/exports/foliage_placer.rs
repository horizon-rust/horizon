pub use world::foliage_placer::{
    FoliageAttachment, FoliageContext, FoliagePlacer, FoliagePlacerType, place_leaves_row,
};
mod __private {
    /// The foliage placer types the mods registered, for the executable to hand to the engine's
    /// tree features before any world opens.
    pub use crate::world::foliage_placer::foliage_placer_type_declarations;
}
