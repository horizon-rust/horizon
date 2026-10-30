//! Settings' Language row (`horizon_launcher::i18n`): the system's language, or one the player
//! picks, each named in itself.
use super::*;
use horizon_launcher::i18n::Language;

impl Launcher {
    pub(super) fn language_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let current = self.settings.language;
        let selected = current.map_or("System", |language| language.name());

        let picker = div()
            .relative()
            .child(
                button("language-picker", selected, Kind::Secondary, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.language_menu = !this.language_menu;
                        cx.notify();
                    },
                )),
            )
            .when(self.language_menu, |picker| {
                let mut options: Vec<_> = Language::ALL
                    .into_iter()
                    .map(|language| (Some(language), language.name()))
                    .collect();
                options.sort_by(|a, b| a.1.cmp(b.1));
                options.insert(0, (None, "System"));

                let mut menu = div()
                    .id("language-menu")
                    .absolute()
                    .top(px(theme::CONTROL + 6.))
                    .right_0()
                    .w(px(220.))
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .border_2()
                    .border_color(t.outline)
                    .bg(t.surface)
                    .flex()
                    .flex_col()
                    .occlude();

                for (i, (choice, name)) in options.into_iter().enumerate() {
                    let chosen = choice == current;
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("language-pick-{i}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .h(px(34.))
                            .px(px(10.))
                            .rounded(px(theme::RADIUS))
                            .cursor_pointer()
                            .hover(move |s| s.bg(t.plate_soft))
                            .child(
                                text(theme::LABEL)
                                    .text_color(if chosen { t.ink } else { t.muted })
                                    .child(tr(name)),
                            )
                            .when(chosen, |d| {
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

                picker.child(gpui::deferred(menu).with_priority(1))
            });

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
