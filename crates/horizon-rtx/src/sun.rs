//! RT lighting's sun pass as a ray tracing pipeline (docs/rtx-4090.md, phase 3's second brick):
//! `shaders/sun.rgen` traces, for each mask texel of a view, the trace kernel's sun ray for the
//! primary surface, with the hardware reordering the threads by what they hit (shader execution
//! reordering, `VK_NV_ray_tracing_invocation_reorder`) and `sun.rahit` doing the kernel's alpha
//! test; the kernel reads the result in place of tracing it (`HORIZON_RT_PIPELINE=1` or `sun`).
//! The same pipeline's second ray generation, `shaders/gi.rgen`, is the bounce pass
//! (docs/rtx-4090-handoff.md, "Next" 2): ReSTIR GI's new sample, the bounce ray and its hit's
//! colour and sun light shaded after the reordering (`HORIZON_RT_PIPELINE=gi`, or `all` for
//! both; `HORIZON_RT_SER=0` traces without reordering, to time what it gives). The
//! pipeline, its shader binding table and its descriptor sets are raw Vulkan on wgpu's device;
//! the resources are wgpu's, transitioned by wgpu to the states the pass reads them in
//! (`CommandEncoder::transition_resources`), bound through their raw handles.

use crate::{INSTANCE_SIZE, RawTlas};
use ash::vk::{self, Handle};
use std::ffi::CStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::hal::api::Vulkan;

/// Descriptor sets allocated at a time (a pool each).
const CHUNK: u32 = 8;
/// Sets past which the pass stops trusting `submitted` alone (see `SunPass::free_slot`).
const MAX_SLOTS: usize = 64;
/// Sets a view keeps written, its current one included: the view's inputs alternate between two
/// frames' textures (`last_mask`...), so two sets serve it with no rewrite, one more for slack.
const KEEP_PER_VIEW: usize = 3;
/// Runs after which a view not seen (an inset turned off) lets go of its sets and outputs.
const FORGET_RUNS: u64 = 256;
/// The arena pages bound (sun.rahit `arena0..7`, as the kernel's `ARENA_PAGES`).
pub const ARENA_PAGES: usize = 8;

/// What the pass reads of a frame (the trace kernel's own resources, `ShadowRays::run`).
pub struct SunInputs<'a> {
    /// The view: 0 the window (or its VR mirror), 1 and 2 the eyes, 3 and 4 VR's insets. A frame
    /// runs the pass once a view.
    pub view: usize,
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
    /// What the bounce pass reads of the kernel's own (`KernelReads`).
    pub kernel: KernelReads<'a>,
}

/// The trace kernel's resources the bounce pass reads (gi.rgen): the sky table, the view's last
/// masks (`last_mask`, `last_light_mask`, `last_direct`), its last reservoirs (`last_gi_a`,
/// `last_gi_b`) and the radiance cache.
pub struct KernelReads<'a> {
    pub sky: &'a wgpu::TextureView,
    pub last_mask: &'a wgpu::TextureView,
    pub last_light: &'a wgpu::TextureView,
    pub last_direct: &'a wgpu::TextureView,
    pub last_gi: [&'a wgpu::TextureView; 2],
    pub cache: &'a wgpu::Buffer,
}

pub use crate::{Kinds, PassViews};

/// Whether the threads are reordered by their hits (`HORIZON_RT_SER=0` turns it off).
fn reorder() -> bool {
    !std::env::var("HORIZON_RT_SER").is_ok_and(|v| v == "0")
}

