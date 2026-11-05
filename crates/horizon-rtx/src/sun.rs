//! RT lighting's sun pass as a ray tracing pipeline (docs/rtx-4090.md, phase 3's second brick):
//! `shaders/sun.rgen` traces, for each mask texel of a view, the trace kernel's sun ray for the
//! primary surface, with the hardware reordering the threads by what they hit (shader execution
//! reordering, `VK_NV_ray_tracing_invocation_reorder`) and `sun.rahit` doing the kernel's alpha
//! test; the kernel reads the result in place of tracing it (`HORIZON_RT_PIPELINE=1`). The
//! pipeline, its shader binding table and its descriptor sets are raw Vulkan on wgpu's device;
//! the resources are wgpu's, transitioned by wgpu to the states the pass reads them in
//! (`CommandEncoder::transition_resources`), bound through their raw handles.

use crate::{INSTANCE_SIZE, RawTlas};
use ash::vk;
use std::ffi::CStr;
use wgpu::hal::api::Vulkan;

/// Descriptor sets in flight at most: a frame's set is written while the frames before still
/// run, so each frame takes the next of a ring.
const FRAMES: usize = 4;
/// The arena pages bound (sun.rahit `arena0..7`, as the kernel's `ARENA_PAGES`).
pub const ARENA_PAGES: usize = 8;

/// What the pass reads of a frame (the trace kernel's own resources, `ShadowRays::run`).
pub struct SunInputs<'a> {
    /// The kernel's `Rt` uniform, written for this view already.
    pub uniform: &'a wgpu::Buffer,
    /// The rays' depth prepass of the view.
    pub depth: &'a wgpu::TextureView,
    /// The mask's size: a ray a texel.
    pub mask_size: [u32; 2],
    pub tlas: &'a RawTlas,
    pub arena: &'a [wgpu::Buffer; ARENA_PAGES],
    pub sprites: &'a wgpu::TextureView,
    pub instances: &'a wgpu::Buffer,
    pub dynamics: &'a wgpu::Buffer,
    pub atlas: &'a wgpu::TextureView,
    /// Sprite layers past the first array are taken as opaque (the kernel's `SPRITE_SPLIT`).
    pub sprite_split: u32,
}

/// Whether the pass is wanted: `HORIZON_RT_PIPELINE=1` (off by default while it is validated
/// against the kernel's own sun).
pub fn wanted() -> bool {
    std::env::var("HORIZON_RT_PIPELINE").is_ok_and(|v| v == "1")
}

/// The pipeline, its table and its sets, and the texture it writes.
pub struct SunPass {
    device: ash::Device,
    rt: ash::khr::ray_tracing_pipeline::Device,
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    sets: [vk::DescriptorSet; FRAMES],
    frame: usize,
    modules: [vk::ShaderModule; 3],
    sbt: Sbt,
    /// The sun's light through, a texel a ray, the mask's size (`Rgba16Float`).
    output: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
}

/// The shader binding table: one buffer, the ray generation, miss and hit group handles each at
/// a base-aligned offset.
struct Sbt {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    raygen: vk::StridedDeviceAddressRegionKHR,
    miss: vk::StridedDeviceAddressRegionKHR,
    hit: vk::StridedDeviceAddressRegionKHR,
}

// Plain Vulkan handles on a device wgpu keeps alive longer than the renderer that holds this.
unsafe impl Send for SunPass {}
unsafe impl Sync for SunPass {}

const RAYGEN: &[u8] = include_bytes!("../shaders/sun.rgen.spv");
const MISS: &[u8] = include_bytes!("../shaders/sun.rmiss.spv");
const ANY_HIT: &[u8] = include_bytes!("../shaders/sun.rahit.spv");

