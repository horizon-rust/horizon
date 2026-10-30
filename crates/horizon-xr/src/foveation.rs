//! Foveated rendering without more views: variable rate shading (`VK_KHR_fragment_shading_rate`)
//! steered by the gaze. Each eye keeps one view at the headset's size; a small image beside it
//! (one texel per 16x16 pixels or so) says how coarsely each part of the eye is shaded: one
//! fragment a pixel where the eyes look, one for 2x2 pixels around that, one for 4x4 further
//! out. Only shading gets cheaper: the geometry and the CPU's work stay the same.
//!
//! wgpu has no shading rate, so `third_party/wgpu-hal` takes the image as an extra attachment
//! of every render pass of the eye's size (`Device::set_fragment_shading_rate`), with a twin
//! of each render pipeline for those passes. The images are made here with raw Vulkan, the
//! usage wgpu doesn't know, and filled through wgpu as an `R8Uint` texture.

use ash::vk;

use crate::{Error, err};

/// Within this angle of the gaze (degrees) every pixel is shaded: the Vision Pro's gaze moves
/// in steps of about 12 degrees, so the sharp area holds a step on each side.
const SHARP: f32 = 25.0;
/// Within this angle, one fragment for 2x2 pixels; beyond it, one for 4x4.
const HALF: f32 = 40.0;

/// The rates as the attachment encodes them: log2 of the width, shifted, and of the height.
const RATE_1X1: u8 = 0;
const RATE_2X2: u8 = (1 << 2) | 1;
const RATE_4X4: u8 = (2 << 2) | 2;

/// The texel size the attachment takes, when the device can shade at the attachment's rate:
/// 16x16 when allowed, else the smallest. `None` without the extension or its feature.
///
/// # Safety
///
/// `instance` and `physical` are valid, and the instance is Vulkan 1.1 or newer.
pub(crate) unsafe fn texel_size(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> Option<[u32; 2]> {
    let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }.ok()?;
    let name = ash::khr::fragment_shading_rate::NAME;
    if !extensions
        .iter()
        .any(|e| e.extension_name_as_c_str() == Ok(name))
    {
        return None;
    }
    let mut features = vk::PhysicalDeviceFragmentShadingRateFeaturesKHR::default();
    let mut all = vk::PhysicalDeviceFeatures2::default().push_next(&mut features);
    unsafe { instance.get_physical_device_features2(physical, &mut all) };
    if features.attachment_fragment_shading_rate == vk::FALSE {
        return None;
    }
    let mut props = vk::PhysicalDeviceFragmentShadingRatePropertiesKHR::default();
    let mut all = vk::PhysicalDeviceProperties2::default().push_next(&mut props);
    unsafe { instance.get_physical_device_properties2(physical, &mut all) };
    let (min, max) = (
        props.min_fragment_shading_rate_attachment_texel_size,
        props.max_fragment_shading_rate_attachment_texel_size,
    );
    let fits = |c: u32, lo: u32, hi: u32| (lo..=hi).contains(&c);
    Some(
        if fits(16, min.width, max.width) && fits(16, min.height, max.height) {
            [16, 16]
        } else {
            [min.width.max(1), min.height.max(1)]
        },
    )
}

/// One eye's rate image.
struct Rate {
    image: vk::Image,
    view: vk::ImageView,
    memory: vk::DeviceMemory,
    texture: wgpu::Texture,
    /// The gaze it was filled for (tenths of a degree), so it is filled again only when the
    /// gaze moves.
    filled: Option<[i32; 2]>,
}

/// Both eyes' rate images, for eyes of `size` pixels.
pub(crate) struct Rates {
    eyes: Vec<Rate>,
    /// The eye's size, and the images' (one texel per `texel` pixels, rounded up).
    size: [u32; 2],
    image_size: [u32; 2],
    texel: [u32; 2],
    device: ash::Device,
}

