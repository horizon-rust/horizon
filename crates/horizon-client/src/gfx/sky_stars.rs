//! RT lighting's real night sky (terrain.wgsl `vs_rt_star`): the Yale Bright Star Catalogue's
//! stars (BSC5, 9096 stars to magnitude 6.5; data in the public domain, converted by Bretton Wade's
//! MIT-licensed YaleBrightStarCatalog), each its J2000 equatorial direction, magnitude and
//! temperature, eight bytes in `stars_bsc5.bin`; and the turn that puts them over the player: the
//! latitude the sun's path leans by (`celestial`, 43 degrees for Aix-en-Provence), the sidereal
//! time from the real date and the in-game hour (noon when the sun is highest).

use glam::{Mat4, Vec4};

/// The catalogue: x, y, z (16-bit, the unit direction times 32767), then the magnitude (`(V + 2)
/// * 25`) and the temperature (in hundreds of kelvins), a byte each.
const CATALOGUE: &[u8] = include_bytes!("stars_bsc5.bin");
/// Stars in the catalogue.
pub const COUNT: u32 = (CATALOGUE.len() / 8) as u32;
/// A star an instance: its three directions and its packed magnitude and temperature.
pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 8,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &wgpu::vertex_attr_array![0 => Sint16x4],
};

/// The shooting stars (terrain.wgsl `rt_meteor`) on a date: the seconds of a slot, the chance a
/// slot holds one, and the active shower (its radiant and rates, `Shower`). The Shooting Stars
/// setting (`setting`): 0 none; 1 the real sky's, the sporadic meteors and the year's showers on
/// their dates, eight times their real rates (a game's night is a few minutes, an observer's
/// hours); 2 frequent, a hundred times as many. The environment's
/// `HORIZON_METEORS` (`off`, `rare`, `frequent`) overrides it.
pub struct Meteors {
    pub slot: f32,
    pub chance: f32,
    /// The shower's radiant (equatorial, J2000; xyz) and its zenithal hourly rate now (w).
    pub radiant: [f32; 4],
    /// Its meteors' speed (km/s, x) and population index (y), the sporadic hourly rate (z).
    pub stream: [f32; 4],
}
pub fn meteors(setting: u32, days: f64) -> Meteors {
    static FORCED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    let forced = *FORCED.get_or_init(|| match std::env::var("HORIZON_METEORS").as_deref() {
        Ok("off" | "0") => Some(0),
        Ok("rare") => Some(1),
        Ok("frequent") => Some(2),
        _ => None,
    });
    let gain = match forced.unwrap_or(setting) {
        0 => {
            return Meteors {
                slot: 0.0,
                chance: 0.0,
                radiant: [0.0; 4],
                stream: [0.0; 4],
            };
        }
        1 => 8.0,
        _ => 100.0,
    };
    let (shower, zhr) = shower(days);
    let sporadic = SPORADIC * gain;
    let zhr = zhr * gain;
    // A slot a meteor at most: short enough for the busiest hour, and no shorter than the sky
    // pass can look back over (a storm's thousands an hour are slots of a third of a second).
    let rate = sporadic + zhr;
    let slot = (3600.0 / (rate * 1.25)).clamp(0.3, 60.0) as f32;
    let (ra, dec) = (shower.ra.to_radians(), shower.dec.to_radians());
    Meteors {
        slot,
        chance: ((rate * f64::from(slot)) / 3600.0).min(1.0) as f32,
        radiant: [
            (dec.cos() * ra.cos()) as f32,
            (dec.cos() * ra.sin()) as f32,
            dec.sin() as f32,
            zhr as f32,
        ],
        stream: [shower.speed, shower.index, sporadic as f32, 0.0],
    }
}

/// The sporadic meteors' hourly rate a dark sky shows the naked eye (to magnitude 6.5), about ten.
const SPORADIC: f64 = 10.0;

