//! libnx's HID: player 1 plus the handheld controls (libnx merges them), and the touch screen.
//! The `PadState` layout and the pad calls are the ones bevy-horizon's `input.rs` runs on hardware;
//! the touch layout follows switch/services/hid.h (checked by the size assertions below).
use crate::{Frame, Pointer, Raw, Touch, TouchTracker, Tracker, style};

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct Stick {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct PadState {
    id_mask: u8,
    active_id_mask: u8,
    read_handheld: bool,
    active_handheld: bool,
    style_set: u32,
    attributes: u32,
    _pad: u32,
    buttons_cur: u64,
    buttons_old: u64,
    sticks: [Stick; 2],
    gc_triggers: [u32; 2],
}

#[repr(C)]
#[derive(Default, Copy, Clone)]
struct TouchState {
    delta_time: u64,
    attributes: u32,
    finger_id: u32,
    x: u32,
    y: u32,
    diameter_x: u32,
    diameter_y: u32,
    rotation_angle: u32,
    _reserved: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct TouchScreenState {
    sampling_number: u64,
    count: i32,
    _reserved: u32,
    touches: [TouchState; 16],
}

const _: () = {
    assert!(size_of::<PadState>() == 56);
    assert!(size_of::<TouchState>() == 40);
    assert!(size_of::<TouchScreenState>() == 656);
};

unsafe extern "C" {
    fn padConfigureInput(max_players: u32, style_set: u32);
    fn padInitializeWithMask(state: *mut PadState, pad_mask: u64);
    fn padUpdate(state: *mut PadState);
    fn hidInitializeTouchScreen();
    fn hidGetTouchScreenStates(states: *mut TouchScreenState, count: usize) -> usize;
    fn appletMainLoop() -> bool;
    fn appletGetOperationMode() -> u32;
    fn appletGetFocusState() -> u32;
}

/// The applet side of libnx: HOME, sleep, docking.
pub struct Libnx;

impl Libnx {
    /// libnx's `appletMainLoop`: handles applet messages; false once the system asks the program
    /// to exit (HOME menu "close", or hbmenu returning).
    pub fn main_loop() -> bool {
        // SAFETY: plain libnx call, valid after startup.
        unsafe { appletMainLoop() }
    }

    /// Docked (1920x1080) rather than handheld (1280x720).
    pub fn docked() -> bool {
        // AppletOperationMode_Handheld = 0, _Console = 1.
        // SAFETY: plain libnx call.
        unsafe { appletGetOperationMode() == 1 }
    }

    /// The program is in the foreground (AppletFocusState_InFocus = 1).
    pub fn in_focus() -> bool {
        // SAFETY: plain libnx call.
        unsafe { appletGetFocusState() == 1 }
    }
}

/// The touch screen alone, for the window system; `Hid::touch` is the same thing for the game.
pub struct TouchScreen {
    touch: TouchTracker,
}

impl TouchScreen {
    pub fn new() -> Self {
        // SAFETY: libnx call, safe to repeat.
        unsafe { hidInitializeTouchScreen() };
        Self {
            touch: TouchTracker::default(),
        }
    }

    pub fn poll(&mut self) -> Pointer {
        read_touch(&mut self.touch)
    }
}

impl Default for TouchScreen {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads the controllers and the touch screen once per frame, on the thread that owns the game.
pub struct Hid {
    pad: PadState,
    pads: Tracker,
    touch: TouchTracker,
}

impl Hid {
    pub fn new() -> Self {
        // SAFETY: libnx's documented start-up order for the pad API; called once, from the main
        // thread, before the first padUpdate.
        let mut pad = PadState::default();
        unsafe {
            padConfigureInput(1, style::STANDARD);
            // Player 1 (bit 0) and the handheld controls (bit 32).
            padInitializeWithMask(&mut pad, 1 | (1 << 32));
            hidInitializeTouchScreen();
        }
        Self {
            pad,
            pads: Tracker::default(),
            touch: TouchTracker::default(),
        }
    }

    /// The pad this frame. `deadzone` is the stick's rest-drift cut (0.1 suits the Switch's ~0.08).
    pub fn pad(&mut self, deadzone: f32) -> Frame {
        // SAFETY: `self.pad` is a valid, initialised PadState.
        unsafe { padUpdate(&mut self.pad) };
        let p = &self.pad;
        let raw = Raw {
            buttons: p.buttons_cur,
            sticks: [
                [p.sticks[0].x, p.sticks[0].y],
                [p.sticks[1].x, p.sticks[1].y],
            ],
            style_set: p.style_set,
        };
        self.pads.frame(raw, deadzone)
    }

    /// The controller styles after the last `pad` call, if they changed on it (to log).
    pub fn style_changed(&self) -> Option<u32> {
        self.pads.changed()
    }

    pub fn touch(&mut self) -> Pointer {
        read_touch(&mut self.touch)
    }
}

fn read_touch(tracker: &mut TouchTracker) -> Pointer {
    // SAFETY: an all-zero TouchScreenState is valid, and libnx fills at most `count` entries.
    let mut s: TouchScreenState = unsafe { core::mem::zeroed() };
    let n = unsafe { hidGetTouchScreenStates(&mut s, 1) };
    let touches: Vec<Touch> = if n == 0 {
        Vec::new()
    } else {
        s.touches[..(s.count.clamp(0, 16)) as usize]
            .iter()
            .map(|t| Touch {
                id: t.finger_id,
                x: t.x as f32,
                y: t.y as f32,
            })
            .collect()
    };
    tracker.update(&touches)
}
