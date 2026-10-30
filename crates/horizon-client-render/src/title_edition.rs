//! "RUST EDITION" under the classic title screen's logo.
//!
//! Vanilla's `textures/gui/title/edition.png` reads "JAVA EDITION". Horizon
//! never ships Mojang's art, so the banner is rebuilt at load time from the
//! player's own copy of it: "JAVA" goes, and R, U and S are assembled out of
//! the strokes of the letters already in that texture (the E's stem and arms,
//! the D's bowl and right side), the way the T is lifted out of "EDITION".
//! Nothing is drawn: every pixel of the new word is one of the texture's own,
//! moved, so the new letters keep its shading, bevels and outline exactly and
//! there are no Mojang pixels in the repository. Only vanilla 26.3's exact
//! texture is rebuilt: a resource pack's edition.png stays as its author drew it.

/// The atlas sprite.
pub const SPRITE: &str = "gui/title/edition";
const WIDTH: usize = 512;
const HEIGHT: usize = 64;
/// FNV-1a over the size and RGBA pixels of 26.3's edition.png.
const VANILLA: u64 = 0x4249_0538_6dfb_181f;
/// "EDITION", outline included, starts at this column; left of it is "JAVA".
const EDITION_START: usize = 204;
/// One letter with its outline and rim: the texture's letters sit 48 columns
/// apart, and so do the new ones.
const BOX: usize = 48;
/// The texture's T, which the new word keeps.
const VANILLA_T: usize = 331;

/// One edit to a letter: move a rect of the texture into the banner, or lay
/// the outline's black over a rect. `to` is where the rect lands.
#[derive(Clone, Copy)]
enum Op {
    Move {
        from: (usize, usize),
        to: (usize, usize),
        size: (usize, usize),
    },
    Black {
        from: (usize, usize),
        to: (usize, usize),
    },
}
use Op::{Black, Move};

/// R, out of the D: its bowl rides eight rows higher (the D's bottom arm
/// becomes the bowl's bottom), the bowl's right side stays, and the E's lower
/// stroke of the V's left arm, the one diagonal the texture has, is the leg:
/// a straight one under the bowl reads as a Q's tail at 1x.
const R: &[Op] = &[
    Move {
        from: (256, 33),
        to: (4, 25),
        size: (41, 18),
    },
    Black {
        from: (4, 43),
        to: (45, 51),
    },
    Move {
        from: (101, 24),
        to: (28, 24),
        size: (15, 27),
    },
];

/// U, out of the D: the top arm goes, and the stem's top closes the second
/// stroke so both reach the letter's top.
const U: &[Op] = &[
    Black {
        from: (63, 4),
        to: (83, 22),
    },
    Move {
        from: (256, 4),
        to: (84, 4),
        size: (11, 10),
    },
];

/// S, out of the E: the middle arm is widened to the full box, and the stem's
/// lower half moves to the right side.
const S: &[Op] = &[
    Black {
        from: (100, 28),
        to: (111, 33),
    },
    Move {
        from: (209, 4),
        to: (101, 18),
        size: (41, 18),
    },
    Move {
        from: (208, 28),
        to: (131, 28),
        size: (11, 23),
    },
];

/// The new word: the donor box in the texture, the box it goes to, and the
/// edits that turn one into the other.
const LETTERS: [(usize, usize, &[Op]); 4] = [
    (252, 0, R),
    (252, 48, U),
    (204, 96, S),
    (VANILLA_T, 144, &[]),
];

/// The banner rebuilt to read "RUST EDITION", or `None` when the texture is
/// not vanilla's (a resource pack's own).
pub fn rust_edition(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    (fingerprint(width, height, pixels) == VANILLA).then(|| compose(pixels))
}