/// The pipeline, its table and its sets, and the texture it writes.
pub struct SunPass {
    device: ash::Device,
    rt: ash::khr::ray_tracing_pipeline::Device,
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    /// The sets' pools, `CHUNK` sets each.
    pools: Vec<vk::DescriptorPool>,
    /// The descriptor sets, each with what it was last written with (`Slot`).
    slots: Vec<Slot>,
    /// Runs recorded (the serial of the last one).
    serial: u64,
    /// The last run whose command buffer was submitted (`submitted`).
    submitted: u64,
    /// The last run the GPU is known to have finished (set by `on_submitted_work_done`).
    done: Arc<AtomicU64>,
    /// Whether the `MAX_SLOTS` fallback was logged.
    warned: bool,
    modules: [vk::ShaderModule; 4],
    sbt: Sbt,
    /// Which ray generations run.
    kinds: Kinds,
    /// The push constants' flags (bit 0: reorder, bit 1: the wind, bits 2 and 3: the bounced
    /// light's turns as the kernel's `GI_TURN_BITS`, bit 4: the kernel's `SKIP` 16, no resting).
    flags: u32,
    /// The outputs, a texel a ray, the mask's size: the sun's light through (`Rgba16Float`), the
    /// bounce pass's hit and what it is (`Rgba32Float`) and its face, colour and sun light
    /// (`Rgba32Uint`). All are bound whichever generations run (the pipeline holds both). One set
    /// a size: VR's eyes share theirs, its insets and the window mirror have their own, so no view
    /// makes another's again (they were remade up to five times a frame).
    outputs: Vec<([u32; 2], Outputs)>,
    /// The views seen lately: their index (0 the window, 1 and 2 the eyes, 3 and 4 the insets),
    /// mask size and last run; the outputs of a size no view has go.
    views: Vec<(usize, [u32; 2], u64)>,
}

/// A descriptor set and what it was written with: the raw handles (`key`, empty when unwritten)
/// and clones of the wgpu objects behind them (`held`). Vulkan forbids updating a set a pending
/// command buffer uses (VUID-vkUpdateDescriptorSets-None-03047, without UPDATE_AFTER_BIND) and
/// destroying an image view or buffer such a command buffer reads through a descriptor
/// (VUID-vkDestroyImageView-imageView-01026, VUID-vkDestroyBuffer-buffer-00922): a set is
/// written again only once the GPU finished the last run that bound it (`last <= done`), and
/// until then it keeps what it points at alive (wgpu doesn't know of the raw command buffer's
/// reads). Holding them also keeps their handles from being recycled, so equal keys mean the
/// same objects and a set already written for them is bound again without a rewrite.
struct Slot {
    set: vk::DescriptorSet,
    key: Vec<u64>,
    held: (Vec<wgpu::Buffer>, Vec<wgpu::TextureView>),
    /// The view and mask size it was last bound for, and that run's serial (0: never).
    view: usize,
    size: [u32; 2],
    last: u64,
}

impl Slot {
    fn clear(&mut self) {
        self.key.clear();
        self.held = (Vec::new(), Vec::new());
    }
}

struct Outputs {
    sun: (wgpu::Texture, wgpu::TextureView),
    gi_a: (wgpu::Texture, wgpu::TextureView),
    gi_b: (wgpu::Texture, wgpu::TextureView),
}

/// The shader binding table: one buffer, the ray generation, miss and hit group handles each at
/// a base-aligned offset.
struct Sbt {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    raygen: vk::StridedDeviceAddressRegionKHR,
    miss: vk::StridedDeviceAddressRegionKHR,
    hit: vk::StridedDeviceAddressRegionKHR,
    /// The bounce pass's ray generation.
    raygen_gi: vk::StridedDeviceAddressRegionKHR,
}

// Plain Vulkan handles on a device wgpu keeps alive longer than the renderer that holds this.
unsafe impl Send for SunPass {}
unsafe impl Sync for SunPass {}

const RAYGEN: &[u8] = include_bytes!("../shaders/sun.rgen.spv");
const MISS: &[u8] = include_bytes!("../shaders/sun.rmiss.spv");
const ANY_HIT: &[u8] = include_bytes!("../shaders/sun.rahit.spv");
const RAYGEN_GI: &[u8] = include_bytes!("../shaders/gi.rgen.spv");

