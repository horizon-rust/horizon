//! Aiming with the eyes in VR (Horizon's own; docs/vr.md): with eye tracking, the block the
//! player looks at is the one they target (its outline, mining, using items on it), not the
//! one straight ahead of the head. The gaze is smoothed and held so a coarse one doesn't
//! make the target flicker (`horizon_client::gaze_aim`). Without a valid gaze the head aims,
//! as before. `HORIZON_XR_AIM=head` starts with the head aiming; clicking both sticks at
//! once switches between the two.
use super::Game;
use horizon_client::{
    gamepad::Button,
    gaze_aim::{Aim, GazeAim, Target},
    gfx::XrLook,
    text::Text,
};

/// What aims in VR, between frames.
#[derive(Default)]
pub(super) struct XrGaze {
    /// The head aims even with eye tracking; read from `HORIZON_XR_AIM` on the first frame.
    head: Option<bool>,
    /// Both sticks were clicked at the last frame.
    chord: bool,
    eyes: GazeAim,
    /// What `pick` aims with this frame.
    pub(super) aim: Aim,
}

impl Game<'_> {
    /// Once per frame, after the head turned the player (`xr_look`): where the eyes aim.
    pub(super) fn xr_gaze_frame(&mut self, look: XrLook, seconds: f32) {
        let head = *self
            .pad
            .gaze
            .head
            .get_or_insert_with(|| std::env::var("HORIZON_XR_AIM").is_ok_and(|v| v == "head"));
        let chord =
            self.pad.state.held(Button::LeftStick) && self.pad.state.held(Button::RightStick);
        let head = if chord && !self.pad.gaze.chord {
            let message = match (head, look.eye_tracking) {
                (true, true) => "VR aim: eyes",
                (true, false) => "VR aim: head (no eye tracking)",
                (false, _) => "VR aim: head",
            };
            self.titles.set_overlay(Text::literal(message), false);
            self.pad.gaze.head = Some(!head);
            !head
        } else {
            head
        };
        self.pad.gaze.chord = chord;
        let gaze = look
            .gaze
            .filter(|_| !head)
            .map(|[turn, pitch]| super::view_vector(self.yaw + turn, pitch));
        let Some(eye) = self.eye_position() else {
            self.pad.gaze.aim = Aim::Head;
            return;
        };
        let reach = self.reach();
        let mut eyes = std::mem::take(&mut self.pad.gaze.eyes);
        self.pad.gaze.aim = eyes.update(seconds, eye, gaze, |dir| {
            self.clip_ray(eye, dir, reach).map(|hit| Target {
                pos: hit.pos,
                point: hit.point,
            })
        });
        self.pad.gaze.eyes = eyes;
    }
}
