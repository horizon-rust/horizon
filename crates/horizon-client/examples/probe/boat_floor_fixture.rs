//! The two ModelPart draw paths on the actual boat floor, for test_boat_floor_parity.py.
use horizon_client::mobs::{EntityModelDocument, anim::Poses, render, render_model};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let document = EntityModelDocument::load(Path::new(&args[0]))?;
    let root = document
        .model("minecraft:boat/oak#main")
        .ok_or("boat model")?;
    let bottom = root
        .children
        .iter()
        .find(|part| part.name == "bottom")
        .ok_or("boat bottom")?;
    let cases: Vec<Value> = serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let mut results = Vec::new();
    for case in cases {
        let camera_y = case["camera_y"].as_f64().ok_or("camera height")? as f32;
        let yaw = case["yaw"].as_f64().ok_or("yaw")? as f32;
        let base = glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.375 - camera_y, 0.0))
            * horizon_client::camera::rotation_y_degrees(180.0 - yaw)
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
            * horizon_client::camera::rotation_y_degrees(90.0);
        let mut animated = Vec::new();
        render::draw(
            bottom,
            base,
            &Poses::new(bottom),
            [0.0, 0.0, 1.0, 1.0],
            0xFFFF_FFFF,
            240,
            0,
            &mut animated,
        );
        let mut direct = Vec::new();
        render_model(
            bottom,
            base,
            &|part| part.pose,
            [0.0, 0.0, 1.0, 1.0],
            240,
            0,
            &mut direct,
        );
        results.push(json!({
            "animated": animated.iter().map(|vertex| vertex.pos).collect::<Vec<_>>(),
            "direct": direct.iter().map(|vertex| vertex.pos).collect::<Vec<_>>(),
        }));
    }
    fs::write(&args[2], serde_json::to_vec(&results)?)?;
    Ok(())
}