impl Rates {
    /// # Safety
    ///
    /// `device` is the Vulkan device with the fragment shading rate enabled for `texel`.
    pub(crate) unsafe fn new(
        device: &wgpu::Device,
        size: [u32; 2],
        texel: [u32; 2],
    ) -> Result<Self, Error> {
        let image_size = [size[0].div_ceil(texel[0]), size[1].div_ceil(texel[1])];
        let hal = unsafe { device.as_hal::<wgpu_hal::api::Vulkan>() }
            .ok_or_else(|| Error("not a Vulkan device".into()))?;
        let raw = hal.raw_device().clone();
        let instance = hal.shared_instance().raw_instance();
        let memory_types =
            unsafe { instance.get_physical_device_memory_properties(hal.raw_physical_device()) };
        let mut eyes = Vec::new();
        for _ in 0..2 {
            let image = unsafe {
                raw.create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(vk::Format::R8_UINT)
                        .extent(vk::Extent3D {
                            width: image_size[0],
                            height: image_size[1],
                            depth: 1,
                        })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(
                            vk::ImageUsageFlags::FRAGMENT_SHADING_RATE_ATTACHMENT_KHR
                                | vk::ImageUsageFlags::TRANSFER_DST,
                        )
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
            }
            .map_err(err("shading rate image"))?;
            let needs = unsafe { raw.get_image_memory_requirements(image) };
            let kind = (0..memory_types.memory_type_count)
                .find(|&i| {
                    needs.memory_type_bits & (1 << i) != 0
                        && memory_types.memory_types[i as usize]
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                })
                .ok_or_else(|| Error("no memory for the shading rate image".into()))?;
            let memory = unsafe {
                raw.allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
            }
            .map_err(err("shading rate memory"))?;
            unsafe { raw.bind_image_memory(image, memory, 0) }
                .map_err(err("shading rate memory"))?;
            let view = unsafe {
                raw.create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(vk::Format::R8_UINT)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        }),
                    None,
                )
            }
            .map_err(err("shading rate view"))?;
            let extent = wgpu::Extent3d {
                width: image_size[0],
                height: image_size[1],
                depth_or_array_layers: 1,
            };
            let texture = unsafe {
                let hal_texture = hal.texture_from_raw(
                    image,
                    &wgpu_hal::TextureDescriptor {
                        label: Some("XR shading rate"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::R8Uint,
                        usage: wgpu::TextureUses::COPY_DST,
                        memory_flags: wgpu_hal::MemoryFlags::empty(),
                        view_formats: Vec::new(),
                    },
                    // `Rates` frees the image, its view and its memory.
                    Some(Box::new(|| {})),
                    wgpu_hal::vulkan::TextureMemory::External,
                );
                device.create_texture_from_hal::<wgpu_hal::api::Vulkan>(
                    hal_texture,
                    &wgpu::TextureDescriptor {
                        label: Some("XR shading rate"),
                        size: extent,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::R8Uint,
                        usage: wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            };
            eyes.push(Rate {
                image,
                view,
                memory,
                texture,
                filled: None,
            });
        }
        Ok(Self {
            eyes,
            size,
            image_size,
            texel,
            device: raw,
        })
    }

    /// Fills each eye's image for where the eyes look: `gaze` in the head's frame (-z ahead),
    /// ahead without one (fixed foveation). Only when the gaze moved.
    pub(crate) fn update(&mut self, queue: &wgpu::Queue, fovs: [openxr::Fovf; 2], gaze: [f32; 3]) {
        let key = [gaze[0], gaze[1]].map(|c| (c * 1000.0).round() as i32);
        let [w, h] = self.image_size;
        for (eye, fov) in self.eyes.iter_mut().zip(fovs) {
            if eye.filled == Some(key) {
                continue;
            }
            let pixels = rates(fov, gaze, self.image_size, self.texel, self.size);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &eye.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            eye.filled = Some(key);
        }
    }

    /// The attachment for eye `eye`'s passes from now on (`None`: no more).
    pub(crate) fn attach(&self, device: &wgpu::Device, eye: Option<usize>) {
        let Some(hal) = (unsafe { device.as_hal::<wgpu_hal::api::Vulkan>() }) else {
            return;
        };
        let attachment = eye
            .and_then(|i| self.eyes.get(i))
            .map(|e| (e.view, self.size));
        // SAFETY: the views are `R8_UINT`, with the attachment usage, as big as the eyes
        // need, and live as long as `self`, which the device outlives; `update`'s writes
        // leave them in `TRANSFER_DST_OPTIMAL`.
        unsafe { hal.set_fragment_shading_rate(attachment) };
    }
}