/// An annual meteor shower (the International Meteor Organization's working list): its peak's day
/// of the year, its zenithal hourly rate there (a perfect sky, the radiant overhead), the days its
/// activity falls to a half each side, its radiant (J2000, degrees), its meteors' speed (km/s) and
/// population index, and whether it is known for outbursts and storms.
struct Shower {
    peak: f64,
    zhr: f64,
    half: f64,
    ra: f64,
    dec: f64,
    speed: f32,
    index: f32,
    storms: bool,
}
const fn shower_of(
    peak: f64,
    zhr: f64,
    half: f64,
    ra: f64,
    dec: f64,
    speed: f32,
    index: f32,
    storms: bool,
) -> Shower {
    Shower {
        peak,
        zhr,
        half,
        ra,
        dec,
        speed,
        index,
        storms,
    }
}
const SHOWERS: [Shower; 10] = [
    shower_of(3.0, 110.0, 0.4, 230.0, 49.0, 41.0, 2.1, false), // Quadrantids
    shower_of(112.0, 18.0, 0.8, 271.0, 34.0, 49.0, 2.1, false), // Lyrids
    shower_of(126.0, 50.0, 3.0, 338.0, -1.0, 66.0, 2.4, false), // eta Aquariids
    shower_of(211.0, 25.0, 4.0, 340.0, -16.0, 41.0, 2.5, false), // Southern delta Aquariids
    shower_of(224.5, 100.0, 1.5, 48.0, 58.0, 59.0, 2.2, false), // Perseids
    shower_of(281.0, 10.0, 0.3, 262.0, 54.0, 20.0, 2.6, true), // Draconids
    shower_of(294.0, 20.0, 2.5, 95.0, 16.0, 66.0, 2.5, false), // Orionids
    shower_of(321.0, 15.0, 1.0, 152.0, 22.0, 71.0, 2.5, true), // Leonids
    shower_of(348.0, 150.0, 1.0, 112.0, 33.0, 35.0, 2.6, false), // Geminids
    shower_of(356.0, 10.0, 0.5, 217.0, 76.0, 33.0, 3.0, false), // Ursids
];

/// The strongest shower `days` since J2000 and its zenithal hourly rate: each one's rate falls off
/// from its peak (a half `half` days each side, exponentially, as the IMO's profiles do), and its
/// year may bring an outburst (one year in ten, five times its rate) or, for the Leonids and the
/// Draconids, a storm (one year in thirty, a hundred times: thousands an hour, as in 1966 and 2001).
fn shower(days: f64) -> (&'static Shower, f64) {
    let mut year = 2000 + (days / 365.2425).floor() as i64;
    let mut start = days_of(&format!("{year}-01-01")).unwrap_or(0.0);
    if days < start {
        year -= 1;
        start = days_of(&format!("{year}-01-01")).unwrap_or(0.0);
    }
    let day = days - start + 1.0;
    let mut best = (&SHOWERS[0], 0.0);
    for (i, shower) in SHOWERS.iter().enumerate() {
        // The distance to its peak around the year's turn too (the Quadrantids, the Ursids).
        let off = (day - shower.peak + 182.6).rem_euclid(365.2425) - 182.6;
        let mut zhr = shower.zhr * 0.5f64.powf(off.abs() / shower.half);
        let luck = year_hash(year, i);
        if shower.storms && luck < 1.0 / 30.0 {
            zhr *= 100.0;
        } else if luck < 0.1 {
            zhr *= 5.0;
        }
        if zhr > best.1 {
            best = (shower, zhr);
        }
    }
    // Below a sporadic meteor or two an hour a shower is not one.
    if best.1 < 1.0 {
        best.1 = 0.0;
    }
    best
}

/// A draw in [0, 1) for shower `i` in `year`.
fn year_hash(year: i64, i: usize) -> f64 {
    let mut h = (year as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}
pub fn buffer(device: &wgpu::Device) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("RT lighting's stars"),
        contents: CATALOGUE,
        usage: wgpu::BufferUsages::VERTEX,
    })
}

/// World directions (x east, y up, z south) from equatorial ones (z toward the celestial north
/// pole, x toward the vernal equinox) at latitude `latitude` degrees north (`tilt`, the sun's
/// path's lean) when the sun stands `sun_angle` degrees past noon, on today's date (the
/// environment's `HORIZON_SKY_DATE=YYYY-MM-DD` another, to see the sky of a season).
pub fn frame(sun_angle: f32, latitude: f32) -> Mat4 {
    let local_hours = (12.0 + f64::from(sun_angle) / 15.0).rem_euclid(24.0);
    sky_turn(days(), local_hours, latitude)
}

/// The sky's date in days since J2000: now, or `HORIZON_SKY_DATE`'s noon.
pub fn days() -> f64 {
    static DATE: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    let date = *DATE.get_or_init(|| {
        std::env::var("HORIZON_SKY_DATE")
            .ok()
            .and_then(|d| days_of(&d))
    });
    date.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64() / 86400.0)
            + 2_440_587.5
            - 2_451_545.0
    })
}

/// `YYYY-MM-DD`'s noon in days since J2000 (the Julian day of the Gregorian date).
pub(super) fn days_of(date: &str) -> Option<f64> {
    let mut parts = date.trim().splitn(3, '-').map(str::parse::<i64>);
    let (y, m, d) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let century = y.div_euclid(100);
    let leap = 2 - century + century.div_euclid(4);
    let julian = (365.25 * (y + 4716) as f64).floor()
        + (30.6001 * (m + 1) as f64).floor()
        + d as f64
        + leap as f64
        - 1524.5;
    Some(julian + 0.5 - 2_451_545.0)
}

