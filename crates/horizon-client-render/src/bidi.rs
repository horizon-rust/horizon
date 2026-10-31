//! Right-to-left text (`FormattedBidiReorder.reorder`, behind
//! `Language.getVisualOrder`): the logical text a player types becomes the
//! glyphs a font draws left to right. Arabic letters are shaped into their
//! joined presentation forms (ICU's `ArabicShaping` with `LETTERS_SHAPE`), the
//! text is reordered by the Unicode bidirectional algorithm, and characters in
//! right-to-left runs are mirrored. Left-to-right text comes back unchanged.
use crate::text::{Span, Text};
use std::borrow::Cow;
use unicode_bidi::{
    BidiClass, Direction, Level, ParagraphBidiInfo, bidi_class, get_base_direction,
};

/// How a letter joins its neighbours, as ICU's shaper sees it: `Right` joins
/// the letter before it only, `Dual` both neighbours, `Mark` is skipped over
/// (diacritics) and `None` never joins.
#[derive(Clone, Copy, PartialEq)]
enum Join {
    None,
    Right,
    Dual,
    Mark,
}
use Join::{Dual, Mark, Right};

/// ICU's joining table (`araLink`): code point ranges and their class, with the
/// lam-alef ligatures at the end. Anything else is `Join::None`.
const JOINS: &[(u32, u32, Join)] = &[
    (0x0622, 0x0625, Right),
    (0x0626, 0x0626, Dual),
    (0x0627, 0x0627, Right),
    (0x0628, 0x0628, Dual),
    (0x0629, 0x0629, Right),
    (0x062A, 0x062E, Dual),
    (0x062F, 0x0632, Right),
    (0x0633, 0x063A, Dual),
    (0x0640, 0x0647, Dual),
    (0x0648, 0x0649, Right),
    (0x064A, 0x064A, Dual),
    (0x064B, 0x0656, Mark),
    (0x0670, 0x0670, Mark),
    (0x0671, 0x0673, Right),
    (0x0675, 0x0677, Right),
    (0x0678, 0x0687, Dual),
    (0x0688, 0x0699, Right),
    (0x069A, 0x06B9, Dual),
    (0x06BA, 0x06BA, Right),
    (0x06BB, 0x06BF, Dual),
    (0x06C0, 0x06C0, Right),
    (0x06C1, 0x06C1, Dual),
    (0x06C2, 0x06CB, Right),
    (0x06CC, 0x06CC, Dual),
    (0x06CD, 0x06CD, Right),
    (0x06CE, 0x06D1, Dual),
    (0x06D2, 0x06D3, Right),
    (0x200D, 0x200D, Dual),
    (0x206D, 0x206F, Mark),
    (0xFEF5, 0xFEFC, Right),
];

/// Letters that have joined forms and the code point of the isolated one; the
/// final, initial and medial forms follow it (a right-joining letter has the
/// isolated and final only). The lam-alef ligatures are their own letters.
/// The extended letters (پ چ ژ ک گ ی and the rest of U+0679 to U+06D3) are
/// shaped too, as ICU's shaper does: `u_shapeArabic` and `ArabicShaping` share
/// one table, and it turns پ into U+FB58, گ into U+FB95, چ into U+FB7D and ژ
/// into U+FB8B. The extended letters that table has no presentation form for
/// (U+0671, U+067F to U+0680, U+0683 to U+0684, U+0687, U+06A4, U+06A6,
/// U+06AD, U+06B1, U+06B3 and the rest of the block) ICU leaves unchanged, so
/// they stay out of this table and are left unchanged here too.
#[rustfmt::skip]
const FORMS: &[(u32, u32)] = &[
    (0x0622, 0xFE81), (0x0623, 0xFE83), (0x0624, 0xFE85), (0x0625, 0xFE87), (0x0626, 0xFE89),
    (0x0627, 0xFE8D), (0x0628, 0xFE8F), (0x0629, 0xFE93), (0x062A, 0xFE95), (0x062B, 0xFE99),
    (0x062C, 0xFE9D), (0x062D, 0xFEA1), (0x062E, 0xFEA5), (0x062F, 0xFEA9), (0x0630, 0xFEAB),
    (0x0631, 0xFEAD), (0x0632, 0xFEAF), (0x0633, 0xFEB1), (0x0634, 0xFEB5), (0x0635, 0xFEB9),
    (0x0636, 0xFEBD), (0x0637, 0xFEC1), (0x0638, 0xFEC5), (0x0639, 0xFEC9), (0x063A, 0xFECD),
    (0x0641, 0xFED1), (0x0642, 0xFED5), (0x0643, 0xFED9), (0x0644, 0xFEDD), (0x0645, 0xFEE1),
    (0x0646, 0xFEE5), (0x0647, 0xFEE9), (0x0648, 0xFEED), (0x0649, 0xFEEF), (0x064A, 0xFEF1),
    (0x0679, 0xFB66), (0x067A, 0xFB5E), (0x067B, 0xFB52), (0x067E, 0xFB56), (0x0686, 0xFB7A),
    (0x0688, 0xFB88), (0x068C, 0xFB84), (0x068D, 0xFB82), (0x068E, 0xFB86), (0x0691, 0xFB8C),
    (0x0698, 0xFB8A), (0x06A9, 0xFB8E), (0x06AF, 0xFB92), (0x06BA, 0xFB9E), (0x06BB, 0xFBA0),
    (0x06BE, 0xFBAA), (0x06C0, 0xFBA4), (0x06C1, 0xFBA6), (0x06C5, 0xFBE0), (0x06C6, 0xFBD9),
    (0x06C7, 0xFBD7), (0x06C8, 0xFBDB), (0x06C9, 0xFBE2), (0x06CB, 0xFBDE), (0x06CC, 0xFBFC),
    (0x06D0, 0xFBE4), (0x06D2, 0xFBAE), (0x06D3, 0xFBB0),
    (0xFEF5, 0xFEF5), (0xFEF7, 0xFEF7), (0xFEF9, 0xFEF9), (0xFEFB, 0xFEFB),
];

