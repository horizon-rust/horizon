// RT lighting's moving casters (gfx/hw_rays.rs `MOVING_MASK`): the entities, the player's body and
// the view model sit in the top-level structure with a mask bit the terrain's sections lack, so a
// ray masked to it meets them alone. `trace` sends one toward the sun beside the sun's own ray
// where the sun pass gave the sun's light, or with `SKIP` 8388608 (otherwise the sun's own ray
// says what stopped it, `hw_light_moving`):
// where it meets one, the shadow there moves with its caster, and `accumulate` keeps the sun's
// history short (a few frames) instead of the 30 a still shadow averages, so the shadow of a block
// in the hand, of the player or of a mob follows it sharp rather than trailing behind.
// Appended to shadow_rays.wgsl with the hybrid's part (shadow_rays_hw.wgsl).
const MOVING_MASK: u32 = 0x08u;
fn hw_moving_shadow(pos: vec3<f32>, l: vec3<f32>, reach: f32) -> bool {
    var q: ray_query;
    hw_from = pos;
    rayQueryInitialize(&q, terrain, RayDesc(0x0u, MOVING_MASK, 0.0, reach, pos + rt.hw_offset.xyz, l));
    while rayQueryProceed(&q) {
        let c = rayQueryGetCandidateIntersection(&q);
        if hw_opaque(c) {
            rayQueryConfirmIntersection(&q);
            rayQueryTerminate(&q);
        }
    }
    return rayQueryGetCommittedIntersection(&q).kind != RAY_QUERY_INTERSECTION_NONE;
}
