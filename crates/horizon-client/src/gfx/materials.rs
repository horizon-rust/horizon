//! The LabPBR maps of the block sprites (`labpbr.rs`) on the GPU. Each kind of map is a few
//! texture arrays, one per map size (and per 2048 layers), so 16x maps stay 16x beside 64x
//! sprites and sprites without maps take no room, and an index buffer: by sprite layer, the
//! array and layer of its map (`array << 16 | layer`, or `NO_MAP`). A stage reads the index at
//! the sprite's layer and samples the map with the sprite's texture coordinate; a ray-traced
//! stage can look materials up the same way. Animated maps change frame with their sprite:
//! each class runs its own `SpriteAnimator` over the sprites' clips.
//!
//! Made only when the resource packs have maps; the Parallax setting reads the normal, the
//! specular and the subsurface maps (`gfx/parallax.rs`), and with them a buffer of which sprite
//! layers are thin (`gfx/transmission.rs`), for the light that comes through leaves and plants.

use super::{animate::SpriteAnimator, array_layers, mip_chain, upload::Uploads};
use crate::labpbr::{Map, MaterialPixels, NO_MAP};
use crate::sprites::SpriteAnimation;
use wgpu::util::DeviceExt;

/// One kind of map on the GPU.
pub struct MapArrays {
    /// The arrays, smallest maps first.
    pub textures: Vec<wgpu::Texture>,
    /// `array << 16 | layer` by sprite layer, or `NO_MAP` (`u32` each).
    pub index: wgpu::Buffer,
    animators: Vec<SpriteAnimator>,
}

impl MapArrays {
    /// A kind of map no sprite has: no arrays, and an index that says so for every layer.
    pub fn empty(device: &wgpu::Device) -> Self {
        Self {
            textures: Vec::new(),
            index: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Block map index"),
                // Reads past the end of a storage buffer give its last element.
                contents: bytemuck::cast_slice(&[NO_MAP]),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            animators: Vec::new(),
        }
    }
}

pub struct MaterialArrays {
    /// By [`Map`]: the normal, specular and subsurface maps.
    pub maps: [MapArrays; Map::COUNT],
    /// By sprite layer, 1 where the texture is thin (`gfx/transmission.rs`), else 0 (`u32` each).
    pub thin: wgpu::Buffer,
    /// Video memory the arrays take, in bytes.
    pub bytes: u64,
}