/// `Bidi_Mirroring_Glyph` (`UCharacter.getMirror`): each pair is two
/// neighbouring characters that mirror each other.
const MIRRORED: &str = "\
        ()<>[]{}«»༺༻༼༽᚛᚜‹›⁅⁆⁽⁾₍₎∈∋∉∌∊∍∕⧵∟⯾∠⦣∡⦛∢⦠∤⫮∼∽≃⋍≅≌≒≓\
        ≔≕≤≥≦≧≨≩≪≫≮≯≰≱≲≳≴≵≶≷≸≹≺≻≼≽≾≿⊀⊁⊂⊃⊄⊅⊆⊇⊈⊉⊊⊋⊏⊐⊑⊒⊘⦸⊢⊣⊦⫞\
        ⊨⫤⊩⫣⊫⫥⊰⊱⊲⊳⊴⊵⊶⊷⊸⟜⋉⋊⋋⋌⋐⋑⋖⋗⋘⋙⋚⋛⋜⋝⋞⋟⋠⋡⋢⋣⋤⋥⋦⋧⋨⋩⋪⋫⋬⋭⋰⋱⋲⋺\
        ⋳⋻⋴⋼⋶⋽⋷⋾⌈⌉⌊⌋〈〉❨❩❪❫❬❭❮❯❰❱❲❳❴❵⟃⟄⟅⟆⟈⟉⟋⟍⟕⟖⟝⟞⟢⟣⟤⟥⟦⟧⟨⟩⟪⟫\
        ⟬⟭⟮⟯⦃⦄⦅⦆⦇⦈⦉⦊⦋⦌⦍⦐⦎⦏⦑⦒⦓⦔⦕⦖⦗⦘⦤⦥⦨⦩⦪⦫⦬⦭⦮⦯⧀⧁⧄⧅⧏⧐⧑⧒⧔⧕⧘⧙⧚⧛\
        ⧨⧩⧸⧹⧼⧽⨫⨬⨭⨮⨴⨵⨼⨽⩤⩥⩹⩺⩻⩼⩽⩾⩿⪀⪁⪂⪃⪄⪅⪆⪇⪈⪉⪊⪋⪌⪍⪎⪏⪐⪑⪒⪓⪔⪕⪖⪗⪘⪙⪚\
        ⪛⪜⪝⪞⪟⪠⪡⪢⪦⪧⪨⪩⪪⪫⪬⪭⪯⪰⪱⪲⪳⪴⪵⪶⪷⪸⪹⪺⪻⪼⪽⪾⪿⫀⫁⫂⫃⫄⫅⫆⫇⫈⫉⫊⫋⫌⫍⫎⫏⫐\
        ⫑⫒⫓⫔⫕⫖⫬⫭⫷⫸⫹⫺⸂⸃⸄⸅⸉⸊⸌⸍⸜⸝⸠⸡⸢⸣⸤⸥⸦⸧⸨⸩⹕⹖⹗⹘⹙⹚⹛⹜〈〉《》「」『』【】\
        〔〕〖〗〘〙〚〛﹙﹚﹛﹜﹝﹞﹤﹥（）＜＞［］｛｝｟｠｢｣";

fn join(c: char) -> Join {
    let c = c as u32;
    JOINS
        .iter()
        .find(|&&(from, to, _)| (from..=to).contains(&c))
        .map_or(Join::None, |&(_, _, join)| join)
}