impl Drop for Rates {
    fn drop(&mut self) {
        for eye in self.eyes.drain(..) {
            drop(eye.texture);
            // SAFETY: the device waited for its work when the session ended; nothing uses
            // the images any more.
            unsafe {
                let _ = self.device.device_wait_idle();
                self.device.destroy_image_view(eye.view, None);
                self.device.destroy_image(eye.image, None);
                self.device.free_memory(eye.memory, None);
            }
        }
    }
}

/// One eye's rate image, rows from the top: each texel's rate from the angle between the
/// gaze and the ray through its centre.
fn rates(
    fov: openxr::Fovf,
    gaze: [f32; 3],
    [w, h]: [u32; 2],
    texel: [u32; 2],
    size: [u32; 2],
) -> Vec<u8> {
    let (l, r) = (fov.angle_left.tan(), fov.angle_right.tan());
    let (d, u) = (fov.angle_down.tan(), fov.angle_up.tan());
    let length = gaze.iter().map(|c| c * c).sum::<f32>().sqrt().max(1e-6);
    let g = gaze.map(|c| c / length);
    let (sharp, half) = (SHARP.to_radians().cos(), HALF.to_radians().cos());
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            // The texel's centre, as a fraction of the eye's image (top left 0, 0).
            let fx = ((x as f32 + 0.5) * texel[0] as f32 / size[0] as f32).min(1.0);
            let fy = ((y as f32 + 0.5) * texel[1] as f32 / size[1] as f32).min(1.0);
            let ray = [l + fx * (r - l), u - fy * (u - d), -1.0];
            let n = ray.iter().map(|c| c * c).sum::<f32>().sqrt();
            let cos = (ray[0] * g[0] + ray[1] * g[1] + ray[2] * g[2]) / n;
            pixels.push(if cos >= sharp {
                RATE_1X1
            } else if cos >= half {
                RATE_2X2
            } else {
                RATE_4X4
            });
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gaze_is_sharp_the_middle_halved_and_the_edges_quartered() {
        let fov = openxr::Fovf {
            angle_left: -60f32.to_radians(),
            angle_right: 45f32.to_radians(),
            angle_up: 45f32.to_radians(),
            angle_down: -50f32.to_radians(),
        };
        let size = [4096, 3288];
        let image = [256, 206];
        let at = |pixels: &[u8], yaw: f32, pitch: f32| {
            // The texel the direction (yaw right, pitch up) falls in.
            let (tx, ty) = (yaw.to_radians().tan(), pitch.to_radians().tan());
            let fx = (tx - fov.angle_left.tan()) / (fov.angle_right.tan() - fov.angle_left.tan());
            let fy = (fov.angle_up.tan() - ty) / (fov.angle_up.tan() - fov.angle_down.tan());
            pixels[(fy * image[1] as f32) as usize * image[0] as usize
                + (fx * image[0] as f32) as usize]
        };
        let ahead = rates(fov, [0.0, 0.0, -1.0], image, [16, 16], size);
        assert_eq!(at(&ahead, 0.0, 0.0), RATE_1X1);
        assert_eq!(at(&ahead, 20.0, 0.0), RATE_1X1);
        assert_eq!(at(&ahead, 32.0, 0.0), RATE_2X2);
        assert_eq!(at(&ahead, -55.0, 0.0), RATE_4X4);
        // Looking 24 degrees left moves the sharp area there.
        let left = [
            -(24f32.to_radians().sin()),
            0.0,
            -(24f32.to_radians().cos()),
        ];
        let looked = rates(fov, left, image, [16, 16], size);
        assert_eq!(at(&looked, -40.0, 0.0), RATE_1X1);
        assert_eq!(at(&looked, 20.0, 0.0), RATE_4X4);
        assert_eq!(ahead.len(), (image[0] * image[1]) as usize);
    }
}
