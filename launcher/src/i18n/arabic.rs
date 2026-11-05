//! Arabic script shaping (contextual letter joining and mandatory ligatures).
//!
//! Maps standard Arabic Unicode characters (U+0600–U+06FF) to their context-dependent
//! Arabic Presentation Forms-B (U+FE70–U+FEFF) and Forms-A (U+FB50–U+FDFF) glyphs
//! (Isolated, Initial, Medial, Final forms and Lam-Alef ligatures).
//!
//! This enables UI engines that lack native OpenType GSUB complex script shaping
//! to render fluent, connected cursive Arabic text using standard system fallback fonts
//! (such as Segoe UI, Arial, San Francisco, or DejaVu Sans).

#[derive(Clone, Copy, PartialEq, Eq)]
enum JoinType {
    None,
    RightOnly,
    Dual,
}

#[derive(Clone, Copy)]
struct LetterForms {
    isolated: char,
    final_form: Option<char>,
    initial: Option<char>,
    medial: Option<char>,
    join_type: JoinType,
}

impl LetterForms {
    const fn right_only(isolated: char, final_form: char) -> Self {
        Self {
            isolated,
            final_form: Some(final_form),
            initial: None,
            medial: None,
            join_type: JoinType::RightOnly,
        }
    }

    const fn dual(isolated: char, final_form: char, initial: char, medial: char) -> Self {
        Self {
            isolated,
            final_form: Some(final_form),
            initial: Some(initial),
            medial: Some(medial),
            join_type: JoinType::Dual,
        }
    }

    const fn none(isolated: char) -> Self {
        Self {
            isolated,
            final_form: None,
            initial: None,
            medial: None,
            join_type: JoinType::None,
        }
    }
}