impl MaterialArrays {
    /// The arrays of `materials`, `split` layers per array at most; `animations` are the
    /// sprites' (their maps' frames go with them); `thin` is each sprite layer's thin flag.
    /// `compute` and `gl` as for the sprite arrays.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        materials: &MaterialPixels,
        split: u32,
        animations: &[SpriteAnimation],
        thin: &[u32],
        compute: bool,
        gl: bool,
    ) -> Self {
        let mut bytes = 0;
        let maps = Map::ALL.map(|map| {
            let set = &materials.maps[map as usize];
            let mut textures = Vec::new();
            let mut animators = Vec::new();
            // Each class's first array.
            let mut first_array = Vec::new();
            for class in &set.classes {
                first_array.push(textures.len() as u32);
                let layers = class.layers.len() as u32;
                let layer_bytes = (class.size * class.size * 4) as usize;
                let arrays: Vec<wgpu::Texture> = (0..layers.div_ceil(split))
                    .map(|array| {
                        let first = array * split;
                        let used = (layers - first).min(split);
                        let start = first as usize * layer_bytes;
                        let (mut mips, levels) =
                            mip_chain(&class.pixels[start..], class.size, &class.mips, first, used);
                        let count = array_layers(used, gl);
                        mips.resize(mips.len() / used as usize * count as usize, 0);
                        bytes += mips.len() as u64;
                        device.create_texture_with_data(
                            queue,
                            &wgpu::TextureDescriptor {
                                label: Some(match map {
                                    Map::Normal => "Block normal maps",
                                    Map::Specular => "Block specular maps",
                                    Map::Subsurface => "Block subsurface maps",
                                }),
                                size: wgpu::Extent3d {
                                    width: class.size,
                                    height: class.size,
                                    depth_or_array_layers: count,
                                },
                                mip_level_count: levels,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING
                                    | wgpu::TextureUsages::COPY_DST
                                    | wgpu::TextureUsages::COPY_SRC
                                    | wgpu::TextureUsages::STORAGE_BINDING,
                                view_formats: &[],
                            },
                            wgpu::util::TextureDataOrder::LayerMajor,
                            &mips,
                        )
                    })
                    .collect();
                // The class's animated maps, at their layer in the class.
                let animated: Vec<SpriteAnimation> = animations
                    .iter()
                    .filter_map(|a| {
                        let local = class.layers.iter().position(|&l| l == a.layer)?;
                        let mut animation = a.map(map)?;
                        animation.layer = local as u32;
                        Some(animation)
                    })
                    .collect();
                if !animated.is_empty() {
                    animators.push(SpriteAnimator::new(
                        device,
                        &animated,
                        class.size,
                        &class.mips,
                        &arrays,
                        split,
                        compute,
                    ));
                }
                textures.extend(arrays);
            }
            let index: Vec<u32> = set
                .index
                .iter()
                .map(|&entry| {
                    if entry == NO_MAP {
                        return NO_MAP;
                    }
                    let (class, local) = ((entry >> 16) as usize, entry & 0xffff);
                    ((first_array[class] + local / split) << 16) | (local % split)
                })
                .collect();
            let index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Block map index"),
                // A storage buffer may not be empty.
                contents: bytemuck::cast_slice(if index.is_empty() { &[NO_MAP] } else { &index }),
                usage: wgpu::BufferUsages::STORAGE,
            });
            MapArrays {
                textures,
                index,
                animators,
            }
        });
        let thin = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Thin sprite layers"),
            // A storage buffer may not be empty.
            contents: bytemuck::cast_slice(if thin.is_empty() { &[0] } else { thin }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Self { maps, thin, bytes }
    }

    /// Shows every animated map's frame at `tick`, as `SpriteAnimator::animate` does for the
    /// sprites at the same tick.
    pub fn animate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut Uploads,
        tick: u64,
    ) {
        for animator in self.maps.iter_mut().flat_map(|m| &mut m.animators) {
            animator.animate(encoder, uploads, tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labpbr::{MapClass, MapSet};

    /// Three sprite layers, the first and third with normal maps of 4 and 8 texels: each in
    /// the array of its size, at its layer there, every mip level made, and the index says
    /// where.
    #[test]
    fn maps_land_in_the_array_of_their_size() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            return;
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let texel = [200u8, 30, 255, 10];
        let class = |size: u32, layer: u32| MapClass {
            size,
            layers: vec![layer],
            pixels: texel.repeat((size * size) as usize),
            mips: vec![crate::mipmap::Settings {
                native_size: size,
                strategy: crate::mipmap::Strategy::Data,
                ..Default::default()
            }],
        };
        let mut materials = MaterialPixels::default();
        materials.maps[Map::Normal as usize] = MapSet {
            index: vec![0, NO_MAP, 1 << 16],
            classes: vec![class(4, 0), class(8, 2)],
        };
        let compute = SpriteAnimator::compute(&adapter, &device);
        let arrays =
            MaterialArrays::new(&device, &queue, &materials, 2048, &[], &[], compute, false);
        let normals = &arrays.maps[Map::Normal as usize];
        assert_eq!(normals.textures.len(), 2);
        assert_eq!(
            normals
                .textures
                .iter()
                .map(|t| (t.width(), t.mip_level_count()))
                .collect::<Vec<_>>(),
            [(4, 3), (8, 4)]
        );
        assert!(arrays.maps[Map::Specular as usize].textures.is_empty());
        assert!(arrays.maps[Map::Subsurface as usize].textures.is_empty());
        assert!(MapArrays::empty(&device).textures.is_empty());
        assert_eq!(arrays.thin.size(), 4);
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 + 12,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        // The 8x map's smallest level: its texel, averaged as data.
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &normals.textures[1],
                mip_level: 3,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range().unwrap();
        assert_eq!(data[..4], texel);
        assert!(arrays.bytes > 0);
    }
}