impl SunPass {
    /// The pass on `device`, where it is a Vulkan device with the ray tracing pipeline and the
    /// invocation reorder extensions (asked for at its creation: gfx.rs, horizon-hdr); `None`
    /// otherwise, with the reason logged.
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        let kinds = Kinds::wanted();
        if !kinds.any() {
            return None;
        }
        let (micromaps, raw_device, instance, physical) = {
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
            // The sections' structures may carry opacity micromaps (`omm`): a ray tracing
            // pipeline must say it reads them.
            let micromaps = extensions.contains(&ash::ext::opacity_micromap::NAME);
            (
                micromaps,
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
            // The bounce pass reads the hit's quad, sprite and entity in its ray generation.
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
                    stages_all,
                ));
            }
            bindings.extend([
                binding(12, vk::DescriptorType::SAMPLED_IMAGE, stages_all),
                binding(13, vk::DescriptorType::STORAGE_BUFFER, stages_all),
                binding(14, vk::DescriptorType::STORAGE_BUFFER, stages_all),
                binding(15, vk::DescriptorType::SAMPLED_IMAGE, stages_all),
                binding(
                    16,
                    vk::DescriptorType::STORAGE_IMAGE,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
                binding(
                    17,
                    vk::DescriptorType::STORAGE_IMAGE,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ),
            ]);
            // The kernel's own the bounce pass reads: six textures, the radiance cache.
            for b in 18..24 {
                bindings.push(binding(
                    b,
                    vk::DescriptorType::SAMPLED_IMAGE,
                    vk::ShaderStageFlags::RAYGEN_KHR,
                ));
            }
            bindings.push(binding(
                24,
                vk::DescriptorType::STORAGE_BUFFER,
                vk::ShaderStageFlags::RAYGEN_KHR,
            ));
            let set_layout = raw_device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .ok()?;
            // The sprite split (both stages) and the flags (the bounce pass).
            let push = [vk::PushConstantRange::default()
                .stage_flags(stages_all)
                .offset(0)
                .size(8)];
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
            let modules = [
                module(RAYGEN)?,
                module(MISS)?,
                module(ANY_HIT)?,
                module(RAYGEN_GI)?,
            ];
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
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::RAYGEN_KHR)
                    .module(modules[3])
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
                // The bounce pass's ray generation (its rays use the same miss and hit group).
                general(3),
            ];
            let info = vk::RayTracingPipelineCreateInfoKHR::default()
                .stages(&stages)
                .groups(&groups)
                .max_pipeline_ray_recursion_depth(1)
                .flags(if micromaps {
                    vk::PipelineCreateFlags::RAY_TRACING_OPACITY_MICROMAP_EXT
                } else {
                    vk::PipelineCreateFlags::empty()
                })
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
            let mut pass = Self {
                device: raw_device,
                rt,
                pipeline,
                layout,
                set_layout,
                pools: Vec::new(),
                slots: Vec::new(),
                serial: 0,
                submitted: 0,
                done: Arc::new(AtomicU64::new(0)),
                warned: false,
                modules,
                sbt,
                kinds,
                flags: 0,
                outputs: Vec::new(),
                views: Vec::new(),
            };
            if !pass.grow() {
                log::warn!("Horizon RTX: no sun pass, its descriptor sets failed");
                return None;
            }
            // Bit 1: the wind sways the foliage (HORIZON_RT_SKIP bit 4096 clear), so sun.rahit
            // ignores a cutout quad met right at the ray's start.
            pass.flags = u32::from(reorder()) | if wind_skipped() { 0 } else { 2 };
            // Bits 2 and 3, bit 4: gi.rgen rests its tiles on the kernel's own turns.
            pass.flags |= gi_turn_bits() << 2 | if skipped(16) { 16 } else { 0 };
            log::info!(
                "Horizon RTX: a ray tracing pipeline for {}{}{}, shader execution reordering {}",
                if kinds.sun { "the sun pass" } else { "" },
                if kinds.sun && kinds.gi { " and " } else { "" },
                if kinds.gi { "the bounce pass" } else { "" },
                if pass.flags & 1 != 0 { "on" } else { "off" },
            );
            Some(pass)
        }
    }

    /// `CHUNK` more descriptor sets, in a pool of their own; false if Vulkan refused.
    fn grow(&mut self) -> bool {
        // Safety: a pool and its sets on this pass's device, destroyed in `drop`.
        unsafe {
            let count = CHUNK;
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::UNIFORM_BUFFER)
                    .descriptor_count(count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(9 * count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_IMAGE)
                    .descriptor_count(3 * count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .descriptor_count(count),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count((ARENA_PAGES as u32 + 3) * count),
            ];
            let Ok(pool) = self.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(count)
                    .pool_sizes(&sizes),
                None,
            ) else {
                return false;
            };
            let layouts = [self.set_layout; CHUNK as usize];
            let Ok(sets) = self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&layouts),
            ) else {
                self.device.destroy_descriptor_pool(pool, None);
                return false;
            };
            self.pools.push(pool);
            self.slots.extend(sets.into_iter().map(|set| Slot {
                set,
                key: Vec::new(),
                held: (Vec::new(), Vec::new()),
                view: 0,
                size: [0, 0],
                last: 0,
            }));
            true
        }
    }

    /// Tells the pass that every run recorded so far was just submitted (call right after the
    /// `queue.submit` that took their command buffers): their sets and what they hold are freed
    /// for reuse once the GPU finishes that submission.
    pub fn submitted(&mut self, queue: &wgpu::Queue) {
        if self.serial > self.submitted {
            self.submitted = self.serial;
            let (done, upto) = (self.done.clone(), self.serial);
            queue.on_submitted_work_done(move || {
                done.fetch_max(upto, Ordering::AcqRel);
            });
        }
    }

    /// A set no pending command buffer uses, the one unused longest; more are made when all are
    /// in use. Past `MAX_SLOTS` (`submitted` not called, or the GPU far behind), a set last bound
    /// `MAX_SLOTS` runs ago counts as finished: a dozen frames or more, where wgpu's surface
    /// keeps two in flight (`desired_maximum_frame_latency`) and OpenXR's `xrWaitFrame` paces the
    /// headset's.
    fn free_slot(&mut self, done: u64, now: u64) -> usize {
        let pick = |slots: &[Slot], done: u64| {
            slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.last <= done)
                .min_by_key(|(_, s)| (!s.key.is_empty(), s.last))
                .map(|(i, _)| i)
        };
        if let Some(i) = pick(&self.slots, done) {
            return i;
        }
        if self.slots.len() >= MAX_SLOTS {
            if !self.warned {
                self.warned = true;
                log::warn!(
                    "Horizon RTX: the sun pass's {} descriptor sets all wait on the GPU; reusing the oldest",
                    self.slots.len()
                );
            }
            if let Some(i) = pick(&self.slots, now.saturating_sub(MAX_SLOTS as u64)) {
                return i;
            }
        }
        let before = self.slots.len();
        if self.grow() {
            return before;
        }
        // Vulkan gave no more sets: the oldest, as the fallback above.
        self.slots
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.last)
            .map_or(0, |(i, _)| i)
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
            raygen_gi: region(3),
        })
    }

    /// The passes' outputs of `size` texels for `view`, made the first time a view has that size;
    /// those of a size no view has any more go (the sets that bound them hold them until the GPU
    /// is done, `Slot`).
    fn outputs(&mut self, device: &wgpu::Device, view: usize, size: [u32; 2]) -> &Outputs {
        let now = self.serial;
        match self.views.iter_mut().find(|v| v.0 == view) {
            Some(v) => {
                v.1 = size;
                v.2 = now;
            }
            None => self.views.push((view, size, now)),
        }
        self.views.retain(|v| v.2 + FORGET_RUNS >= now);
        let views = &self.views;
        self.outputs
            .retain(|(have, _)| views.iter().any(|v| v.1 == *have));
        if !self.outputs.iter().any(|(have, _)| *have == size) {
            let make = |label, format| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0].max(1),
                        height: size[1].max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view)
            };
            self.outputs.push((
                size,
                Outputs {
                    sun: make("RT lighting's sun pass", wgpu::TextureFormat::Rgba16Float),
                    gi_a: make(
                        "RT lighting's bounce pass, hits",
                        wgpu::TextureFormat::Rgba32Float,
                    ),
                    gi_b: make(
                        "RT lighting's bounce pass, surfaces",
                        wgpu::TextureFormat::Rgba32Uint,
                    ),
                },
            ));
        }
        &self
            .outputs
            .iter()
            .find(|(have, _)| *have == size)
            .expect("made above")
            .1
    }

    /// Which ray generations run (the kernel's `rt.mode.w` bits).
    pub fn kinds(&self) -> Kinds {
        self.kinds
    }

    /// Records the passes for a view, after the kernel's uniform was written for it and before
    /// the kernel runs, and gives the textures the kernel reads (`sun_pass`, `gi_pass_a`,
    /// `gi_pass_b`) for the passes that ran. Raw commands can't share
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
    ) -> PassViews {
        self.serial += 1;
        let now = self.serial;
        let outputs = self.outputs(device, inputs.view, inputs.mask_size);
        let (sun_texture, view) = outputs.sun.clone();
        let (gi_a_texture, gi_a) = outputs.gi_a.clone();
        let (gi_b_texture, gi_b) = outputs.gi_b.clone();
        let written = |texture| wgpu::TextureTransition {
            texture,
            selector: None,
            state: wgpu::TextureUses::STORAGE_READ_WRITE,
        };
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
            .chain([
                storage(inputs.instances),
                storage(inputs.dynamics),
                storage(inputs.kernel.cache),
            ])
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
            sampled(inputs.kernel.sky.texture()),
            sampled(inputs.kernel.last_mask.texture()),
            sampled(inputs.kernel.last_light.texture()),
            sampled(inputs.kernel.last_direct.texture()),
            sampled(inputs.kernel.last_gi[0].texture()),
            sampled(inputs.kernel.last_gi[1].texture()),
            written(&sun_texture),
            written(&gi_a_texture),
            written(&gi_b_texture),
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

        // Safety: the resources' own handles, read while they live.
        let buffer = |b: &wgpu::Buffer| -> Option<vk::Buffer> {
            let hal = unsafe { b.as_hal::<Vulkan>() }?;
            Some(unsafe { hal.raw_handle() })
        };
        let image = |v: &wgpu::TextureView| -> Option<vk::ImageView> {
            let hal = unsafe { v.as_hal::<Vulkan>() }?;
            Some(unsafe { hal.raw_handle() })
        };
        // What the set points at, in a fixed order: its key and the objects it holds.
        let kernel = &inputs.kernel;
        let held_buffers: Vec<wgpu::Buffer> = [inputs.uniform]
            .into_iter()
            .chain(inputs.arena.iter())
            .chain([inputs.instances, inputs.dynamics, kernel.cache])
            .cloned()
            .collect();
        let held_views: Vec<wgpu::TextureView> = [
            inputs.depth,
            &view,
            &gi_a,
            &gi_b,
            inputs.sprites,
            inputs.atlas,
            kernel.sky,
            kernel.last_mask,
            kernel.last_light,
            kernel.last_direct,
            kernel.last_gi[0],
            kernel.last_gi[1],
        ]
        .into_iter()
        .cloned()
        .collect();
        let key: Vec<u64> = [inputs.tlas.id(), inputs.tlas.raw_handle().as_raw()]
            .into_iter()
            .chain(
                held_buffers
                    .iter()
                    .map(|b| buffer(b).unwrap_or_default().as_raw()),
            )
            .chain(
                held_views
                    .iter()
                    .map(|v| image(v).unwrap_or_default().as_raw()),
            )
            .collect();
        let done = self.done.load(Ordering::Acquire);
        // A set already written with these: bound again as it is (a set may be bound by any
        // number of pending command buffers, as long as none updates it).
        let (index, write) = match self.slots.iter().position(|s| s.key == key) {
            Some(i) => (i, false),
            None => (self.free_slot(done, now), true),
        };
        {
            let slot = &mut self.slots[index];
            if write {
                slot.key = key;
                slot.held = (held_buffers, held_views);
            }
            slot.view = inputs.view;
            slot.size = inputs.mask_size;
            slot.last = now;
        }
        // The view's older sets, the GPU done with them, let go of what they hold: past the
        // `KEEP_PER_VIEW` latest, or of another size (a resize), or of a view gone.
        let mut mine: Vec<usize> = (0..self.slots.len())
            .filter(|&i| {
                i != index && self.slots[i].view == inputs.view && !self.slots[i].key.is_empty()
            })
            .collect();
        mine.sort_by_key(|&i| std::cmp::Reverse(self.slots[i].last));
        for (rank, &i) in mine.iter().enumerate() {
            let slot = &mut self.slots[i];
            if slot.last <= done && (rank + 1 >= KEEP_PER_VIEW || slot.size != inputs.mask_size) {
                slot.clear();
            }
        }
        for slot in &mut self.slots {
            if !slot.key.is_empty() && slot.last <= done && slot.last + FORGET_RUNS < now {
                slot.clear();
            }
        }
        let set = self.slots[index].set;
        // The set written only when no pending command buffer uses it (`free_slot`), before the
        // commands that read it are recorded.
        if write {
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
            buffer_write(
                24,
                vk::DescriptorType::STORAGE_BUFFER,
                buffer(inputs.kernel.cache),
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
            let gi_infos = [&gi_a, &gi_b].map(|v| {
                [vk::DescriptorImageInfo {
                    sampler: vk::Sampler::null(),
                    image_view: image(v).unwrap_or_default(),
                    image_layout: vk::ImageLayout::GENERAL,
                }]
            });
            let kernel = &inputs.kernel;
            let read_infos = [
                kernel.sky,
                kernel.last_mask,
                kernel.last_light,
                kernel.last_direct,
                kernel.last_gi[0],
                kernel.last_gi[1],
            ]
            .map(|v| {
                [vk::DescriptorImageInfo {
                    sampler: vk::Sampler::null(),
                    image_view: image(v).unwrap_or_default(),
                    image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                }]
            });
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
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(16)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .image_info(&gi_infos[0]),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(17)
                    .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                    .image_info(&gi_infos[1]),
            ]);
            descriptor_writes.extend((18..).zip(&read_infos).map(|(binding, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(binding)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(info)
            }));
            // Safety: the resources' own handles, alive while the set holds them (`Slot`).
            unsafe { self.device.update_descriptor_sets(&descriptor_writes, &[]) };
        }
        // Safety: the set's resources alive while it holds them (`Slot`), the pipeline and table
        // this pass's own, recorded on the encoder's own command buffer.
        unsafe {
            let [width, height] = inputs.mask_size;
            let kinds = self.kinds;
            let push: [u32; 2] = [inputs.sprite_split, self.flags];
            raw.as_hal_mut::<Vulkan, _, _>(|hal| {
                let Some(hal) = hal else {
                    return;
                };
                let cb = hal.raw_handle();
                // wgpu's transitions above (and its barriers after the commands that wrote
                // these: the depth prepass, the uniform's copy, the kernel's last reads of the
                // outputs) wait in VERTEX|FRAGMENT|COMPUTE_SHADER only (wgpu-hal conv.rs
                // `map_texture_usage_to_barrier`), so nothing orders them, nor their layout
                // transitions, before the ray tracing stage. This barrier's first scope takes
                // those stages in, which chains them (spec 7.1, execution dependency chains, and
                // 7.6.1's image layout transitions) to the rays' reads and writes: read after
                // write for the inputs, write after read for the outputs the last view's kernel
                // read.
                let before = [vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::SHADER_READ
                            | vk::AccessFlags::SHADER_WRITE
                            | vk::AccessFlags::UNIFORM_READ
                            | vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR,
                    )];
                self.device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                    vk::DependencyFlags::empty(),
                    &before,
                    &[],
                    &[],
                );
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
                    vk::ShaderStageFlags::RAYGEN_KHR | vk::ShaderStageFlags::ANY_HIT_KHR,
                    0,
                    bytemuck::cast_slice(&push),
                );
                // The two generations write textures of their own: no barrier between them.
                for (on, raygen) in [
                    (kinds.sun, &self.sbt.raygen),
                    (kinds.gi, &self.sbt.raygen_gi),
                ] {
                    if on {
                        self.rt.cmd_trace_rays(
                            cb,
                            raygen,
                            &self.sbt.miss,
                            &self.sbt.hit,
                            &vk::StridedDeviceAddressRegionKHR::default(),
                            width.max(1),
                            height.max(1),
                            1,
                        );
                    }
                }
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
        PassViews {
            sun: self.kinds.sun.then_some(view),
            gi: self.kinds.gi.then_some([gi_a, gi_b]),
        }
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
            for pool in self.pools.drain(..) {
                self.device.destroy_descriptor_pool(pool, None);
            }
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

/// Whether `HORIZON_RT_SKIP` has the wind's bit (4096, horizon-client gfx/wind.rs).
fn wind_skipped() -> bool {
    skipped(4096)
}

/// Whether `HORIZON_RT_SKIP` has `bit` (the kernel's `SKIP`, as the game starts with it).
fn skipped(bit: u32) -> bool {
    std::env::var("HORIZON_RT_SKIP")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .is_some_and(|s| s & bit != 0)
}

/// The bounced light's turns as the trace kernel takes them (horizon-client-render
/// gfx/shadow_rays.rs `gi_turn_bits`, the same rule): `HORIZON_RT_GI_TURNS=4` one frame in four
/// (2 bits), otherwise one in eight (3).
fn gi_turn_bits() -> u32 {
    match std::env::var("HORIZON_RT_GI_TURNS").as_deref() {
        Ok("4") => 2,
        _ => 3,
    }
}
