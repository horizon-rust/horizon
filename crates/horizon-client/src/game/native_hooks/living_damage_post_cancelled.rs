use super::*;

impl HitLevel<'_> {
    /// `LivingDamageEvent.Post` for a hit the incoming check canceled: entity `entity` of type
    /// `kind` was not hurt by the hit `container` describes. It only listens, and fires once per
    /// canceled hit, at the cancel. It can't write the entity or the world's blocks: the level
    /// host gives the hit's random and level attachments.
    pub(in crate::game) fn native_living_damage_post_cancelled(
        &mut self,
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        container: &mod_api::DamageContainer,
    ) {
        self.fire_living_damage_post_cancelled(entity, kind, source, container);
    }
}
