//! Which blocks a hand or a wind charge's burst flips (#18502): vanilla's
//! `BlockBehaviour.onExplosionHit` (`TRIGGER_BLOCK`) and the doors' and
//! trapdoors' own hand `use`. Both ask the block's class, never its id, and a
//! door or trapdoor asks its `BlockSetType`'s flags.
use super::classes::Class;
use super::data::BlockData;

/// The classes whose own `use`/`onExplosionHit` flips them. A door and a
/// trapdoor keep the two `BlockSetType` flags vanilla asks of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flippable {
    Door { by_hand: bool, by_wind_charge: bool },
    FenceGate,
    Button,
    Lever,
    Bell,
    Cauldron,
}

/// [`Flippable`] of a block, or None when neither a hand nor a burst acts on it.
fn flippable(data: &BlockData) -> Option<Flippable> {
    if data.is(Class::Door) || data.is(Class::TrapDoor) {
        let set = data.set_type.as_ref();
        return Some(Flippable::Door {
            by_hand: set.is_some_and(|t| t.can_open_by_hand),
            by_wind_charge: set.is_some_and(|t| t.can_open_by_wind_charge),
        });
    }
    if data.is(Class::FenceGate) {
        return Some(Flippable::FenceGate);
    }
    if data.is(Class::Button) {
        return Some(Flippable::Button);
    }
    if data.is(Class::Lever) {
        return Some(Flippable::Lever);
    }
    if data.is(Class::Bell) {
        return Some(Flippable::Bell);
    }
    if data.is(Class::AbstractCauldron) {
        return Some(Flippable::Cauldron);
    }
    None
}

impl Flippable {
    /// `BlockBehaviour.onExplosionHit` for a `TRIGGER_BLOCK` blast: a door or
    /// trapdoor asks `BlockSetType.canOpenByWindCharge` (an iron one answers
    /// false, a copper one true), a fence gate, button, lever or bell always
    /// reacts, and a cauldron is no burst target.
    fn by_wind_charge(self) -> bool {
        match self {
            Self::Door { by_wind_charge, .. } => by_wind_charge,
            Self::FenceGate | Self::Button | Self::Lever | Self::Bell => true,
            Self::Cauldron => false,
        }
    }
    /// Whether a player's hand flips it as it uses the block, which a guest's
    /// client predicts at once (its own click, before the server's changes
    /// arrive): a door or trapdoor asks `BlockSetType.canOpenByHand` (an iron
    /// one has no hand use), a fence gate, lever, button or cauldron reacts,
    /// and a bell rings without flipping.
    fn by_hand(self) -> bool {
        match self {
            Self::Door { by_hand, .. } => by_hand,
            Self::FenceGate | Self::Button | Self::Lever | Self::Cauldron => true,
            Self::Bell => false,
        }
    }
}

/// Whether a block reacts to a wind charge's burst by being used.
pub fn flips_by_wind_charge(data: &BlockData) -> bool {
    flippable(data).is_some_and(Flippable::by_wind_charge)
}

/// Whether a player's hand flips the block as it uses it: the same classes
/// [`flips_by_wind_charge`] walks, asking the hand's flag instead of the burst's.
pub fn flips_by_hand(data: &BlockData) -> bool {
    flippable(data).is_some_and(Flippable::by_hand)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two answers of a block's own `use` and `onExplosionHit`, which need
    /// no resources: a hand flips a cauldron and a door whose set type allows
    /// hands, a burst opens a wooden or copper door and leaves a cauldron
    /// alone, and an iron door is no target of either.
    #[test]
    fn a_hand_and_a_burst_flip_different_blocks() {
        let wooden = Flippable::Door {
            by_hand: true,
            by_wind_charge: true,
        };
        let iron = Flippable::Door {
            by_hand: false,
            by_wind_charge: false,
        };
        assert!(wooden.by_hand() && wooden.by_wind_charge());
        assert!(!iron.by_hand() && !iron.by_wind_charge());
        for block in [Flippable::FenceGate, Flippable::Button, Flippable::Lever] {
            assert!(block.by_hand() && block.by_wind_charge(), "{block:?}");
        }
        // A cauldron fills from a use and never from a burst; a bell rings, so
        // it is no hand flip.
        assert!(Flippable::Cauldron.by_hand() && !Flippable::Cauldron.by_wind_charge());
        assert!(Flippable::Bell.by_wind_charge() && !Flippable::Bell.by_hand());
    }
}
