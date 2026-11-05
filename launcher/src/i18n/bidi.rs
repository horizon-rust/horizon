//! Bidirectional text reordering (UAX #9) for right-to-left languages (Arabic, Urdu, Persian, Hebrew, Yiddish).
//!
//! GPUI lays out text from left to right without applying the Unicode Bidirectional Algorithm.
//! This module shapes cursive Arabic/Urdu/Persian letters into their joined forms, reorders each
//! paragraph/line from logical order into visual order according to UAX #9 using `unicode-bidi`,
//! and mirrors paired punctuation (parentheses, brackets) in RTL runs.

use std::borrow::Cow;
use unicode_bidi::{
    bidi_class, get_base_direction, BidiClass, Direction, Level, ParagraphBidiInfo,
};

/// Paired mirroring characters from Unicode (UCharacter.getMirror).
const MIRRORED: &str = "\
    ()<>()[]{}«»༺༻༼༽᚛᚜‹›⁅⁆⁽⁾₍₎∈∋∉∌∊∍∕⧵∟⯾∠⦣∡⦛∢⦠∤⫮∼∽≃⋍≅≌≒≓\
    ≔≕≤≥≦≧≨≩≪≫≮≯≰≱≲≳≴≵≶≷≸≹≺≻≼≽≾≿⊀⊁⊂⊃⊄⊅⊆⊇⊈⊉⊊⊋⊏⊐⊑⊒⊘⦸⊢⊣⊦⫞\
    ⊨⫤⊩⫣⊫⫥⊰⊱⊲⊳⊴⊵⊶⊷⊸⟜⋉⋊⋋⋌⋐⋑⋖⋗⋘⋙⋚⋛⋜⋝⋞⋟⋠⋡⋢⋣⋤⋥⋦⋧⋨⋩⋪⋫⋬⋭⋰⋱⋲⋺\
    ⋳⋻⋴⋼⋶⋽⋷⋾⌈⌉⌊⌋〈〉❨❩❪❫❬❭❮❯❰❱❲❳❴❵⟃⟄⟅⟆⟈⟉⟋⟍⟕⟖⟝⟞⟢⟣⟤⟥⟦⟧⟨⟩⟪⟫\
    ⟬⟭⟮⟯⦃⦄⦅⦆⦇⦈⦉⦊⦋⦌⦍⦐⦎⦏⦑⦒⦓⦔⦕⦖⦗⦘⦤⦥⦨⦩⦪⦫⦬⦭⦮⦯⧀⧁⧄⧅⧏⧐⧑⧒⧔⧕⧘⧙⧚⧛\
    ⧨⧩⧸⧹⧼⧽⨫⨬⨭⨮⨴⨵⨼⨽⩤⩥⩹⩺⩻⩼⩽⩾⩿⪀⪁⪂⪃⪄⪅⪆⪇⪈⪉⪊⪋⪌⪍⪎⪏⪐⪑⪒⪓⪔⪕⪖⪗⪘⪙⪚\
    ⪛⪜⪝⪞⪟⪠⪡⪢⪦⪧⪨⪩⪪⪫⪬⪭⪯⪰⪱⪲⪳⪴⪵⪶⪷⪸⪹⪺⪻⪼⪽⪾⪿⫀⫁⫂⫃⫄⫅⫆⫇⫈⫉⫊⫋⫌⫍⫎⫏⫐\
    ⫑⫒⫓⫔⫕⫖⫬⫭⫷⫸⫹⫺⸂⸃⸄⸅⸉⸊⸌⸍⸜⸝⸠⸡⸢⸣⸤⸥⸦⸧⸨⸩⹕⹖⹗⹘⹙⹚⹛⹜〈〉《》「」『』【】\
    〔〕〖〗〘〙〚‛﹙﹚﹛﹜﹝﹞﹤﹥（）＜＞［］｛｝｟｠｢｣";

/// Mirrors paired characters (such as parentheses, brackets) in RTL runs.
pub fn mirror(c: char) -> char {
    if bidi_class(c) != BidiClass::ON {
        return c;
    }
    let mut pairs = MIRRORED.chars();
    while let (Some(a), Some(b)) = (pairs.next(), pairs.next()) {
        if c == a {
            return b;
        }
        if c == b {
            return a;
        }
    }
    c
}