/// The turn for `days` since J2000 (the date's midnight counts), `local_hours` of local solar
/// time, at `latitude` degrees.
fn sky_turn(days: f64, local_hours: f64, latitude: f32) -> Mat4 {
    // Local sidereal time: the Greenwich one at the day's 0h, plus the local time (the longitude
    // folded in, as local solar time already has it).
    let midnight = (days - 0.5).floor() + 0.5;
    let sidereal = (6.697_374_558 + 0.065_709_824_419_08 * midnight + 1.002_737_909 * local_hours)
        .rem_euclid(24.0);
    let theta = (sidereal / 24.0 * std::f64::consts::TAU) as f32;
    let (sin_t, cos_t) = theta.sin_cos();
    let phi = latitude.to_radians();
    let (sin_p, cos_p) = phi.sin_cos();
    // Hour angle frame: x' toward the meridian, y' west of it, z' the pole.
    let to_local = |x: f32, y: f32, z: f32| {
        let xp = cos_t * x + sin_t * y;
        let yp = -sin_t * x + cos_t * y;
        let zp = z;
        // East, up, south (Minecraft: north is -z).
        Vec4::new(yp, cos_p * xp + sin_p * zp, sin_p * xp - cos_p * zp, 0.0)
    };
    Mat4::from_cols(
        to_local(1.0, 0.0, 0.0),
        to_local(0.0, 1.0, 0.0),
        to_local(0.0, 0.0, 1.0),
        Vec4::W,
    )
}

#[cfg(test)]
mod tests {
    /// The Perseids peak on 12 August at about a hundred an hour, and are gone in March; the
    /// Geminids lead in mid-December.
    #[test]
    fn the_showers_come_on_their_dates() {
        let (perseids, zhr) = super::shower(super::days_of("2026-08-12").unwrap());
        assert_eq!(perseids.ra, 48.0);
        assert!((60.0..=600.0).contains(&zhr), "{zhr}");
        let (_, march) = super::shower(super::days_of("2026-03-10").unwrap());
        assert!(march < 1.0, "{march}");
        let (geminids, _) = super::shower(super::days_of("2025-12-14").unwrap());
        assert_eq!(geminids.ra, 112.0);
        let (quadrantids, _) = super::shower(super::days_of("2027-01-03").unwrap());
        assert_eq!(quadrantids.ra, 230.0);
    }

    /// A slot holds one meteor at most, even in a storm, and none when the setting is off.
    #[test]
    fn the_slots_hold_one_meteor_at_most() {
        for setting in 0..3 {
            for day in (0..800).map(|d| f64::from(d) * 0.5 + 9000.0) {
                let m = super::meteors(setting, day);
                assert!((0.0..=1.0).contains(&m.chance));
                assert!(setting == 0 || m.slot >= 0.3);
            }
        }
    }

    use super::*;
    use glam::Vec3;

    #[test]
    fn a_date_is_its_days_since_j2000() {
        assert_eq!(days_of("2000-01-01"), Some(0.0));
        assert_eq!(days_of("2026-07-15"), Some(9692.0));
        assert_eq!(days_of("2026-13-01"), None);
    }

    #[test]
    fn the_pole_stands_at_the_latitude_over_the_north() {
        let m = sky_turn(9000.3, 22.0, 43.5);
        let pole = m.transform_vector3(Vec3::Z);
        assert!((pole.y - 43.5f32.to_radians().sin()).abs() < 1e-4, "{pole}");
        assert!(pole.z < 0.0 && pole.x.abs() < 1e-4, "{pole}");
    }

    #[test]
    fn the_sky_turns_from_east_to_west() {
        // A star due south, half an hour later: west of south (toward -x).
        let now = sky_turn(9000.3, 22.0, 43.5);
        let star = now.inverse().transform_vector3(Vec3::new(0.0, 0.5, 0.866));
        let later = sky_turn(9000.3, 22.5, 43.5).transform_vector3(star);
        assert!(later.x < -0.05, "{later}");
    }

    #[test]
    fn the_catalogue_is_whole() {
        assert_eq!(COUNT, 9096);
        // Sirius (HR 2491, magnitude -1.46): the brightest.
        let brightest = CATALOGUE.chunks(8).map(|s| s[6]).min().expect("stars");
        assert!((13..=14).contains(&brightest), "{brightest}");
    }
}
