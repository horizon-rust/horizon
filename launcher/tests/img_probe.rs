// The probe needs `render_to_image` and the capture switches that only `test-support` builds have;
// on a default build this target is empty, as CI's `--all-targets` lint expects.
#![cfg(feature = "img-probe")]

use std::time::Duration;

use gpui::{
    div, img, point, prelude::*, px, size, Bounds, Context, ImageSource, IntoElement, ObjectFit,
    Render, Window, WindowBounds, WindowOptions,
};

struct Probe;

impl Probe {
    fn icon() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("horizon-img-probe");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("icon.png");
        std::fs::write(&path, include_bytes!("fixtures/icon.png")).unwrap();
        path
    }

    fn cell(label: &'static str, body: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .child(body)
            .child(div().text_sm().child(label))
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let source = ImageSource::from(Self::icon());
        div()
            .size_full()
            .bg(gpui::white())
            .flex()
            .items_center()
            .justify_center()
            .gap(px(24.))
            .child(Self::cell(
                "plain",
                img(source.clone()).flex_none().size(px(128.)),
            ))
            .child(Self::cell(
                "cover",
                img(source.clone())
                    .flex_none()
                    .size(px(36.))
                    .object_fit(ObjectFit::Cover),
            ))
            .child(Self::cell(
                "rounded+hidden",
                div()
                    .flex_none()
                    .size(px(40.))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(gpui::green())
                    .child(
                        img(source.clone())
                            .size(px(36.))
                            .object_fit(ObjectFit::Cover),
                    ),
            ))
            .child(Self::cell(
                "hidden only",
                div()
                    .flex_none()
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
                "rounded only",
                div()
                    .flex_none()
                    .size(px(40.))
                    .rounded(px(8.))
                    .bg(gpui::blue())
                    .child(img(source).size(px(36.)).object_fit(ObjectFit::Cover)),
            ))
    }
}

#[test]
#[ignore = "opens a real GPUI window and captures its frame"]
fn cached_png_renders() {
    gpui_platform::application().run(|cx| {
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(40.), px(40.)),
                        size(px(760.), px(220.)),
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
            let out = std::env::temp_dir().join("horizon-img-probe.png");
            match window.update(cx, |_, window, _| window.render_to_image()) {
                Ok(Ok(image)) => {
                    image.save(&out).unwrap();
                    println!(
                        "SAVED {} {}x{}",
                        out.display(),
                        image.width(),
                        image.height()
                    );
                }
                Ok(Err(error)) => println!("RENDER ERROR {error}"),
                Err(error) => println!("UPDATE ERROR {error}"),
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
