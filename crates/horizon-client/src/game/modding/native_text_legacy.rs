//! `text_from_legacy`: legacy `§` formatting codes read as the font draws them.
use super::Game;
use horizon_client::text::Text;

impl Game<'_> {
    /// Legacy formatting codes read as the font draws them, and the `§x` hex colour form.
    pub(super) fn text_from_legacy(&self, legacy: &str) -> Text {
        Text::legacy_with_hex(legacy)
    }
}

#[cfg(test)]
mod tests {
    use super::Text;
    use crate::game::api_values::TryIntoApi;

    /// Colour codes reset decorations, `§r` resets everything, unknown codes are dropped.
    #[test]
    fn text_from_legacy_reads_section_codes() {
        let text = Text::legacy("§cRed §lbold§r plain§zx");
        assert_eq!(text.plain(), "Red bold plainx");
        let spans: Vec<_> = text.chars().map(|(c, style)| (c, style.clone())).collect();
        assert_eq!(spans[0].1.color, Some(0xFF5555));
        assert!(!spans[0].1.is_bold());
        assert!(spans[4].1.is_bold() && spans[4].1.color == Some(0xFF5555));
        assert_eq!(spans[9].1.color, None);
        let api: mod_api::Text = text.try_into_api().unwrap();
        assert!(api.validate().is_ok());
    }

    /// `§x` and six digit pairs set that colour (and clear decorations, as a colour code does);
    /// an incomplete run is dropped like an unknown code; the font's own `legacy` ignores it.
    #[test]
    fn text_from_legacy_reads_the_hex_colour_form() {
        let text = Text::legacy_with_hex("§lA§x§f§f§0§0§a§aB§X§0§0§0§0§0§1C");
        assert_eq!(text.plain(), "ABC");
        let spans: Vec<_> = text.chars().map(|(c, style)| (c, style.clone())).collect();
        assert!(spans[0].1.is_bold());
        assert_eq!(spans[1].1.color, Some(0xFF00AA));
        assert!(!spans[1].1.is_bold(), "a colour clears the decorations");
        assert_eq!(spans[2].1.color, Some(0x000001), "upper case X reads too");
        let short: Vec<_> = Text::legacy_with_hex("§x§f§fD")
            .chars()
            .map(|(c, style)| (c, style.color))
            .collect();
        assert_eq!(short, [('D', Some(0xFFFFFF))], "§x dropped, then §f twice");
        let font: Vec<_> = Text::legacy("§x§f§f§0§0§a§aB")
            .chars()
            .map(|(c, style)| (c, style.color))
            .collect();
        assert_eq!(font, [('B', Some(0x55FF55))], "the font's reading: the last §a");
    }
}
