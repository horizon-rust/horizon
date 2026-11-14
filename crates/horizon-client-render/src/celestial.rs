//! The sun's, moon's and stars' path across the sky, as the renderer draws it.
//!
//! Vanilla's `SkyRenderer` puts a body on a frame turned Y -90°, then X by its angle (0 at noon):
//! the path runs east to west through the zenith. Horizon can tilt that path toward the south
//! (`Graphics::sun_path_tilt`), the way shader packs' `sunPathRotation` does, so that the noon sun
//! stands off the zenith and shadows have a direction. Sunrise and sunset stay due east and west.
//! It only moves what is drawn and lit: the time of day, the day/night cycle, beds and clocks
//! read `Sky`'s angles untouched.
use glam::{Mat4, Vec3};

/// The turn of the bodies' path: `tilt` degrees about the world's X axis (east-west), a positive
/// angle moving the noon sun toward +Z (south). An Iris/OptiFine pack's `sunPathRotation` is the
/// same turn with the other sign (`from_sun_path_rotation`).
pub fn tilt_matrix(tilt: f32) -> Mat4 {
    Mat4::from_rotation_x(tilt.to_radians())
}

/// `tilt` for a pack's `sunPathRotation`: Iris turns the bodies by `Z sunPathRotation` between
/// `Y -90` and `X angle` (`CelestialUniforms.getCelestialPosition`), which is the world's X axis
/// with the other sign: its usual -40 puts the noon sun in the south.
pub fn from_sun_path_rotation(rotation: f32) -> f32 {
    -rotation
}

/// The frame a body (or the star field) is drawn on: `angle` in degrees (`Sky::sun_angle`,
/// `moon_angle`, `star_angle`), its path tilted by `tilt` degrees.
pub fn frame(angle: f32, tilt: f32) -> Mat4 {
    tilt_matrix(tilt)
        * Mat4::from_rotation_y((-90.0f32).to_radians())
        * Mat4::from_rotation_x(angle.to_radians())
}

/// The unit world direction toward a body at `angle` degrees on the path tilted by `tilt`: the
/// light direction for the sun (by day) and the moon (by night).
pub fn direction(angle: f32, tilt: f32) -> Vec3 {
    frame(angle, tilt).transform_vector3(Vec3::Y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_noon_is_the_zenith_and_the_sun_sets_in_the_west() {
        assert!(direction(0.0, 0.0).abs_diff_eq(Vec3::Y, 1e-6));
        // Sunrise (angle -90, day time 0) in the east, sunset (90) in the west.
        assert!(direction(-90.0, 0.0).abs_diff_eq(Vec3::X, 1e-6));
        assert!(direction(90.0, 0.0).abs_diff_eq(-Vec3::X, 1e-6));
    }

    #[test]
    fn a_tilt_moves_noon_south_and_keeps_sunrise_and_sunset() {
        let noon = direction(0.0, 40.0);
        assert!((noon.y - 40f32.to_radians().cos()).abs() < 1e-6);
        assert!(
            (noon.z - 40f32.to_radians().sin()).abs() < 1e-6,
            "south is +Z"
        );
        assert!(direction(-90.0, 40.0).abs_diff_eq(Vec3::X, 1e-6));
        assert!(direction(90.0, 40.0).abs_diff_eq(-Vec3::X, 1e-6));
        // The moon, opposite the sun, culminates in the north at midnight.
        assert!(direction(180.0, 40.0).z < 0.0);
    }

    #[test]
    fn iris_sun_path_rotation_matches_its_own_matrices() {
        // CelestialUniforms: Y -90, Z sunPathRotation, X angle.
        for (rotation, angle) in [(-40.0f32, 0.0f32), (-40.0, 30.0), (25.0, -60.0)] {
            let iris = (Mat4::from_rotation_y((-90.0f32).to_radians())
                * Mat4::from_rotation_z(rotation.to_radians())
                * Mat4::from_rotation_x(angle.to_radians()))
            .transform_vector3(Vec3::Y);
            let ours = direction(angle, from_sun_path_rotation(rotation));
            assert!(
                ours.abs_diff_eq(iris, 1e-5),
                "{rotation} {angle}: {ours} vs {iris}"
            );
        }
    }
}