impl SunPass {
    /// The pass on `device`, where it is a Vulkan device with the ray tracing pipeline and the
    /// invocation reorder extensions (asked for at its creation: gfx.rs, horizon-hdr); `None`
    /// otherwise, with the reason logged.
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !wanted() {
            return None;
        }
        let (raw_device, instance, physical) = {
            // Safety: read and cloned (the handles and function tables); dropped before wgpu
            // takes the device back.
            let hal = unsafe { device.as_hal::<Vulkan>() }?;
            let extensions = hal.enabled_device_extensions();
            for needed in [
                ash::khr::ray_tracing_pipeline::NAME,
                ash::nv::ray_tracing_invocation_reorder::NAME,
            ] {
                if !extensions.contains(&needed) {
                    log::warn!(
                        "Horizon RTX: no sun pass, the device lacks {}",
                        needed.to_string_lossy()
                    );
                    return None;
                }
            }
            (
                hal.raw_device().clone(),
                hal.shared_instance().raw_instance().clone(),
                hal.raw_physical_device(),
            )
        };
        let rt = ash::khr::ray_tracing_pipeline::Device::new(&instance, &raw_device);
        // The table's alignments and the handles' size.
        let mut rt_props = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
        let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut rt_props);
        // Safety: a query on the adapter's own physical device.
        unsafe { instance.get_physical_device_properties2(physical, &mut props) };
        let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical) };

        // Safety for what follows: handles made on this device, destroyed in `drop`, used only
        // between wgpu's commands on its own command buffers.
        unsafe {
            let stages_all = vk::ShaderStageFlags::RAYGEN_KHR | vk::ShaderStageFlags::ANY_HIT_KHR;
            let binding = |b: u32, ty: vk::DescriptorType, stages: vk::ShaderStageFlags| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(b)
                    .descriptor_type(ty)
                    .descriptor_count(1)
                    .stage_flags(stages)
            };
            let mut bindings = vec![
                binding(
                    0,
                    vk::DescriptorType::UNIFORM_BUFFER,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
                binding(
                    1,
                    vk::DescriptorType::SAMPLED_IMAGE,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
                binding(
                    2,
                    vk::DescriptorType::STORAGE_IMAGE,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
                binding(
                    3,
                    vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
            ];
            for page in 0..ARENA_PAGES as u32 {
                bindings.push(binding(
                    4 + page,
                    vk::DescriptorType::STORAGE_BUFFER,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                ));
            }
            bindings.extend([
                binding(
                    12,
                    vk::DescriptorType::SAMPLED_IMAGE,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                ),
                binding(
                    13,
                    vk::DescriptorType::STORAGE_BUFFER,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                ),
                binding(
                    14,
                    vk::DescriptorType::STORAGE_BUFFER,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                ),
                binding(
                    15,
                    vk::DescriptorType::SAMPLED_IMAGE,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                ),
            ]);
            let _ = stages_all;
            let set_layout = raw_device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .ok()?;
            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::ANY_HIT_KHR)
                .offset(0)
                .size(4)];
            let set_layouts = [set_layout];
            let layout = raw_device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&push),
                    None,
                )
                .ok()?;
            let module = |bytes: &[u8]| -> Option<vk::ShaderModule> {
                let words = ash::util::read_spv(&mut std::io::Cursor::new(bytes)).ok()?;
                raw_device
                    .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
                    .ok()
            };
            let modules = [module(RAYGEN)?, module(MISS)?, module(ANY_HIT)?];
            let entry: &CStr = c"main";
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::RAYGEN_KHR)
                    .module(modules[0])
                    .name(entry),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::MISS_KHR)
                    .module(modules[1])
                    .name(entry),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::ANY_HIT_KHR)
                    .module(modules[2])
                    .name(entry),
            ];
            let general = |shader: u32| {
                vk::RayTracingShaderGroupCreateInfoKHR::default()
                    .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
                    .general_shader(shader)
                    .closest_hit_shader(vk::SHADER_UNUSED_KHR)
                    .any_hit_shader(vk::SHADER_UNUSED_KHR)
                    .intersection_shader(vk::SHADER_UNUSED_KHR)
            };
            let groups = [
                general(0),
                general(1),
                // The hit group: the any-hit alone (a shadow ray skips the closest hit).
                vk::RayTracingShaderGroupCreateInfoKHR::default()
                    .ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
                    .general_shader(vk::SHADER_UNUSED_KHR)
                    .closest_hit_shader(vk::SHADER_UNUSED_KHR)
                    .any_hit_shader(2)
                    .intersection_shader(vk::SHADER_UNUSED_KHR),
            ];
            let info = vk::RayTracingPipelineCreateInfoKHR::default()
                .stages(&stages)
                .groups(&groups)
                .max_pipeline_ray_recursion_depth(1)
                .layout(layout);
            let pipeline = match rt.create_ray_tracing_pipelines(
                vk::DeferredOperationKHR::null(),
                vk::PipelineCache::null(),
                &[info],
                None,
            ) {
                Ok(p) => p[0],
                Err((_, e)) => {
                    log::warn!("Horizon RTX: no sun pass, the pipeline failed: {e:?}");
                    return None;
                }
            };
            let sbt = Self::table(
                &raw_device,
                &rt,
                pipeline,
                &rt_props,
                &memory_properties,
                groups.len() as u32,
            )?;
            // The sets: one a frame of the ring.
            let count = FRAMES as u32;
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(3 * count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_IMAGE)
                    .descriptor_count(count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .descriptor_count(count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count((ARENA_PAGES as u32 + 2) * count),
            ];
            let pool = raw_device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(count)
                        .pool_sizes(&sizes),
                    None,
                )
                .ok()?;
            let layouts = [set_layout; FRAMES];
            let sets = raw_device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(pool)
                        .set_layouts(&layouts),
                )
                .ok()?;
            log::info!(
                "Horizon RTX: the sun pass as a ray tracing pipeline with shader execution reordering"
            );
            Some(Self {
                device: raw_device,
                rt,
                pipeline,
                layout,
                set_layout,
                pool,
                sets: sets.try_into().ok()?,
                frame: 0,
                modules,
                sbt,
                output: None,
            })
        }
    }

    /// The shader binding table of `groups` groups: the handles at base-aligned offsets in a
    /// host-visible buffer with a device address.
    unsafe fn table(
        device: &ash::Device,
        rt: &ash::khr::ray_tracing_pipeline::Device,
        pipeline: vk::Pipeline,
        props: &vk::PhysicalDeviceRayTracingPipelinePropertiesKHR,
        memory: &vk::PhysicalDeviceMemoryProperties,
        groups: u32,
    ) -> Option<Sbt> {
        let handle = props.shader_group_handle_size as u64;
        let aligned = handle.next_multiple_of(u64::from(props.shader_group_handle_alignment));
        let base = aligned.next_multiple_of(u64::from(props.shader_group_base_alignment));
        let size = base * u64::from(groups);
        let handles = unsafe {
            rt.get_ray_tracing_shader_group_handles(
                pipeline,
                0,
                groups,
                (handle * u64::from(groups)) as usize,
            )
        }
        .ok()?;
        let buffer = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(
                        vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
                            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                            | vk::BufferUsageFlags::TRANSFER_DST,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .ok()?;
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
        let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let type_index = (0..memory.memory_type_count).find(|&i| {
            requirements.memory_type_bits & (1 << i) != 0
                && memory.memory_types[i as usize]
                    .property_flags
                    .contains(wanted)
        })?;
        let mut flags =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(type_index)
                    .push_next(&mut flags),
                None,
            )
        }
        .ok()?;
        unsafe {
            device.bind_buffer_memory(buffer, memory, 0).ok()?;
            let mapped = device
                .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                .ok()? as *mut u8;
            std::ptr::write_bytes(mapped, 0, size as usize);
            for g in 0..groups as usize {
                std::ptr::copy_nonoverlapping(
                    handles.as_ptr().add(g * handle as usize),
                    mapped.add(g * base as usize),
                    handle as usize,
                );
            }
            device.unmap_memory(memory);
        }
        let address = unsafe {
            device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
        };
        let region = |group: u64| {
            vk::StridedDeviceAddressRegionKHR::default()
                .device_address(address + group * base)
                .stride(aligned)
                .size(aligned)
        };
        Some(Sbt {
            buffer,
            memory,
            raygen: region(0),
            miss: region(1),
            hit: region(2),
        })
    }

    /// The pass's output of `size` texels, made when the size changes.
    fn output(&mut self, device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        if let Some((_, view, have)) = &self.output
            && *have == size
        {
            return view.clone();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("RT lighting's sun pass"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.output = Some((texture, view.clone(), size));
        view
    }

    /// Records the pass for a view, after the kernel's uniform was written for it and before the
    /// kernel runs, and gives the texture the kernel reads (`sun_pass`). Raw commands can't share
    /// an encoder with wgpu's (wgpu records its own when the encoder finishes): `encoder` gets the
    /// resources' transitions and is finished into `split`, the pass follows in a command buffer
    /// of its own, and `encoder` is a new one for the rest of the frame. Submit `split` in order
    /// before it.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        split: &mut Vec<wgpu::CommandBuffer>,
        inputs: SunInputs<'_>,
    ) -> wgpu::TextureView {
        let view = self.output(device, inputs.mask_size);
        let output_texture = self.output.as_ref().expect("made above").0.clone();
        // wgpu puts the resources in the states the pass reads them in (and knows it did).
        fn storage(b: &wgpu::Buffer) -> wgpu::BufferTransition<&wgpu::Buffer> {
            wgpu::BufferTransition {
                buffer: b,
                state: wgpu::BufferUses::STORAGE_READ_ONLY,
            }
        }
        let buffers: Vec<wgpu::BufferTransition<&wgpu::Buffer>> = inputs
            .arena
            .iter()
            .map(storage)
            .chain([storage(inputs.instances), storage(inputs.dynamics)])
            .chain([wgpu::BufferTransition {
                buffer: inputs.uniform,
                state: wgpu::BufferUses::UNIFORM,
            }])
            .collect();
        fn sampled(t: &wgpu::Texture) -> wgpu::TextureTransition<&wgpu::Texture> {
            wgpu::TextureTransition {
                texture: t,
                selector: None,
                state: wgpu::TextureUses::RESOURCE,
            }
        }
        let textures = [
            sampled(inputs.depth.texture()),
            sampled(inputs.sprites.texture()),
            sampled(inputs.atlas.texture()),
            wgpu::TextureTransition {
                texture: &output_texture,
                selector: None,
                state: wgpu::TextureUses::STORAGE_READ_WRITE,
            },
        ];
        encoder.transition_resources(buffers.into_iter(), textures.into_iter());
        let before = std::mem::replace(
            encoder,
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Horizon frame, after the sun pass"),
            }),
        );
        split.push(before.finish());
        let mut raw = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Horizon RTX: the sun pass"),
        });

        let set = self.sets[self.frame % FRAMES];
        self.frame = self.frame.wrapping_add(1);
        // Safety: the resources' own handles, alive for the frame (wgpu holds them), the set of
        // this frame of the ring, written before the commands that read it are recorded.
        unsafe {
            let buffer = |b: &wgpu::Buffer| -> Option<vk::Buffer> {
                let hal = b.as_hal::<Vulkan>()?;
                Some(hal.raw_handle())
            };
            let image = |v: &wgpu::TextureView| -> Option<vk::ImageView> {
                let hal = v.as_hal::<Vulkan>()?;
                Some(hal.raw_handle())
            };
            let mut buffer_infos = Vec::new();
            let mut writes = Vec::new();
            let mut buffer_write = |binding: u32, ty: vk::DescriptorType, b: Option<vk::Buffer>| {
                if let Some(b) = b {
                    buffer_infos.push([vk::DescriptorBufferInfo {
                        buffer: b,
                        offset: 0,
                        range: vk::WHOLE_SIZE,
                    }]);
                    writes.push((binding, ty, buffer_infos.len() - 1));
                }
            };
            buffer_write(
                0,
                vk::DescriptorType::UNIFORM_BUFFER,
                buffer(inputs.uniform),
            );
            for (page, b) in inputs.arena.iter().enumerate() {
                buffer_write(
                    4 + page as u32,
                    vk::DescriptorType::STORAGE_BUFFER,
                    buffer(b),
                );
            }
            buffer_write(
                13,
                vk::DescriptorType::STORAGE_BUFFER,
                buffer(inputs.instances),
            );
            buffer_write(
                14,
                vk::DescriptorType::STORAGE_BUFFER,
                buffer(inputs.dynamics),
            );
            let mut descriptor_writes: Vec<vk::WriteDescriptorSet> = writes
                .iter()
                .map(|&(binding, ty, i)| {
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(binding)
                        .descriptor_type(ty)
                        .buffer_info(&buffer_infos[i])
                })
                .collect();
            let depth_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: image(inputs.depth).unwrap_or_default(),
                // A depth texture read by shaders (wgpu-hal `derive_image_layout`).
                image_layout: vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL,
            }];
            let output_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: image(&view).unwrap_or_default(),
                image_layout: vk::ImageLayout::GENERAL,
            }];
            let sprites_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: image(inputs.sprites).unwrap_or_default(),
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            let atlas_info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: image(inputs.atlas).unwrap_or_default(),
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            let tlas = [inputs.tlas.raw_handle()];
            let mut tlas_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                .acceleration_structures(&tlas);
            descriptor_writes.extend([
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&depth_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .image_info(&output_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(3)
                    .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .descriptor_count(1)
                    .push_next(&mut tlas_write),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(12)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&sprites_info),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(15)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&atlas_info),
            ]);
            self.device.update_descriptor_sets(&descriptor_writes, &[]);

            let [width, height] = inputs.mask_size;
            raw.as_hal_mut::<Vulkan, _, _>(|hal| {
                let Some(hal) = hal else {
                    return;
                };
                let cb = hal.raw_handle();
                self.device.cmd_bind_pipeline(
                    cb,
                    vk::PipelineBindPoint::RAY_TRACING_KHR,
                    self.pipeline,
                );
                self.device.cmd_bind_descriptor_sets(
                    cb,
                    vk::PipelineBindPoint::RAY_TRACING_KHR,
                    self.layout,
                    0,
                    &[set],
                    &[],
                );
                self.device.cmd_push_constants(
                    cb,
                    self.layout,
                    vk::ShaderStageFlags::ANY_HIT_KHR,
                    0,
                    &inputs.sprite_split.to_le_bytes(),
                );
                self.rt.cmd_trace_rays(
                    cb,
                    &self.sbt.raygen,
                    &self.sbt.miss,
                    &self.sbt.hit,
                    &vk::StridedDeviceAddressRegionKHR::default(),
                    width.max(1),
                    height.max(1),
                    1,
                );
                // The kernel (a compute pass) reads what the rays wrote; wgpu's own barrier for
                // the texture knows nothing of the ray tracing stage.
                let after = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ)];
                self.device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::PipelineStageFlags::COMPUTE_SHADER
                        | vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &after,
                    &[],
                    &[],
                );
            });
        }
        split.push(raw.finish());
        view
    }
}

impl Drop for SunPass {
    fn drop(&mut self) {
        // Safety: the handles are this pass's own; the device waits for the frames in flight
        // before they go (a teardown, not a frame).
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.layout, None);
            self.device.destroy_descriptor_pool(self.pool, None);
            self.device
                .destroy_descriptor_set_layout(self.set_layout, None);
            for module in self.modules {
                self.device.destroy_shader_module(module, None);
            }
            self.device.destroy_buffer(self.sbt.buffer, None);
            self.device.free_memory(self.sbt.memory, None);
        }
    }
}

/// The instance buffer's row size, re-exported for the pass's callers' checks.
pub const ROW: u64 = INSTANCE_SIZE;
