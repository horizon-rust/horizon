//! Settings' Language row (`horizon_launcher::i18n`): the system's language, or one the player
//! picks, each named in itself.
use super::*;
use horizon_launcher::i18n::Language;

impl Launcher {
    pub(super) fn language_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let current = self.settings.language;
        let label = current.map_or_else(|| tr("System"), |l| l.name());
        let trigger_id = SharedString::from("language");
        let trigger = keyboard(
            track_hover(
                div()
                    .id(trigger_id.clone())
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .min_w(px(150.))
                    .h(px(theme::CONTROL))
                    .px(px(14.))
                    .border_2()
                    .border_color(t.outline)
                    .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
                    .bg(t.button)
                    .text_color(t.ink)
                    .font_family(theme::PIXEL)
                    .text_size(px(15.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .child(hover_layer(
                        &trigger_id,
                        t.bevel.opacity(if t.dark { 0.10 } else { 0.35 }),
                        0.,
                        0.,
                    ))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_between()
                            .gap(px(10.))
                            .pb(px(2.))
                            .child(label)
                            .child(blocks::icon("chevron-down", 16., t.muted)),
                    ),
                trigger_id,
            ),
            &t,
        )
        .active(|s| s.opacity(theme::PRESSED).top(px(1.)))
        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
            if !this.closed_by_this_press(event) {
                this.language_menu = !this.language_menu;
            }
            cx.notify();
        }));
        let mut picker = div().relative().flex_none().child(trigger);
        if self.language_menu {
            let mut options = vec![(None, tr("System"))];
            options.extend(Language::ALL.map(|l| (Some(l), l.name())));
            let mut menu = div()
                .id("language-menu")
                .absolute()
                .top(px(theme::CONTROL + 6.))
                .right_0()
                .w(px(200.))
                .p(px(6.))
                .border_2()
                .border_color(t.outline)
                .bg(t.surface)
                .flex()
                .flex_col()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.language_menu = false;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            for (choice, name) in options {
                let selected = choice == current;
                let id_key = choice.map_or("system", |l| l.code());
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("language-{id_key}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::PIXEL)
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if selected { t.ink } else { t.ink_soft })
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                        .child(name)
                        .when(selected, |d| {
                            d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.language = choice;
                            this.settings.save();
                            i18n::set(this.settings.language());
                            this.language_menu = false;
                            cx.notify();
                        })),
                );
            }
            picker = picker.child(gpui::deferred(menu).with_priority(1));
        }
        let said = trf(
            "System follows your computer's language: {language} here. A string not translated \
             yet shows in English.",
            &[("language", Language::system().name())],
        );
        index_row(
            0,
            tr("Language"),
            Some(div().child(said).into_any_element()),
            Some(picker.into_any_element()),
            &t,
        )
    }
}
