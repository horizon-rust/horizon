//! RT lighting's naked-eye planets (terrain.wgsl `vs_rt_planet`): Mercury, Venus, Mars, Jupiter,
//! Saturn and, at the edge of sight, Uranus, where they stand among the stars on the sky's date
//! (gfx/sky_stars.rs `frame` turns them with the stars). Each planet's and the Earth's orbit from
//! its Keplerian elements and their drift a century (E. M. Standish's "Keplerian Elements for
//! Approximate Positions of the Major Planets", JPL, the fit for 1800 to 2050: a few arcminutes
//! off the full ephemeris), the planet seen from the Earth, its brightness from its distances and
//! its phase (Mallama & Hilton 2018), its colour the light its surface reflects.

use glam::DVec3;

/// The planets drawn.
pub const COUNT: u32 = PLANETS.len() as u32;
/// A planet an instance: its J2000 equatorial direction and its apparent magnitude, then its
/// colour (the brightest channel 1).
pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 32,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
};

/// An orbit's elements at J2000 and their change a Julian century: the semi-major axis (au), the
/// eccentricity, the inclination to the ecliptic, the mean longitude, the longitude of the
/// perihelion and that of the ascending node (degrees).
struct Orbit {
    at: [f64; 6],
    rate: [f64; 6],
}