/// Whether a character can affect bidi ordering.
pub fn directional(c: char) -> bool {
    c as u32 >= 0x0590
        && (matches!(c as u32, 0x064B..=0x0652)
            || matches!(
                bidi_class(c),
                BidiClass::R
                    | BidiClass::AL
                    | BidiClass::AN
                    | BidiClass::LRE
                    | BidiClass::RLE
                    | BidiClass::LRO
                    | BidiClass::RLO
                    | BidiClass::PDF
                    | BidiClass::LRI
                    | BidiClass::RLI
                    | BidiClass::FSI
                    | BidiClass::PDI
            ))
}

/// Whether the string needs no reordering.
pub fn unchanged(mut chars: impl Iterator<Item = char>, default_rtl: bool) -> bool {
    !default_rtl && !chars.any(directional)
}

/// Reorders a single line from logical to visual order according to UAX #9.
fn reorder_line(line: &str, default_rtl: bool) -> String {
    if line.is_empty() {
        return String::new();
    }
    // Shape Arabic cursive presentation forms first
    let shaped = crate::i18n::arabic::reshape(line);
    let chars: Vec<char> = shaped.chars().collect();
    if unchanged(chars.iter().copied(), default_rtl) {
        return shaped;
    }

    let level =
        (default_rtl && get_base_direction(shaped.as_str()) == Direction::Mixed).then(Level::rtl);
    let info = ParagraphBidiInfo::new(&shaped, level);
    let levels = info.reordered_levels_per_char(0..shaped.len());
    let visual = ParagraphBidiInfo::reorder_visual(&levels);

    let mut out = String::with_capacity(shaped.len());
    for i in visual {
        let c = if levels[i].is_rtl() {
            mirror(chars[i])
        } else {
            chars[i]
        };
        out.push(c);
    }
    out
}

/// Reorders a text from logical to visual order according to UAX #9.
/// Each line (separated by `\n`) is reordered independently.
pub fn visual_str<'a>(s: &'a str, default_rtl: bool) -> Cow<'a, str> {
    if unchanged(s.chars(), default_rtl) {
        return Cow::Borrowed(s);
    }
    if !s.contains('\n') {
        return Cow::Owned(reorder_line(s, default_rtl));
    }
    let lines: Vec<String> = s
        .split('\n')
        .map(|l| reorder_line(l, default_rtl))
        .collect();
    Cow::Owned(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_parentheses_in_rtl() {
        assert_eq!(mirror('('), ')');
        assert_eq!(mirror(')'), '(');
        assert_eq!(mirror('['), ']');
        assert_eq!(mirror(']'), '[');
        // Latin letters are not mirrored
        assert_eq!(mirror('A'), 'A');
    }

    #[test]
    fn reorders_arabic_word_to_visual_order() {
        // "اللغة":
        // Logical order of shaped letters:
        // [FE8D (Alef), FEDF (Lam), FEE0 (Lam), FED0 (Ghain), FE94 (Teh Marbuta)]
        // In visual order for LTR renderer:
        // [FE94 (Teh Marbuta), FED0 (Ghain), FEE0 (Lam), FEDF (Lam), FE8D (Alef)]
        let visual = visual_str("اللغة", true);
        assert_eq!(
            visual.as_ref(),
            "\u{FE94}\u{FED0}\u{FEE0}\u{FEDF}\u{FE8D}",
            "Visual order should draw Teh Marbuta first (on left) and Alef last (on right)"
        );
    }

    #[test]
    fn reorders_al_arabiyyah_to_visual_order() {
        // "العربية":
        // In visual order: Teh Marbuta, Yeh, Beh, Reh, Ain, Lam, Alef
        let visual = visual_str("العربية", true);
        assert_eq!(
            visual.as_ref(),
            "\u{FE94}\u{FEF4}\u{FE91}\u{FEAE}\u{FECC}\u{FEDF}\u{FE8D}"
        );
    }

    #[test]
    fn preserves_ltr_text_without_rtl() {
        let plain = "Hello, world!";
        assert_eq!(visual_str(plain, false).as_ref(), plain);
    }

    #[test]
    fn reorders_mixed_arabic_and_latin() {
        // "PNG أو JPEG"
        let mixed = "PNG أو JPEG";
        let visual = visual_str(mixed, true);
        // Latin words retain LTR glyph order
        assert!(visual.contains("PNG"));
        assert!(visual.contains("JPEG"));
    }

    #[test]
    fn mirrors_brackets_in_rtl_context() {
        let with_brackets = "الوكيل (Proxy)";
        let visual = visual_str(with_brackets, true);
        // In visual RTL, the parentheses enclosing Proxy should be correctly positioned
        assert!(visual.contains("Proxy"));
    }
}