fn fingerprint(width: u32, height: u32, pixels: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let bytes = width.to_le_bytes().into_iter().chain(height.to_le_bytes());
    for b in bytes.chain(pixels.iter().copied()) {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// "RUST" in place of "JAVA"; `pixels` is a 512x64 RGBA banner laid out as
/// vanilla's.
fn compose(pixels: &[u8]) -> Vec<u8> {
    let mut out = pixels.to_vec();
    for row in out.as_chunks_mut::<{ WIDTH * 4 }>().0 {
        row[..EDITION_START * 4].fill(0);
    }
    for (donor, dest, ops) in LETTERS {
        move_rect(&mut out, pixels, (donor, 0), (dest, 0), (BOX, HEIGHT));
        for op in ops {
            match *op {
                Move { from, to, size } => move_rect(&mut out, pixels, from, to, size),
                Black { from, to } => {
                    for y in from.1..to.1 {
                        for x in from.0..to.0 {
                            out[(y * WIDTH + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 255]);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Copy a rect of the texture into the banner, keeping its transparency.
fn move_rect(
    out: &mut [u8],
    pixels: &[u8],
    from: (usize, usize),
    to: (usize, usize),
    size: (usize, usize),
) {
    for y in 0..size.1 {
        for x in 0..size.0 {
            let source = ((from.1 + y) * WIDTH + from.0 + x) * 4;
            if pixels[source + 3] == 0 {
                continue;
            }
            let dest = ((to.1 + y) * WIDTH + to.0 + x) * 4;
            out[dest..][..4].copy_from_slice(&pixels[source..][..4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A banner laid out as vanilla's, in made-up inks: the whole box of every
    /// letter is a colour of its own, so a stroke that moves shows up.
    fn stand_in() -> Vec<u8> {
        // Where vanilla's letters start: JAVA, then EDITION.
        let boxes = [0, 48, 96, 144, 204, 252, 300, 331, 381, 413, 460];
        let mut out = vec![0; WIDTH * HEIGHT * 4];
        for (i, &start) in boxes.iter().enumerate() {
            let ink = [20 + 20 * i as u8, 200, 100, 255];
            for y in 0..HEIGHT {
                for x in start..(start + BOX).min(WIDTH) {
                    out[(y * WIDTH + x) * 4..][..4].copy_from_slice(&ink);
                }
            }
        }
        // A colour only "JAVA" has, to see it go.
        out[(20 * WIDTH + 30) * 4..][..4].copy_from_slice(&[255, 0, 255, 255]);
        out
    }

    fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
        pixels[(y * WIDTH + x) * 4..][..4].try_into().unwrap()
    }

    fn columns(pixels: &[u8], range: std::ops::Range<usize>) -> Vec<u8> {
        pixels
            .as_chunks::<{ WIDTH * 4 }>()
            .0
            .iter()
            .flat_map(|row| row[range.start * 4..range.end * 4].to_vec())
            .collect()
    }

    #[test]
    fn rust_replaces_java_and_edition_is_untouched() {
        let source = stand_in();
        let out = compose(&source);
        assert_eq!(out.len(), WIDTH * HEIGHT * 4);
        assert_eq!(
            columns(&out, EDITION_START..WIDTH),
            columns(&source, EDITION_START..WIDTH)
        );
        assert!(!out.as_chunks::<4>().0.contains(&[255, 0, 255, 255]));
        // Each new letter carries its donor's inks: R and U the D's (5), S the
        // E's (4), T vanilla's own (7).
        assert_eq!(pixel(&out, 8, 20), pixel(&source, 260, 20), "R is the D");
        assert_eq!(pixel(&out, 56, 20), pixel(&source, 260, 20), "U is the D");
        assert_eq!(pixel(&out, 104, 20), pixel(&source, 212, 20), "S is the E");
        assert_eq!(pixel(&out, 152, 20), pixel(&source, 335, 20), "T is the T");
        // The word spans "JAVA"'s columns, so the banner stays centred.
        let used = |x: usize| (0..HEIGHT).any(|y| pixel(&out, x, y)[3] > 0);
        let last = (0..EDITION_START).rev().find(|&x| used(x)).unwrap();
        assert!(
            used(0) && (189..=196).contains(&last),
            "RUST ends at {last}"
        );
        // And the word gap stays open.
        let gap = columns(&out, last + 1..EDITION_START);
        assert!(gap.as_chunks::<4>().0.iter().all(|p| p[3] == 0));
    }

    #[test]
    fn only_vanilla_is_rebuilt() {
        let source = stand_in();
        assert!(rust_edition(512, 64, &source).is_none());
    }

    /// Against the prepared vanilla resources when they are present.
    #[test]
    fn prepared_vanilla_edition() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION)
            .join("assets/minecraft/textures/gui/title/edition.png");
        let Ok((w, h, pixels)) = crate::hud::decode(&path) else {
            return;
        };
        let out = rust_edition(w, h, &pixels).expect("vanilla's edition.png");
        assert_eq!(
            columns(&out, EDITION_START..WIDTH),
            columns(&pixels, EDITION_START..WIDTH)
        );
        // No drawn pixels: everything left of EDITION is one of the texture's
        // own, so it keeps the texture's inks.
        let inks: std::collections::HashSet<[u8; 4]> =
            pixels.as_chunks::<4>().0.iter().copied().collect();
        for p in out.as_chunks::<4>().0 {
            assert!(p[3] == 0 || inks.contains(p), "{p:?} is not the texture's");
        }
        let mut edited = pixels.clone();
        edited[3] ^= 1;
        assert!(rust_edition(w, h, &edited).is_none());
    }
}