fn letter_info(c: char) -> Option<LetterForms> {
    match c {
        // Hamza
        '\u{0621}' => Some(LetterForms::none('\u{FE80}')),
        // Alef with Madda Above
        '\u{0622}' => Some(LetterForms::right_only('\u{FE81}', '\u{FE82}')),
        // Alef with Hamza Above
        '\u{0623}' => Some(LetterForms::right_only('\u{FE83}', '\u{FE84}')),
        // Waw with Hamza Above
        '\u{0624}' => Some(LetterForms::right_only('\u{FE85}', '\u{FE86}')),
        // Alef with Hamza Below
        '\u{0625}' => Some(LetterForms::right_only('\u{FE87}', '\u{FE88}')),
        // Yeh with Hamza Above
        '\u{0626}' => Some(LetterForms::dual(
            '\u{FE89}', '\u{FE8A}', '\u{FE8B}', '\u{FE8C}',
        )),
        // Alef
        '\u{0627}' => Some(LetterForms::right_only('\u{FE8D}', '\u{FE8E}')),
        // Beh
        '\u{0628}' => Some(LetterForms::dual(
            '\u{FE8F}', '\u{FE90}', '\u{FE91}', '\u{FE92}',
        )),
        // Teh Marbuta
        '\u{0629}' => Some(LetterForms::right_only('\u{FE93}', '\u{FE94}')),
        // Teh
        '\u{062A}' => Some(LetterForms::dual(
            '\u{FE95}', '\u{FE96}', '\u{FE97}', '\u{FE98}',
        )),
        // Theh
        '\u{062B}' => Some(LetterForms::dual(
            '\u{FE99}', '\u{FE9A}', '\u{FE9B}', '\u{FE9C}',
        )),
        // Jeem
        '\u{062C}' => Some(LetterForms::dual(
            '\u{FE9D}', '\u{FE9E}', '\u{FE9F}', '\u{FEA0}',
        )),
        // Hah
        '\u{062D}' => Some(LetterForms::dual(
            '\u{FEA1}', '\u{FEA2}', '\u{FEA3}', '\u{FEA4}',
        )),
        // Khah
        '\u{062E}' => Some(LetterForms::dual(
            '\u{FEA5}', '\u{FEA6}', '\u{FEA7}', '\u{FEA8}',
        )),
        // Dal
        '\u{062F}' => Some(LetterForms::right_only('\u{FEA9}', '\u{FEAA}')),
        // Thal
        '\u{0630}' => Some(LetterForms::right_only('\u{FEAB}', '\u{FEAC}')),
        // Reh
        '\u{0631}' => Some(LetterForms::right_only('\u{FEAD}', '\u{FEAE}')),
        // Zain
        '\u{0632}' => Some(LetterForms::right_only('\u{FEAF}', '\u{FEB0}')),
        // Seen
        '\u{0633}' => Some(LetterForms::dual(
            '\u{FEB1}', '\u{FEB2}', '\u{FEB3}', '\u{FEB4}',
        )),
        // Sheen
        '\u{0634}' => Some(LetterForms::dual(
            '\u{FEB5}', '\u{FEB6}', '\u{FEB7}', '\u{FEB8}',
        )),
        // Sad
        '\u{0635}' => Some(LetterForms::dual(
            '\u{FEB9}', '\u{FEBA}', '\u{FEBB}', '\u{FEBC}',
        )),
        // Dad
        '\u{0636}' => Some(LetterForms::dual(
            '\u{FEBD}', '\u{FEBE}', '\u{FEBF}', '\u{FEC0}',
        )),
        // Tah
        '\u{0637}' => Some(LetterForms::dual(
            '\u{FEC1}', '\u{FEC2}', '\u{FEC3}', '\u{FEC4}',
        )),
        // Zah
        '\u{0638}' => Some(LetterForms::dual(
            '\u{FEC5}', '\u{FEC6}', '\u{FEC7}', '\u{FEC8}',
        )),
        // Ain
        '\u{0639}' => Some(LetterForms::dual(
            '\u{FEC9}', '\u{FECA}', '\u{FECB}', '\u{FECC}',
        )),
        // Ghain
        '\u{063A}' => Some(LetterForms::dual(
            '\u{FECD}', '\u{FECE}', '\u{FECF}', '\u{FED0}',
        )),
        // Tatweel (Kashida)
        '\u{0640}' => Some(LetterForms::dual(
            '\u{0640}', '\u{0640}', '\u{0640}', '\u{0640}',
        )),
        // Feh
        '\u{0641}' => Some(LetterForms::dual(
            '\u{FED1}', '\u{FED2}', '\u{FED3}', '\u{FED4}',
        )),
        // Qaf
        '\u{0642}' => Some(LetterForms::dual(
            '\u{FED5}', '\u{FED6}', '\u{FED7}', '\u{FED8}',
        )),
        // Kaf
        '\u{0643}' => Some(LetterForms::dual(
            '\u{FED9}', '\u{FEDA}', '\u{FEDB}', '\u{FEDC}',
        )),
        // Lam
        '\u{0644}' => Some(LetterForms::dual(
            '\u{FEDD}', '\u{FEDE}', '\u{FEDF}', '\u{FEE0}',
        )),
        // Meem
        '\u{0645}' => Some(LetterForms::dual(
            '\u{FEE1}', '\u{FEE2}', '\u{FEE3}', '\u{FEE4}',
        )),
        // Noon
        '\u{0646}' => Some(LetterForms::dual(
            '\u{FEE5}', '\u{FEE6}', '\u{FEE7}', '\u{FEE8}',
        )),
        // Heh
        '\u{0647}' => Some(LetterForms::dual(
            '\u{FEE9}', '\u{FEEA}', '\u{FEEB}', '\u{FEEC}',
        )),
        // Waw
        '\u{0648}' => Some(LetterForms::right_only('\u{FEED}', '\u{FEEE}')),
        // Alef Maksura
        '\u{0649}' => Some(LetterForms::right_only('\u{FEEF}', '\u{FEF0}')),
        // Yeh
        '\u{064A}' => Some(LetterForms::dual(
            '\u{FEF1}', '\u{FEF2}', '\u{FEF3}', '\u{FEF4}',
        )),
        // Persian Pe
        '\u{067E}' => Some(LetterForms::dual(
            '\u{FB56}', '\u{FB57}', '\u{FB58}', '\u{FB59}',
        )),
        // Persian Tcheh
        '\u{0686}' => Some(LetterForms::dual(
            '\u{FB7A}', '\u{FB7B}', '\u{FB7C}', '\u{FB7D}',
        )),
        // Persian / Urdu Jeh
        '\u{0698}' => Some(LetterForms::right_only('\u{FB8A}', '\u{FB8B}')),
        // Persian Keheh
        '\u{06A9}' => Some(LetterForms::dual(
            '\u{FB8E}', '\u{FB8F}', '\u{FB90}', '\u{FB91}',
        )),
        // Persian Gaf
        '\u{06AF}' => Some(LetterForms::dual(
            '\u{FB92}', '\u{FB93}', '\u{FB94}', '\u{FB95}',
        )),
        // Persian / Urdu Farsi Yeh
        '\u{06CC}' => Some(LetterForms::dual(
            '\u{FBFC}', '\u{FBFD}', '\u{FBFE}', '\u{FBFF}',
        )),
        _ => None,
    }
}

