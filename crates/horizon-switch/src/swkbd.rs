//! The Switch's system software keyboard (libnx `swkbd`): a library applet that takes the screen
//! while it is open and returns a UTF-8 string. [`text_input`] blocks until the player confirms
//! or cancels, so the game stops drawing meanwhile, as for any system applet.
use std::ffi::{CString, c_char};

/// Room for libnx's `SwkbdConfig`, whose layout this crate does not mirror: libnx's `swkbd*`
/// functions fill it in. It is a few hundred bytes plus the applet's argument block, well under
/// this (libnx `switch/applets/swkbd.h`).
#[repr(C, align(16))]
struct Config([u8; 16 * 1024]);

unsafe extern "C" {
    fn swkbdCreate(config: *mut Config, max_dictwords: i32) -> u32;
    fn swkbdClose(config: *mut Config);
    fn swkbdConfigMakePresetDefault(config: *mut Config);
    fn swkbdConfigSetOkButtonText(config: *mut Config, text: *const c_char);
    fn swkbdConfigSetGuideText(config: *mut Config, text: *const c_char);
    fn swkbdConfigSetInitialText(config: *mut Config, text: *const c_char);
    fn swkbdConfigSetStringLenMax(config: *mut Config, length: u32);
    fn swkbdShow(config: *mut Config, out: *mut c_char, size: usize) -> u32;
}

/// What the keyboard shows.
pub struct Prompt<'a> {
    /// The grey hint in the empty field.
    pub guide: &'a str,
    /// The confirm button's label ("Done").
    pub ok: &'a str,
    /// Text the field starts with.
    pub initial: &'a str,
    /// The longest text it accepts, in characters.
    pub max_len: u32,
}

/// Opens the keyboard. `None` if the player cancelled or the applet failed.
pub fn text_input(prompt: &Prompt<'_>) -> Option<String> {
    let c = |s: &str| CString::new(s.replace('\0', "")).ok();
    let (guide, ok, initial) = (c(prompt.guide)?, c(prompt.ok)?, c(prompt.initial)?);
    // Four bytes per character (UTF-8) and the terminator.
    let mut out = vec![0u8; prompt.max_len as usize * 4 + 1];
    let mut config = Box::new(Config([0; 16 * 1024]));
    // SAFETY: `config` is zeroed memory larger than libnx's SwkbdConfig and lives past `swkbdClose`;
    // the C strings outlive the calls (libnx copies them); `out` has the size passed to `swkbdShow`.
    unsafe {
        if swkbdCreate(&mut *config, 0) != 0 {
            return None;
        }
        swkbdConfigMakePresetDefault(&mut *config);
        swkbdConfigSetOkButtonText(&mut *config, ok.as_ptr());
        swkbdConfigSetGuideText(&mut *config, guide.as_ptr());
        swkbdConfigSetInitialText(&mut *config, initial.as_ptr());
        swkbdConfigSetStringLenMax(&mut *config, prompt.max_len);
        let rc = swkbdShow(&mut *config, out.as_mut_ptr().cast(), out.len());
        swkbdClose(&mut *config);
        if rc != 0 {
            return None;
        }
    }
    let end = out.iter().position(|&b| b == 0).unwrap_or(out.len());
    String::from_utf8(out[..end].to_vec()).ok()
}