/// The letter whose joined form `c` is (ICU shapes the forms again), or `c`.
fn letter_of(c: char) -> char {
    let c32 = c as u32;
    if !matches!(c32, 0xFE70..=0xFEFC) {
        return c;
    }
    if (0xFE70..0xFE80).contains(&c32) {
        // The diacritics' forms pair up from fathatan on.
        return char::from_u32(0x064B + (c32 - 0xFE70) / 2).unwrap_or(c);
    }
    FORMS
        .iter()
        .find(|&&(letter, iso)| {
            let forms = if join(char::from_u32(letter).unwrap_or(c)) == Dual {
                4
            } else {
                2
            };
            (iso..iso + forms).contains(&c32)
        })
        .and_then(|&(letter, _)| char::from_u32(letter))
        .unwrap_or(c)
}

/// `ArabicShaping(LETTERS_SHAPE).shape`: lam followed by an alef (diacritics
/// may sit between) becomes one ligature, then every letter takes the isolated,
/// final, initial or medial form its neighbours call for. Diacritics take the
/// tatweel form between two joined letters. The text can only get shorter.
fn shape(text: &[char]) -> Vec<char> {
    let letters: Vec<char> = text.iter().map(|&c| letter_of(c)).collect();
    let mut chars = Vec::with_capacity(letters.len());
    let mut i = 0;
    while i < letters.len() {
        if letters[i] == '\u{644}' {
            let mut at = i + 1;
            while letters.get(at).is_some_and(|&c| join(c) == Mark) {
                at += 1;
            }
            let alef = letters.get(at).and_then(|&c| {
                ['\u{622}', '\u{623}', '\u{625}', '\u{627}']
                    .iter()
                    .position(|&a| a == c)
            });
            if let Some(alef) = alef {
                chars.push(char::from_u32(0xFEF5 + 2 * alef as u32).unwrap_or('\u{644}'));
                chars.extend_from_slice(&letters[i + 1..at]);
                i = at + 1;
                continue;
            }
        }
        chars.push(letters[i]);
        i += 1;
    }
    // Whether the nearest letter before joins the next one, and the joins of
    // the nearest letter after.
    let mut joins_next = false;
    let mut out = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        let next_joins = chars[i + 1..]
            .iter()
            .map(|&n| join(n))
            .find(|&j| j != Mark)
            .is_some_and(|j| j != Join::None);
        let kind = join(c);
        let shaped = match kind {
            Join::None => {
                joins_next = false;
                c
            }
            Mark => match c as u32 {
                m @ 0x064B..=0x0652 => {
                    let iso = 0xFE70 + 2 * (m - 0x064B);
                    let tatweel = joins_next && next_joins && !matches!(m, 0x064C | 0x064D);
                    char::from_u32(iso + u32::from(tatweel)).unwrap_or(c)
                }
                _ => c,
            },
            Right | Dual => {
                let to_next = kind == Dual && next_joins;
                let form = match (joins_next, to_next) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (false, true) => 2,
                    (true, true) => 3,
                };
                joins_next = kind == Dual;
                FORMS
                    .binary_search_by_key(&(c as u32), |&(letter, _)| letter)
                    .ok()
                    .and_then(|at| char::from_u32(FORMS[at].1 + form))
                    .unwrap_or(c)
            }
        };
        out.push(shaped);
    }
    out
}

