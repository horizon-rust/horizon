//! RT lighting's sky textures (the Moon's map, the Milky Way's) and the sizes of the sun and the
//! moon they go with (gfx.rs may not grow).

use super::*;

/// RT lighting's moon map: NASA's LROC colour map of the Moon (CGI Moon Kit, Scientific
/// Visualization Studio, public domain), equirectangular, the near side in the middle, sRGB.
pub(super) fn moon_map(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    sky_map(
        device,
        queue,
        "RT lighting's moon",
        include_bytes!("moon_lroc.png"),
    )
}
/// An equirectangular sRGB map of the sky (the moon's face) as a texture.
pub(super) fn sky_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    png: &[u8],
) -> wgpu::TextureView {
    let (width, height, rgba) = crate::hud::decode_bytes(png).expect("the sky map decodes");
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
        )
        .create_view(&Default::default())
}
/// RT lighting's Milky Way: NASA's Deep Star Maps 2020 Milky Way layer (Scientific Visualization
/// Studio, public domain; its diffuse light without the stars, in celestial coordinates, centred
/// on right ascension 0h, right ascension increasing to the left). Its luminance at its full
/// 4096x2048 and its colour, which varies slowly, at 1024x512, put together here in linear light
/// into one sRGB map with its mips: the sky samples the level its pixel covers (terrain.wgsl
/// `rt_milky_way`), sharp up close and no shimmer far.
pub(super) fn milky_way_map(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let decode = |png: &[u8]| crate::hud::decode_bytes(png).expect("the Milky Way's maps decode");
    let (width, height, luminance) = decode(include_bytes!("milky_way_luminance.png"));
    let (cw, ch, colour) = decode(include_bytes!("milky_way_colour.png"));
    let to_linear: [f32; 256] = std::array::from_fn(|i| {
        let x = i as f32 / 255.0;
        if x <= 0.040_45 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    });
    let to_srgb = |x: f32| {
        let s = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
    };
    // The colour's tint (its light over its luminance), bilinear across right ascension's seam.
    let tint = |x: f32, y: f32| {
        let x = x - 0.5;
        let y = (y - 0.5).clamp(0.0, (ch - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| {
            let i = (i as i32).rem_euclid(cw as i32) as u32;
            let j = (j as u32).min(ch - 1);
            let p = &colour[((i + j * cw) * 4) as usize..][..3];
            Vec3::new(
                to_linear[p[0] as usize],
                to_linear[p[1] as usize],
                to_linear[p[2] as usize],
            )
        };
        let c = at(x0, y0)
            .lerp(at(x0 + 1.0, y0), fx)
            .lerp(at(x0, y0 + 1.0).lerp(at(x0 + 1.0, y0 + 1.0), fx), fy);
        c / c.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(1e-6)
    };
    let scale = (cw as f32 / width as f32, ch as f32 / height as f32);
    let mut level: Vec<Vec3> = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            let t = tint((x as f32 + 0.5) * scale.0, (y as f32 + 0.5) * scale.1);
            t * to_linear[luminance[(i * 4) as usize] as usize]
        })
        .collect();
    let (mut w, mut h) = (width, height);
    let mut bytes = Vec::new();
    let mut levels = 0;
    loop {
        bytes.extend(
            level
                .iter()
                .flat_map(|c| [to_srgb(c.x), to_srgb(c.y), to_srgb(c.z), 255]),
        );
        levels += 1;
        if w == 1 || h == 1 {
            break;
        }
        let (nw, nh) = (w / 2, h / 2);
        level = (0..nw * nh)
            .map(|i| {
                let (x, y) = (i % nw * 2, i / nw * 2);
                let at = |dx: u32, dy: u32| level[(x + dx + (y + dy) * w) as usize];
                (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25
            })
            .collect();
        (w, h) = (nw, nh);
    }
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("RT lighting's Milky Way"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &bytes,
        )
        .create_view(&Default::default())
}
/// The real sun's angular radius (0.53 degrees across).
pub(super) const SUN_RADIUS: f32 = 0.004_63;
/// The lunar regolith's reflectance by colour, relative to green (red, green, blue): its spectrum
/// rises toward the red, a B-V 0.27 redder than the sunlight it reflects; its luminance kept.
pub(super) const MOON_REFLECTANCE: [f32; 3] = [1.12, 1.0, 0.82];
/// The real moon's angular radius (0.52 degrees across, its mean).
pub(super) const MOON_RADIUS: f32 = 0.004_52;
