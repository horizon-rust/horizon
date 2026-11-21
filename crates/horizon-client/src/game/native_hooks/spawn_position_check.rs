use super::*;

impl Game<'_> {
    /// Cancel a natural spawn at a position for the player it is for
    pub(in crate::game) fn native_spawn_position_check(
        &self,
        call: mod_api::__private::SpawnPositionCheckCall<'_>,
    ) {
        self.fire_spawn_position_check(call);
    }
}