/// Tashkeel / Harakat and other non-spacing combining marks that do not break joining.
fn is_transparent(c: char) -> bool {
    matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}')
}

/// Mandatory Lam-Alef ligatures.
fn lam_alef(alef: char, final_form: bool) -> Option<char> {
    match alef {
        '\u{0622}' => Some(if final_form { '\u{FEF6}' } else { '\u{FEF5}' }), // Madda
        '\u{0623}' => Some(if final_form { '\u{FEF8}' } else { '\u{FEF7}' }), // Hamza Above
        '\u{0625}' => Some(if final_form { '\u{FEFA}' } else { '\u{FEF9}' }), // Hamza Below
        '\u{0627}' => Some(if final_form { '\u{FEFC}' } else { '\u{FEFB}' }), // Plain Alef
        _ => None,
    }
}

/// Reshapes Arabic text into cursive presentation forms with connected glyphs and ligatures.
pub fn reshape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;

    while i < n {
        let c = chars[i];
        let Some(info) = letter_info(c) else {
            out.push(c);
            i += 1;
            continue;
        };

        // Mandatory Lam-Alef ligature check
        if c == '\u{0644}' {
            let mut j = i + 1;
            while j < n && is_transparent(chars[j]) {
                j += 1;
            }
            if j < n {
                let next_char = chars[j];
                // Check if the previous non-transparent character connects forward
                let mut prev_connects = false;
                let mut k = i as isize - 1;
                while k >= 0 && is_transparent(chars[k as usize]) {
                    k -= 1;
                }
                if k >= 0 {
                    if let Some(prev_info) = letter_info(chars[k as usize]) {
                        prev_connects = prev_info.join_type == JoinType::Dual;
                    }
                }

                if let Some(ligature) = lam_alef(next_char, prev_connects) {
                    out.push(ligature);
                    // Emit any transparent marks between Lam and Alef
                    for mark in &chars[i + 1..j] {
                        out.push(*mark);
                    }
                    i = j + 1;
                    continue;
                }
            }
        }

        // Determine joining context
        let mut prev_connects = false;
        let mut k = i as isize - 1;
        while k >= 0 && is_transparent(chars[k as usize]) {
            k -= 1;
        }
        if k >= 0 {
            if let Some(prev_info) = letter_info(chars[k as usize]) {
                prev_connects = prev_info.join_type == JoinType::Dual;
            }
        }

        let mut next_connects = false;
        let mut k = i + 1;
        while k < n && is_transparent(chars[k]) {
            k += 1;
        }
        if k < n {
            if let Some(next_info) = letter_info(chars[k]) {
                next_connects = matches!(next_info.join_type, JoinType::Dual | JoinType::RightOnly);
            }
        }

        let can_join_left = info.join_type == JoinType::Dual;

        let shaped = match (prev_connects, next_connects && can_join_left) {
            (true, true) => info.medial.or(info.final_form).unwrap_or(info.isolated),
            (true, false) => info.final_form.unwrap_or(info.isolated),
            (false, true) => info.initial.unwrap_or(info.isolated),
            (false, false) => info.isolated,
        };

        out.push(shaped);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_allughah_into_connected_forms() {
        let reshaped = reshape("اللغة");
        assert_eq!(
            reshaped,
            "\u{FE8D}\u{FEDF}\u{FEE0}\u{FED0}\u{FE94}",
            "اللغة should be shaped into isolated Alef, initial Lam, medial Lam, medial Ghain, final Teh Marbuta"
        );
    }

    #[test]
    fn shapes_al_arabiyyah_into_connected_forms() {
        let reshaped = reshape("العربية");
        assert_eq!(
            reshaped,
            "\u{FE8D}\u{FEDF}\u{FECC}\u{FEAE}\u{FE91}\u{FEF4}\u{FE94}",
            "العربية should be shaped into isolated Alef, initial Lam, medial Ain, final Reh, initial Beh, medial Yeh, final Teh Marbuta"
        );
    }

    #[test]
    fn shapes_lam_alef_ligatures() {
        // Isolated Lam-Alef: لا
        assert_eq!(reshape("لا"), "\u{FEFB}");
        // Final Lam-Alef in سلام: س (initial) + ل + ا (final Lam-Alef) + م (isolated)
        assert_eq!(reshape("سلام"), "\u{FEB3}\u{FEFC}\u{FEE1}");
    }

    #[test]
    fn preserves_non_arabic_text_and_placeholders() {
        let mixed = "PNG أو JPEG {name}";
        let reshaped = reshape(mixed);
        assert!(reshaped.starts_with("PNG "));
        assert!(reshaped.contains(" JPEG {name}"));
    }
}