/// `UCharacter.getMirror`.
fn mirror(c: char) -> char {
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

/// Whether a character can change anything: right-to-left letters and numbers,
/// the explicit direction controls and the diacritics that shaping turns into
/// letters. Without any, left-to-right text keeps its order.
fn directional(c: char) -> bool {
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

/// Whether `chars` keep their order and shapes: left-to-right text without
/// anything directional, in a language that reads left to right by default.
fn unchanged(mut chars: impl Iterator<Item = char>, default_rtl: bool) -> bool {
    !default_rtl && !chars.any(directional)
}

/// `FormattedBidiReorder.reorder`: `chars` shaped, in visual order, with each
/// character's position in the shaped text. A character's style is the one at
/// its position in the shaped text, as `SubStringSource` takes it (the text
/// shrinks at a lam-alef ligature).
fn reorder(chars: &[char], default_rtl: bool) -> Vec<(char, usize)> {
    let shaped = shape(chars);
    let plain: String = shaped.iter().collect();
    // DEFAULT_RTL only changes text without a strong character.
    let level =
        (default_rtl && get_base_direction(plain.as_str()) == Direction::Mixed).then(Level::rtl);
    let info = ParagraphBidiInfo::new(&plain, level);
    let levels = info.reordered_levels_per_char(0..plain.len());
    ParagraphBidiInfo::reorder_visual(&levels)
        .into_iter()
        .map(|i| {
            let c = if levels[i].is_rtl() {
                mirror(shaped[i])
            } else {
                shaped[i]
            };
            (c, i)
        })
        .collect()
}

/// `Language.getVisualOrder` of a text (its `§` codes already resolved), for
/// the current language's direction.
pub fn visual(text: &Text) -> Cow<'_, Text> {
    let default_rtl = crate::lang::bidirectional();
    if unchanged(text.chars().map(|(c, _)| c), default_rtl) {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().map(|(c, _)| c).collect();
    let styles: Vec<_> = text.chars().map(|(_, style)| style).collect();
    let mut out = Text::default();
    for (c, at) in reorder(&chars, default_rtl) {
        match out.spans.last_mut() {
            Some(span) if span.style == *styles[at] => span.text.push(c),
            _ => out.spans.push(Span {
                text: c.to_string(),
                style: styles[at].clone(),
            }),
        }
    }
    Cow::Owned(out)
}

/// The same for a plain string.
pub fn visual_str(s: &str) -> Cow<'_, str> {
    let default_rtl = crate::lang::bidirectional();
    if unchanged(s.chars(), default_rtl) {
        return Cow::Borrowed(s);
    }
    let chars: Vec<char> = s.chars().collect();
    Cow::Owned(
        reorder(&chars, default_rtl)
            .into_iter()
            .map(|(c, _)| c)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `shape` makes of `s`.
    fn shaped(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        shape(&chars).into_iter().collect()
    }

    /// `s` in visual order, for a left-to-right language.
    fn visual(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        reorder(&chars, false).into_iter().map(|(c, _)| c).collect()
    }

    // The expected forms are ICU's: `u_shapeArabic(text, U_SHAPE_LETTERS_SHAPE)`
    // on ICU 79.1 returns exactly these code points for these strings, so a
    // missing or wrong row in `FORMS` fails here instead of in a chat window.

    #[test]
    fn persian_letters_take_their_joined_forms() {
        // پ گ چ ژ: initial, medial, medial, final.
        assert_eq!(
            shaped("\u{067E}\u{06AF}\u{0686}\u{0698}"),
            "\u{FB58}\u{FB95}\u{FB7D}\u{FB8B}"
        );
        // کتاب: ک initial, ت medial, ا final, ب isolated.
        assert_eq!(
            shaped("\u{06A9}\u{062A}\u{0627}\u{0628}"),
            "\u{FB90}\u{FE98}\u{FE8E}\u{FE8F}"
        );
        // یک: ی initial, ک final.
        assert_eq!(shaped("\u{06CC}\u{06A9}"), "\u{FBFE}\u{FB8F}");
    }

    #[test]
    fn right_joining_letters_take_no_initial_or_medial_form() {
        // ژ joins the letter before it only: it takes the final form and the ب
        // after it stays isolated, as ICU shapes the pair.
        assert_eq!(
            shaped("\u{0628}\u{0698}\u{0628}"),
            "\u{FE91}\u{FB8B}\u{FE8F}"
        );
    }

    #[test]
    fn base_arabic_keeps_the_forms_it_had() {
        // ب ب ب: initial, medial, final, the control for the rows above.
        assert_eq!(
            shaped("\u{0628}\u{0628}\u{0628}"),
            "\u{FE91}\u{FE92}\u{FE90}"
        );
    }

    #[test]
    fn lam_followed_by_an_alef_is_one_ligature() {
        assert_eq!(shaped("\u{0644}\u{0627}"), "\u{FEFB}");
        assert_eq!(shaped("\u{0644}\u{0622}"), "\u{FEF5}");
    }

    #[test]
    fn left_to_right_text_keeps_its_order() {
        assert_eq!(visual("Hello, world!"), "Hello, world!");
        // "MC " stays left to right; the Persian run after it is shaped and
        // reversed, so پ U+FB58 ends up rightmost.
        assert_eq!(
            visual("MC \u{067E}\u{06AF}\u{0686}\u{0698}"),
            "MC \u{FB8B}\u{FB7D}\u{FB95}\u{FB58}"
        );
    }

    #[test]
    fn the_forms_table_is_sorted_and_every_row_joins() {
        // `shape` binary-searches `FORMS`, so it has to be sorted by letter.
        for pair in FORMS.windows(2) {
            assert!(pair[0].0 < pair[1].0, "FORMS is unsorted at {pair:?}");
        }
        // A letter `JOINS` calls `Join::None` never reaches its row.
        for &(letter, _) in FORMS {
            let letter = char::from_u32(letter).expect("a letter is a char");
            assert!(
                !matches!(join(letter), Join::None),
                "{letter} does not join"
            );
        }
    }
}
