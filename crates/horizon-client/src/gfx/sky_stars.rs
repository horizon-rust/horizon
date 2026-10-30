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

/// The shooting stars' rate (terrain.wgsl `rt_meteor`): the seconds of a slot that holds a meteor
/// with the given chance, by the Shooting Stars setting (`setting`: 0 none, 1 rare, a few a night,
/// a night lasting about eight minutes; 2 frequent, one every couple of seconds). The environment's
/// `HORIZON_METEORS` (`off`, `rare`, `frequent`) overrides it.
pub fn meteors(setting: u32) -> [f32; 2] {
    static FORCED: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    let forced = *FORCED.get_or_init(|| match std::env::var("HORIZON_METEORS").as_deref() {
        Ok("off" | "0") => Some(0),
        Ok("rare") => Some(1),
        Ok("frequent") => Some(2),
        _ => None,
    });
    match forced.unwrap_or(setting) {
        0 => [0.0, 0.0],
        1 => [30.0, 0.2],
        _ => [2.5, 1.0],
    }
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
/// path's lean) when the sun stands `sun_angle` degrees past noon, on today's date.
pub fn frame(sun_angle: f32, latitude: f32) -> Mat4 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() / 86400.0)
        + 2_440_587.5
        - 2_451_545.0;
    let local_hours = (12.0 + f64::from(sun_angle) / 15.0).rem_euclid(24.0);
    sky_turn(days, local_hours, latitude)
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
    use super::*;
    use glam::Vec3;

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
