// The probe needs `render_to_image` and the capture switches that only `test-support` builds have;
// on a default build this target is empty, as CI's `--all-targets` lint expects.
#![cfg(feature = "img-probe")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::RgbaImage;

use gpui::{
    div, img, point, prelude::*, px, size, Bounds, Context, ImageSource, IntoElement, ObjectFit,
    Render, Window, WindowBounds, WindowOptions,
};

/// Logical size of the window; the frame comes back in device pixels.
const WINDOW_WIDTH: f32 = 760.;
const WINDOW_HEIGHT: f32 = 220.;
/// Top-left quadrant of the fixture's 2x2 checkerboard, in sRGB.
const QUADRANT: [u8; 3] = [240, 200, 40];
/// Cells sit at fixed offsets so the sampled points do not depend on text metrics.
const TOP: f32 = 40.;
const CELL_WIDTH: f32 = 140.;
/// A point 32 px inside the 128 px control image, in the top-left quadrant.
const PLAIN_SAMPLE: (f32, f32) = (58., 72.);
/// A point 9 px inside the 36 px image clipped by the 40 px rounded box.
const CLIPPED_SAMPLE: (f32, f32) = (359., 49.);

struct Probe;

impl Probe {
    fn icon() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-img-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.png");
        std::fs::write(&path, include_bytes!("fixtures/icon.png")).unwrap();
        path
    }

    fn cell(left: f32, label: &'static str, body: impl IntoElement) -> gpui::Div {
        div()
            .absolute()
            .left(px(left))
            .top(px(TOP))
            .w(px(CELL_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .child(body)
            .child(div().text_sm().child(label))
    }

    fn clipped(source: &ImageSource) -> gpui::Div {
        div()
            .size(px(40.))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(gpui::green())
            .child(
                img(source.clone())
                    .size(px(36.))
                    .object_fit(ObjectFit::Cover),
            )
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let source = ImageSource::from(Self::icon());
        div()
            .size_full()
            .relative()
            .bg(gpui::white())
            .child(Self::cell(20., "plain", img(source.clone()).size(px(128.))))
            .child(Self::cell(
                160.,
                "cover",
                img(source.clone())
                    .size(px(36.))
                    .object_fit(ObjectFit::Cover),
            ))
            .child(Self::cell(300., "rounded+hidden", Self::clipped(&source)))
            .child(Self::cell(
                440.,
                "hidden only",
                div()
                    .size(px(40.))
                    .overflow_hidden()
                    .bg(gpui::yellow())
                    .child(
                        img(source.clone())
                            .size(px(36.))
                            .object_fit(ObjectFit::Cover),
                    ),
            ))
            .child(Self::cell(
                580.,
                "rounded only",
                div()
                    .size(px(40.))
                    .rounded(px(8.))
                    .bg(gpui::blue())
                    .child(img(source).size(px(36.)).object_fit(ObjectFit::Cover)),
            ))
    }
}

/// Compares one device pixel at a logical position with the fixture's quadrant colour. Rounded
/// corners are square on purpose on Windows, so only interior points are sampled.
fn sample(frame: &RgbaImage, logical: (f32, f32), what: &str) -> Result<(), String> {
    let scale = frame.width() as f32 / WINDOW_WIDTH;
    let x = (logical.0 * scale).round() as u32;
    let y = (logical.1 * scale).round() as u32;
    if x >= frame.width() || y >= frame.height() {
        return Err(format!(
            "{what}: sample ({x}, {y}) lies outside the {}x{} frame",
            frame.width(),
            frame.height()
        ));
    }
    let [r, g, b, _] = frame.get_pixel(x, y).0;
    let near = |actual: u8, expected: u8| actual.abs_diff(expected) <= 16;
    if near(r, QUADRANT[0]) && near(g, QUADRANT[1]) && near(b, QUADRANT[2]) {
        Ok(())
    } else {
        Err(format!(
            "{what}: expected rgb({}, {}, {}) at ({x}, {y}), got rgb({r}, {g}, {b})",
            QUADRANT[0], QUADRANT[1], QUADRANT[2]
        ))
    }
}

fn capture(frame: &RgbaImage) -> Result<PathBuf, String> {
    let out = std::env::temp_dir().join(format!("horizon-img-probe-{}.png", std::process::id()));
    frame
        .save(&out)
        .map_err(|error| format!("saving {} failed: {error}", out.display()))?;
    sample(frame, PLAIN_SAMPLE, "unclipped control icon")?;
    sample(frame, CLIPPED_SAMPLE, "icon inside a rounded overflow clip")?;
    Ok(out)
}

#[test]
#[ignore = "opens a real GPUI window and captures its frame"]
fn cached_png_renders() {
    let outcome: Arc<Mutex<Option<Result<PathBuf, String>>>> = Arc::new(Mutex::new(None));
    gpui_platform::application().run({
        let outcome = outcome.clone();
        move |cx| {
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(40.), px(40.)),
                            size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                        ))),
                        focus: false,
                        show: true,
                        ..WindowOptions::default()
                    },
                    |_, cx| cx.new(|_| Probe),
                )
                .unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(2500))
                    .await;
                let result = match window.update(cx, |_, window, _| window.render_to_image()) {
                    Ok(Ok(frame)) => capture(&frame),
                    Ok(Err(error)) => Err(format!("render_to_image failed: {error}")),
                    Err(error) => Err(format!("window update failed: {error}")),
                };
                *outcome.lock().unwrap() = Some(result);
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
    let result = outcome.lock().unwrap().take();
    match result {
        Some(Ok(path)) => println!("SAVED {}", path.display()),
        Some(Err(error)) => panic!("{error}"),
        None => panic!("the probe exited without capturing a frame"),
    }
}
