//! The controllers through OpenXR actions: one action set with sticks, triggers, grips and
//! face buttons, bound for the common interaction profiles. The runtime maps whatever the
//! player holds (Touch, Index, hand tracking turned into pinches) onto them.

use openxr as xr;

use crate::{Error, err};

/// The controllers' state at the last `Xr::controls`, laid out like a gamepad: sticks are
/// `[x, y]` with right and up positive, triggers 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub left_trigger: f32,
    pub right_trigger: f32,
    pub left_grip: bool,
    pub right_grip: bool,
    pub left_stick_click: bool,
    pub right_stick_click: bool,
    /// Right hand's lower and upper face buttons (A and B on Touch).
    pub a: bool,
    pub b: bool,
    /// Left hand's lower and upper face buttons (X and Y on Touch).
    pub x: bool,
    pub y: bool,
    pub menu: bool,
}

pub(crate) struct Actions {
    set: xr::ActionSet,
    left_stick: xr::Action<xr::Vector2f>,
    right_stick: xr::Action<xr::Vector2f>,
    left_trigger: xr::Action<f32>,
    right_trigger: xr::Action<f32>,
    buttons: [xr::Action<bool>; 9],
}

/// `Actions::buttons`, in this order.
const BUTTONS: [(&str, &str); 9] = [
    ("left_grip", "Left grip"),
    ("right_grip", "Right grip"),
    ("left_stick_click", "Left stick click"),
    ("right_stick_click", "Right stick click"),
    ("a", "A"),
    ("b", "B"),
    ("x", "X"),
    ("y", "Y"),
    ("menu", "Menu"),
];

impl Actions {
    pub(crate) fn new(
        instance: &xr::Instance,
        session: &xr::Session<xr::Vulkan>,
    ) -> Result<Self, Error> {
        let set = instance
            .create_action_set("gameplay", "Gameplay", 0)
            .map_err(err("action set"))?;
        let vector = |name: &str, label: &str| set.create_action::<xr::Vector2f>(name, label, &[]);
        let float = |name: &str, label: &str| set.create_action::<f32>(name, label, &[]);
        let left_stick = vector("left_stick", "Move").map_err(err("action"))?;
        let right_stick = vector("right_stick", "Turn").map_err(err("action"))?;
        let left_trigger = float("left_trigger", "Use").map_err(err("action"))?;
        let right_trigger = float("right_trigger", "Attack").map_err(err("action"))?;
        let mut buttons = Vec::new();
        for (name, label) in BUTTONS {
            buttons.push(
                set.create_action::<bool>(name, label, &[])
                    .map_err(err("action"))?,
            );
        }
        let buttons: [xr::Action<bool>; 9] =
            buttons.try_into().map_err(|_| Error("actions".into()))?;
        let actions = Self {
            set,
            left_stick,
            right_stick,
            left_trigger,
            right_trigger,
            buttons,
        };
        actions.suggest(instance);
        session
            .attach_action_sets(&[&actions.set])
            .map_err(err("xrAttachSessionActionSets"))?;
        Ok(actions)
    }

