// The look's tone curve, which fits the world's light to the display: as written where the
// display reaches the scene's peak, eased into the display's peak below it. In SDR nothing was
// lifted, and Horizon clips at white.
fn tonemap(input: TonemapInput) -> vec3<f32> {
    if frame.hdr == 0u {
        return input.color;
    }
    let top = max(input.color.r, max(input.color.g, input.color.b));
    return input.color * hdr_tone_scale(top, HDR_SCENE_PEAK, frame.headroom, frame.paper_white);
}
