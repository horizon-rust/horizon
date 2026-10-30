//! Settings' Language row (`horizon_launcher::i18n`): the system's language, or one the player
//! picks, each named in itself.
use super::*;
use horizon_launcher::i18n::Language;

impl Launcher {
    pub(super) fn language_row(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut options = vec![(None, "System")];
        options.extend(Language::ALL.map(|l| (Some(l), l.name())));
        let picker = segments("language", &options, self.settings.language, &t, {
            let this = cx.entity().downgrade();
            move |choice, _, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.settings.language = choice;
                    this.settings.save();
                    i18n::set(this.settings.language());
                    cx.notify();
                });
            }
        })
        // Languages grow with the translation list: give them the row's full width and
        // wrap whole choices instead of taking space away from the description.
        .w_full()
        .min_w_0()
        .flex_wrap();
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
        .flex_col()
        .map(|mut row| {
            row.style().align_items = Some(gpui::AlignItems::Stretch);
            row
        })
    }
}