    /// Bindings for each profile; a profile the runtime doesn't know is skipped.
    fn suggest(&self, instance: &xr::Instance) {
        let [l_grip, r_grip, l_click, r_click, a, b, x, y, menu] = &self.buttons;
        let touch: Vec<(&str, Binding)> = vec![
            (
                "/user/hand/left/input/thumbstick",
                Binding::Vector(&self.left_stick),
            ),
            (
                "/user/hand/right/input/thumbstick",
                Binding::Vector(&self.right_stick),
            ),
            (
                "/user/hand/left/input/trigger/value",
                Binding::Float(&self.left_trigger),
            ),
            (
                "/user/hand/right/input/trigger/value",
                Binding::Float(&self.right_trigger),
            ),
            ("/user/hand/left/input/squeeze/value", Binding::Bool(l_grip)),
            (
                "/user/hand/right/input/squeeze/value",
                Binding::Bool(r_grip),
            ),
            (
                "/user/hand/left/input/thumbstick/click",
                Binding::Bool(l_click),
            ),
            (
                "/user/hand/right/input/thumbstick/click",
                Binding::Bool(r_click),
            ),
            ("/user/hand/right/input/a/click", Binding::Bool(a)),
            ("/user/hand/right/input/b/click", Binding::Bool(b)),
            ("/user/hand/left/input/x/click", Binding::Bool(x)),
            ("/user/hand/left/input/y/click", Binding::Bool(y)),
            ("/user/hand/left/input/menu/click", Binding::Bool(menu)),
        ];
        let index: Vec<(&str, Binding)> = vec![
            (
                "/user/hand/left/input/thumbstick",
                Binding::Vector(&self.left_stick),
            ),
            (
                "/user/hand/right/input/thumbstick",
                Binding::Vector(&self.right_stick),
            ),
            (
                "/user/hand/left/input/trigger/value",
                Binding::Float(&self.left_trigger),
            ),
            (
                "/user/hand/right/input/trigger/value",
                Binding::Float(&self.right_trigger),
            ),
            ("/user/hand/left/input/squeeze/value", Binding::Bool(l_grip)),
            (
                "/user/hand/right/input/squeeze/value",
                Binding::Bool(r_grip),
            ),
            (
                "/user/hand/left/input/thumbstick/click",
                Binding::Bool(l_click),
            ),
            (
                "/user/hand/right/input/thumbstick/click",
                Binding::Bool(r_click),
            ),
            ("/user/hand/right/input/a/click", Binding::Bool(a)),
            ("/user/hand/right/input/b/click", Binding::Bool(b)),
            ("/user/hand/left/input/a/click", Binding::Bool(x)),
            ("/user/hand/left/input/b/click", Binding::Bool(y)),
        ];
        let simple: Vec<(&str, Binding)> = vec![
            (
                "/user/hand/right/input/select/click",
                Binding::Float(&self.right_trigger),
            ),
            (
                "/user/hand/left/input/select/click",
                Binding::Float(&self.left_trigger),
            ),
            ("/user/hand/left/input/menu/click", Binding::Bool(menu)),
        ];
        for (profile, bindings) in [
            ("/interaction_profiles/oculus/touch_controller", touch),
            ("/interaction_profiles/valve/index_controller", index),
            ("/interaction_profiles/khr/simple_controller", simple),
        ] {
            let Ok(profile_path) = instance.string_to_path(profile) else {
                continue;
            };
            let mut list = Vec::new();
            for (path, binding) in &bindings {
                let Ok(p) = instance.string_to_path(path) else {
                    continue;
                };
                list.push(match binding {
                    Binding::Vector(a) => xr::Binding::new(*a, p),
                    Binding::Float(a) => xr::Binding::new(*a, p),
                    Binding::Bool(a) => xr::Binding::new(*a, p),
                });
            }
            if let Err(e) = instance.suggest_interaction_profile_bindings(profile_path, &list) {
                log::warn!("Horizon VR: no {profile} bindings ({e})");
            }
        }
    }

    pub(crate) fn read(&self, session: &xr::Session<xr::Vulkan>) -> Controls {
        if session
            .sync_actions(&[xr::ActiveActionSet::new(&self.set)])
            .is_err()
        {
            return Controls::default();
        }
        let vector = |a: &xr::Action<xr::Vector2f>| {
            a.state(session, xr::Path::NULL)
                .map(|s| [s.current_state.x, s.current_state.y])
                .unwrap_or_default()
        };
        let float = |a: &xr::Action<f32>| {
            a.state(session, xr::Path::NULL)
                .map(|s| s.current_state)
                .unwrap_or(0.0)
        };
        let button = |i: usize| {
            self.buttons[i]
                .state(session, xr::Path::NULL)
                .map(|s| s.current_state)
                .unwrap_or(false)
        };
        Controls {
            left_stick: vector(&self.left_stick),
            right_stick: vector(&self.right_stick),
            left_trigger: float(&self.left_trigger),
            right_trigger: float(&self.right_trigger),
            left_grip: button(0),
            right_grip: button(1),
            left_stick_click: button(2),
            right_stick_click: button(3),
            a: button(4),
            b: button(5),
            x: button(6),
            y: button(7),
            menu: button(8),
        }
    }
}

enum Binding<'a> {
    Vector(&'a xr::Action<xr::Vector2f>),
    Float(&'a xr::Action<f32>),
    Bool(&'a xr::Action<bool>),
}