/// Standish's table 1 (1800 to 2050), in the order above.
#[rustfmt::skip]
const MERCURY: Orbit = Orbit {
    at:   [0.387_099_27, 0.205_635_93, 7.004_979_02, 252.250_323_5, 77.457_796_28, 48.330_765_93],
    rate: [0.000_000_37, 0.000_019_06, -0.005_947_49, 149_472.674_111_75, 0.160_476_89, -0.125_340_81],
};
#[rustfmt::skip]
const VENUS: Orbit = Orbit {
    at:   [0.723_335_66, 0.006_776_72, 3.394_676_05, 181.979_099_5, 131.602_467_18, 76.679_842_55],
    rate: [0.000_003_9, -0.000_041_07, -0.000_788_9, 58_517.815_387_29, 0.002_683_29, -0.277_694_18],
};
/// The Earth's and the Moon's barycentre: the Earth's place within a few thousand kilometres.
#[rustfmt::skip]
const EARTH: Orbit = Orbit {
    at:   [1.000_002_61, 0.016_711_23, -0.000_015_31, 100.464_571_66, 102.937_681_93, 0.0],
    rate: [0.000_005_62, -0.000_043_92, -0.012_946_68, 35_999.372_449_81, 0.323_273_64, 0.0],
};
#[rustfmt::skip]
const MARS: Orbit = Orbit {
    at:   [1.523_710_34, 0.093_394_1, 1.849_691_42, -4.553_432_05, -23.943_629_59, 49.559_538_91],
    rate: [0.000_018_47, 0.000_078_82, -0.008_131_31, 19_140.302_684_99, 0.444_410_88, -0.292_573_43],
};
#[rustfmt::skip]
const JUPITER: Orbit = Orbit {
    at:   [5.202_887, 0.048_386_24, 1.304_396_95, 34.396_440_51, 14.728_479_83, 100.473_909_09],
    rate: [-0.000_116_07, -0.000_132_53, -0.001_837_14, 3_034.746_127_75, 0.212_526_68, 0.204_691_06],
};
#[rustfmt::skip]
const SATURN: Orbit = Orbit {
    at:   [9.536_675_94, 0.053_861_79, 2.485_991_87, 49.954_244_23, 92.598_878_31, 113.662_424_48],
    rate: [-0.001_250_6, -0.000_509_91, 0.001_936_09, 1_222.493_622_01, -0.418_972_16, -0.288_677_94],
};
#[rustfmt::skip]
const URANUS: Orbit = Orbit {
    at:   [19.189_164_64, 0.047_257_44, 0.772_637_83, 313.238_104_51, 170.954_276_3, 74.016_925_03],
    rate: [-0.001_961_76, -0.000_043_97, -0.002_429_39, 428.482_027_85, 0.408_052_81, 0.042_405_89],
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Planet {
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
}

/// The planets, their orbits and their colours: not a blackbody's but the sunlight their surfaces
/// give back, in light (Venus's clouds white, Mercury's dust a pinkish grey, Mars's iron oxides
/// orange-red, Jupiter's ammonia clouds cream, Saturn's a paler yellow, Uranus's methane a pale
/// cyan).
const PLANETS: [(Planet, Orbit, [f32; 3]); 6] = [
    (Planet::Mercury, MERCURY, [1.0, 0.84, 0.78]),
    (Planet::Venus, VENUS, [1.0, 0.98, 0.92]),
    (Planet::Mars, MARS, [1.0, 0.52, 0.3]),
    (Planet::Jupiter, JUPITER, [1.0, 0.92, 0.78]),
    (Planet::Saturn, SATURN, [1.0, 0.88, 0.64]),
    (Planet::Uranus, URANUS, [0.72, 0.92, 1.0]),
];

/// The obliquity of the ecliptic at J2000 (degrees): the turn from ecliptic to equatorial places.
const OBLIQUITY: f64 = 23.439_28;

/// Saturn's ring plane's pole, its J2000 right ascension and declination (degrees, the IAU's).
const SATURN_POLE: [f64; 2] = [40.589, 83.537];

pub fn buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("RT lighting's planets"),
        size: u64::from(COUNT) * 32,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// The planets' instances `days` after J2000 (`LAYOUT`).
pub fn instances(days: f64) -> [[f32; 8]; COUNT as usize] {
    let earth = heliocentric(&EARTH, days);
    PLANETS.map(|(planet, orbit, colour)| {
        let (direction, magnitude) = seen(planet, heliocentric(&orbit, days), earth);
        let d = direction.as_vec3();
        [
            d.x,
            d.y,
            d.z,
            magnitude as f32,
            colour[0],
            colour[1],
            colour[2],
            0.0,
        ]
    })
}

/// A planet's place from the Sun `days` after J2000, in au, J2000 equatorial (x toward the vernal
/// equinox, z the celestial north pole).
fn heliocentric(orbit: &Orbit, days: f64) -> DVec3 {
    let centuries = days / 36_525.0;
    let [a, e, inclination, mean_longitude, perihelion, node] =
        std::array::from_fn(|i| orbit.at[i] + orbit.rate[i] * centuries);
    // The argument of the perihelion (from the node) and the mean anomaly (from the perihelion).
    let argument = (perihelion - node).to_radians();
    let mean_anomaly = (mean_longitude - perihelion).to_radians();
    // Kepler's equation, M = E - e sin E, by Newton's steps from E = M (a few for these orbits,
    // none more eccentric than Mercury's 0.21).
    let mut anomaly = mean_anomaly + e * mean_anomaly.sin();
    for _ in 0..8 {
        anomaly -= (anomaly - e * anomaly.sin() - mean_anomaly) / (1.0 - e * anomaly.cos());
    }
    // In the orbit's plane, x toward the perihelion...
    let x = a * (anomaly.cos() - e);
    let y = a * (1.0 - e * e).sqrt() * anomaly.sin();
    // ...turned by the argument, tilted by the inclination about the node, the node turned to its
    // longitude: the ecliptic place.
    let (sin_w, cos_w) = argument.sin_cos();
    let (sin_n, cos_n) = node.to_radians().sin_cos();
    let (sin_i, cos_i) = inclination.to_radians().sin_cos();
    let ecliptic = DVec3::new(
        (cos_w * cos_n - sin_w * sin_n * cos_i) * x + (-sin_w * cos_n - cos_w * sin_n * cos_i) * y,
        (cos_w * sin_n + sin_w * cos_n * cos_i) * x + (-sin_w * sin_n + cos_w * cos_n * cos_i) * y,
        sin_w * sin_i * x + cos_w * sin_i * y,
    );
    // The ecliptic leans by the obliquity about the equinox's line.
    let (sin_e, cos_e) = OBLIQUITY.to_radians().sin_cos();
    DVec3::new(
        ecliptic.x,
        cos_e * ecliptic.y - sin_e * ecliptic.z,
        sin_e * ecliptic.y + cos_e * ecliptic.z,
    )
}

/// A planet at `at` seen from the Earth at `earth` (both from the Sun): its direction and its
/// apparent magnitude.
fn seen(planet: Planet, at: DVec3, earth: DVec3) -> (DVec3, f64) {
    let toward = at - earth;
    let (sun, distance) = (at.length(), toward.length());
    // The phase angle, at the planet between the Sun and the Earth: 0 full, 180 new.
    let phase = (at.dot(toward) / (sun * distance))
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees();
    let direction = toward / distance;
    (
        direction,
        5.0 * (sun * distance).log10() + reduced_magnitude(planet, phase, direction),
    )
}

/// A planet's magnitude at 1 au from the Sun and the Earth at the phase angle `phase` (degrees),
/// Mallama & Hilton's 2018 fits ("Computing apparent planetary magnitudes for The Astronomical
/// Almanac"): Mercury's and Venus's dim steeply as they turn their night sides to us (Venus's
/// clouds forward-scatter again in its thinnest crescents); the outer planets, never seen far
/// from full, barely. Saturn's rings add their light by how open they lie to us (`toward`, the
/// direction from the Earth); Mars's albedo markings and Uranus's tilt are left out (a tenth of a
/// magnitude).
fn reduced_magnitude(planet: Planet, phase: f64, toward: DVec3) -> f64 {
    let a = phase;
    match planet {
        Planet::Mercury => {
            let a = a.min(170.0);
            -0.613 + 6.328e-2 * a - 1.6336e-3 * a.powi(2) + 3.3644e-5 * a.powi(3)
                - 3.4265e-7 * a.powi(4)
                + 1.6893e-9 * a.powi(5)
                - 3.0334e-12 * a.powi(6)
        }
        Planet::Venus if a < 163.7 => {
            -4.384 - 1.044e-3 * a + 3.687e-4 * a.powi(2) - 2.814e-6 * a.powi(3)
                + 8.938e-9 * a.powi(4)
        }
        Planet::Venus => {
            let a = a.min(179.0);
            236.058_28 - 2.819_14 * a + 8.390_34e-3 * a.powi(2)
        }
        Planet::Mars => -1.601 + 2.267e-2 * a - 1.302e-4 * a.powi(2),
        Planet::Jupiter => -9.395 - 3.7e-4 * a + 6.16e-4 * a.powi(2),
        Planet::Saturn => {
            // The rings' tilt to us: the sine of the Earth's latitude over the ring plane.
            let (ra, dec) = (SATURN_POLE[0].to_radians(), SATURN_POLE[1].to_radians());
            let pole = DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin());
            let tilt = pole.dot(toward).abs();
            -8.914 - 1.825 * tilt + 2.6e-2 * a - 0.378 * tilt * (-2.25 * a).exp()
        }
        Planet::Uranus => -7.110 + 6.587e-3 * a + 1.045e-4 * a.powi(2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A planet's right ascension and declination (degrees) and magnitude on a date's noon (UT).
    fn sky(planet: Planet, date: &str) -> (f64, f64, f64) {
        let days = super::super::sky_stars::days_of(date).expect("date");
        let (_, orbit, _) = PLANETS.iter().find(|p| p.0 == planet).expect("planet");
        let (d, magnitude) = seen(
            planet,
            heliocentric(orbit, days),
            heliocentric(&EARTH, days),
        );
        let ra = d.y.atan2(d.x).to_degrees().rem_euclid(360.0);
        (ra, d.z.asin().to_degrees(), magnitude)
    }

    /// The angle (degrees) between two places on the sky.
    fn apart(a: (f64, f64), b: (f64, f64)) -> f64 {
        let unit = |(ra, dec): (f64, f64)| {
            let (ra, dec) = (ra.to_radians(), dec.to_radians());
            DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
        };
        unit(a).dot(unit(b)).clamp(-1.0, 1.0).acos().to_degrees()
    }

    // JPL Horizons' geocentric astrometric places (ICRF) and magnitudes at noon UT.
    const EPHEMERIS: [(Planet, &str, f64, f64, f64); 8] = [
        (Planet::Mercury, "2026-03-01", 350.754_71, -0.321_03, 2.210),
        // Venus at its greatest eastern elongation.
        (Planet::Venus, "2026-08-15", 186.961_10, -4.563_56, -4.434),
        (Planet::Venus, "2026-10-31", 202.361_04, -14.727_60, -4.133),
        // Mars, Jupiter, Saturn and Uranus at opposition.
        (Planet::Mars, "2025-01-16", 118.552_78, 25.217_08, -1.501),
        (Planet::Mars, "2026-10-31", 140.942_05, 16.949_22, 0.892),
        (Planet::Jupiter, "2026-01-10", 111.378_32, 22.246_26, -2.676),
        (Planet::Saturn, "2026-10-04", 11.102_98, 1.823_86, 0.331),
        (Planet::Uranus, "2026-11-21", 61.487_24, 20.700_40, 5.600),
    ];

    #[test]
    fn the_planets_stand_where_the_ephemeris_has_them() {
        for (planet, date, ra, dec, _) in EPHEMERIS {
            let got = sky(planet, date);
            let off = apart((got.0, got.1), (ra, dec));
            assert!(off < 0.5, "{planet:?} {date}: {got:?} is {off} degrees off");
        }
    }

    #[test]
    fn the_planets_shine_as_bright_as_the_ephemeris_has_them() {
        for (planet, date, _, _, magnitude) in EPHEMERIS {
            let got = sky(planet, date).2;
            assert!(
                (got - magnitude).abs() < 0.15,
                "{planet:?} {date}: magnitude {got}, not {magnitude}"
            );
        }
        // Venus a little past greatest elongation: -4.x; Jupiter at opposition: -2.x.
        assert!((-4.9..-4.0).contains(&sky(Planet::Venus, "2026-08-15").2));
        assert!((-3.0..-2.0).contains(&sky(Planet::Jupiter, "2026-01-10").2));
    }

    #[test]
    fn the_instances_are_unit_directions_and_their_colours() {
        for p in instances(9800.0) {
            let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-5, "{p:?}");
            assert!((-5.0..6.5).contains(&p[3]), "{p:?}");
            assert_eq!(p[4].max(p[5]).max(p[6]), 1.0, "{p:?}");
        }
    }
}
