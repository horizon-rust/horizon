//! `ScreensClientHost::request_screenshot` from a wasm mod reaches the client with
//! its size intact, and each named refusal comes back as the native call returns
//! it (#9606).
use super::*;
use mod_api::ScreenshotError;
use mod_api::prelude::*;
use std::cell::RefCell;

/// A client that takes one screenshot at a time, and records each size asked for.
#[derive(Default)]
struct Client {
    asked: RefCell<Vec<Option<[u32; 2]>>>,
    in_progress: RefCell<bool>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(screens));
impl ScreensClientHost for Client {
    fn request_screenshot(&mut self, size: Option<[u32; 2]>) -> Result<(), ScreenshotError> {
        self.asked.borrow_mut().push(size);
        if size.is_some() {
            return Err(ScreenshotError::UnsupportedSize);
        }
        if self.in_progress.replace(true) {
            return Err(ScreenshotError::Busy);
        }
        Ok(())
    }
}

#[test]
fn a_wasm_mods_screenshot_crosses_the_wire_and_a_second_is_refused_while_the_first_runs() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: client and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    assert_eq!(
        guest::Guest.request_screenshot(Some([3840, 2160])),
        Err(ScreenshotError::UnsupportedSize)
    );
    assert_eq!(guest::Guest.request_screenshot(None), Ok(()));
    assert_eq!(
        guest::Guest.request_screenshot(None),
        Err(ScreenshotError::Busy)
    );
    guest::set_transport(None);
    assert_eq!(*client.asked.borrow(), [Some([3840, 2160]), None, None]);
}
