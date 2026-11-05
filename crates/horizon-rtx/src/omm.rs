//! The cutout sections' bottom-level structures with opacity micromaps (`HORIZON_RT_OMM=1`,
//! docs/rtx-4090-handoff.md): `VK_EXT_opacity_micromap`, the Vulkan side of DXR 1.2's opacity
//! micromaps. A section whose alpha-tested geometries gain from them (`micromap::Cache::section`)
//! has its micromap built here, in a pool of Horizon's own, and its structure (wgpu's, made by
//! `create_blas` as any other) built by a raw command with the micromap attached, then marked
//! built for wgpu. The hardware then skips the transparent texels of leaves and plants and
//! confirms the opaque ones without running the alpha test (the any-hit, a ray query's candidate).
//!
//! What this needs that wgpu doesn't give: buffers for micromaps (their own usages) and a raw
//! command buffer. The pool is one device-local buffer, its ranges handed out first-fit; the
//! inputs go through a host-visible ring, a slot a frame; a structure's micromap is freed a few
//! frames after the structure goes (`Held`).

use crate::micromap::{FORMAT_4_STATE, SectionMicromaps};
use ash::vk;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wgpu::hal::api::Vulkan;

/// Whether the micromaps are asked for (`HORIZON_RT_OMM=1`; off by default until measured).
pub fn wanted() -> bool {
    std::env::var("HORIZON_RT_OMM").is_ok_and(|v| v == "1")
}

/// The micromaps' pool.
const POOL_BYTES: u64 = 64 << 20;
/// A frame's slot of the inputs' ring, and the slots: a slot is written again `SLOTS` frames
/// later, when the GPU has long read it (wgpu keeps two or three frames in flight).
const SLOT_BYTES: u64 = 4 << 20;
const SLOTS: u64 = 4;
/// How many frames after its structure goes a micromap (or an old scratch buffer) is freed.
const KEEP_FRAMES: u64 = SLOTS + 1;
/// A micromap's offset in its buffer (the specification: a multiple of 256), and the inputs'.
const ALIGN: u64 = 256;
/// How many sections between two lines of the log.
const STAT_EVERY: u64 = 200;

/// One geometry of a section's structure: its first vertex in the section's vertex buffer, its
/// quads (two triangles each through the quad indices), and its flags.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub first_vertex: u64,
    pub quads: u32,
    /// Its hits confirmed by the hardware (no any-hit).
    pub opaque: bool,
    /// Its any-hit once a ray (the translucent geometry).
    pub once: bool,
}

/// A section to build: its structure (made by wgpu for `geometries`), the buffer holding its
/// vertices, its geometries, and their micromaps (`SectionMicromaps::geometries` names them by
/// their place in `geometries`).
pub struct Section<'a> {
    pub blas: &'a mut wgpu::Blas,
    pub vertices: &'a wgpu::Buffer,
    pub geometries: &'a [Geometry],
    pub micromaps: &'a SectionMicromaps,
}

/// A section's micromap, held as long as its structure: dropped, the micromap is freed a few
/// frames later.
pub struct Held {
    id: u64,
    gone: Arc<Mutex<Vec<u64>>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Ok(mut gone) = self.gone.lock() {
            gone.push(self.id);
        }
    }
}

struct Live {
    micromap: vk::MicromapEXT,
    range: (u64, u64),
}

/// A buffer and its memory, made raw.
struct RawBuffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    address: u64,
}

/// The raw builds of the sections' structures with micromaps.
pub struct RawSections {
    device: ash::Device,
    structures: ash::khr::acceleration_structure::Device,
    micromaps: Micromaps,
    /// The finest level the device takes in four states.
    max_level: u32,
    pool: RawBuffer,
    /// The pool's free ranges, `(offset, size)`, by offset.
    free: Vec<(u64, u64)>,
    ring: RawBuffer,
    mapped: *mut u8,
    scratch: Option<(wgpu::Buffer, u64, u64)>,
    old_scratch: Vec<(u64, wgpu::Buffer)>,
    live: HashMap<u64, Live>,
    next_id: u64,
    gone: Arc<Mutex<Vec<u64>>>,
    dying: Vec<(u64, Live)>,
    frame: u64,
    /// Sections built with micromaps, left to wgpu (the pool, the ring or the structure's size),
    /// and their triangles left to the shader, of all.
    stat: [u64; 4],
}

// Plain Vulkan handles and a mapping of Horizon's own memory, used from the render thread.
unsafe impl Send for RawSections {}
unsafe impl Sync for RawSections {}

/// `VK_EXT_opacity_micromap`'s calls (ash has only their table).
struct Micromaps(ash::ext::opacity_micromap::Device);

impl Micromaps {
    unsafe fn create_micromap(
        &self,
        info: &vk::MicromapCreateInfoEXT<'_>,
        _: Option<()>,
    ) -> Result<vk::MicromapEXT, vk::Result> {
        let mut micromap = vk::MicromapEXT::null();
        // Safety: the caller's.
        unsafe {
            (self.0.fp().create_micromap_ext)(
                self.0.device(),
                info,
                std::ptr::null(),
                &mut micromap,
            )
        }
        .result_with_success(micromap)
    }
    unsafe fn destroy_micromap(&self, micromap: vk::MicromapEXT, _: Option<()>) {
        // Safety: the caller's.
        unsafe { (self.0.fp().destroy_micromap_ext)(self.0.device(), micromap, std::ptr::null()) }
    }
    unsafe fn cmd_build_micromaps(
        &self,
        cb: vk::CommandBuffer,
        infos: &[vk::MicromapBuildInfoEXT<'_>],
    ) {
        // Safety: the caller's.
        unsafe { (self.0.fp().cmd_build_micromaps_ext)(cb, infos.len() as u32, infos.as_ptr()) }
    }
    unsafe fn get_micromap_build_sizes(
        &self,
        ty: vk::AccelerationStructureBuildTypeKHR,
        info: &vk::MicromapBuildInfoEXT<'_>,
        sizes: &mut vk::MicromapBuildSizesInfoEXT<'_>,
    ) {
        // Safety: the caller's.
        unsafe { (self.0.fp().get_micromap_build_sizes_ext)(self.0.device(), ty, info, sizes) }
    }
}

/// The memory type of `bits` with `flags`, from the device's.
fn memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && props.memory_types[i as usize]
                .property_flags
                .contains(flags)
    })
}

/// A buffer of `size` bytes with `usage` (and a device address) in memory with `flags`.
fn raw_buffer(
    device: &ash::Device,
    props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: vk::BufferUsageFlags,
    flags: vk::MemoryPropertyFlags,
) -> Option<RawBuffer> {
    // Safety: plain creation on a live device; each failure destroys what was made.
    unsafe {
        let buffer = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(usage | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .ok()?;
        let needs = device.get_buffer_memory_requirements(buffer);
        let Some(kind) = memory_type(props, needs.memory_type_bits, flags) else {
            device.destroy_buffer(buffer, None);
            return None;
        };
        let mut address_flags =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let Ok(memory) = device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(needs.size)
                .memory_type_index(kind)
                .push_next(&mut address_flags),
            None,
        ) else {
            device.destroy_buffer(buffer, None);
            return None;
        };
        if device.bind_buffer_memory(buffer, memory, 0).is_err() {
            device.destroy_buffer(buffer, None);
            device.free_memory(memory, None);
            return None;
        }
        let address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer));
        Some(RawBuffer {
            buffer,
            memory,
            address,
        })
    }
}

/// The address of a wgpu buffer.
fn address(device: &ash::Device, buffer: &wgpu::Buffer) -> Option<u64> {
    // Safety: the buffer's own handle, the device that made it.
    let hal = unsafe { buffer.as_hal::<Vulkan>() }?;
    let raw = unsafe { hal.raw_handle() };
    Some(unsafe {
        device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(raw))
    })
}

/// First fit of `size` bytes, aligned, in `free`.
fn take(free: &mut Vec<(u64, u64)>, size: u64) -> Option<u64> {
    let size = size.next_multiple_of(ALIGN);
    let i = free.iter().position(|&(_, s)| s >= size)?;
    let (offset, s) = free[i];
    if s == size {
        free.remove(i);
    } else {
        free[i] = (offset + size, s - size);
    }
    Some(offset)
}

/// Gives `(offset, size)` back to `free`, joined with its neighbours.
fn give(free: &mut Vec<(u64, u64)>, (offset, size): (u64, u64)) {
    let size = size.next_multiple_of(ALIGN);
    let i = free.partition_point(|&(o, _)| o < offset);
    free.insert(i, (offset, size));
    if i + 1 < free.len() && free[i].0 + free[i].1 == free[i + 1].0 {
        free[i].1 += free[i + 1].1;
        free.remove(i + 1);
    }
    if i > 0 && free[i - 1].0 + free[i - 1].1 == free[i].0 {
        free[i - 1].1 += free[i].1;
        free.remove(i);
    }
}

/// What a section's builds read, gathered before the infos that point into it.
struct Plan {
    section: usize,
    micromap: vk::MicromapEXT,
    range: (u64, u64),
    usage: Vec<vk::MicromapUsageEXT>,
    data: u64,
    triangles: u64,
    micromap_scratch: u64,
    /// Each geometry's triangles' indices' address and usage counts (`None`: no micromap).
    geometries: Vec<Option<(u64, Vec<vk::MicromapUsageEXT>)>>,
    vertices: u64,
    blas: vk::AccelerationStructureKHR,
    blas_scratch: u64,
}

impl RawSections {
    /// A section left to wgpu, and why: the first few in the log, all in the count.
    fn left(&mut self, why: &str) {
        self.stat[1] += 1;
        if self.stat[1] <= 5 {
            log::warn!("Horizon RTX: a section's structure left to wgpu, without micromaps: {why}");
        }
        if (self.stat[0] + self.stat[1]) % STAT_EVERY == 0 {
            log::info!(
                "Horizon RTX: {} sections built with opacity micromaps, {} left to wgpu",
                self.stat[0],
                self.stat[1]
            );
        }
    }

    /// The raw builds, on a Vulkan device with `VK_EXT_opacity_micromap` enabled
    /// (horizon-hdr's device) and `HORIZON_RT_OMM=1`; `None` otherwise.
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        if !wanted() {
            return None;
        }
        // Safety: read, cloned (the handles and function tables), dropped before wgpu takes the
        // device back.
        let hal = unsafe { device.as_hal::<Vulkan>() }?;
        if !hal
            .enabled_device_extensions()
            .contains(&ash::ext::opacity_micromap::NAME)
        {
            log::warn!(
                "Horizon RTX: HORIZON_RT_OMM=1, but the device has no VK_EXT_opacity_micromap: the sections' structures stay wgpu's"
            );
            return None;
        }
        let raw = hal.raw_device().clone();
        let instance = hal.shared_instance().raw_instance().clone();
        let physical = hal.raw_physical_device();
        drop(hal);
        let structures = ash::khr::acceleration_structure::Device::new(&instance, &raw);
        let micromaps = Micromaps(ash::ext::opacity_micromap::Device::new(&instance, &raw));
        let mut omm = vk::PhysicalDeviceOpacityMicromapPropertiesEXT::default();
        let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut omm);
        // Safety: queries on the device's own physical device.
        let memory = unsafe {
            instance.get_physical_device_properties2(physical, &mut props);
            instance.get_physical_device_memory_properties(physical)
        };
        let max_level = omm.max_opacity4_state_subdivision_level;
        let pool = raw_buffer(
            &raw,
            &memory,
            POOL_BYTES,
            vk::BufferUsageFlags::MICROMAP_STORAGE_EXT,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let Some(ring) = raw_buffer(
            &raw,
            &memory,
            SLOT_BYTES * SLOTS,
            vk::BufferUsageFlags::MICROMAP_BUILD_INPUT_READ_ONLY_EXT
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        ) else {
            // Safety: made above, never used.
            unsafe {
                raw.destroy_buffer(pool.buffer, None);
                raw.free_memory(pool.memory, None);
            }
            return None;
        };
        // Safety: host-visible memory of Horizon's own, mapped once for its life.
        let mapped = unsafe {
            raw.map_memory(
                ring.memory,
                0,
                SLOT_BYTES * SLOTS,
                vk::MemoryMapFlags::empty(),
            )
        }
        .ok()? as *mut u8;
        log::info!(
            "Horizon RTX: the cutout sections' structures built with opacity micromaps, {} MB of pool, level {} at most",
            POOL_BYTES >> 20,
            max_level.min(crate::micromap::MAX_LEVEL)
        );
        Some(Self {
            device: raw,
            structures,
            micromaps,
            max_level,
            pool,
            free: vec![(0, POOL_BYTES)],
            ring,
            mapped,
            scratch: None,
            old_scratch: Vec::new(),
            live: HashMap::new(),
            next_id: 0,
            gone: Arc::default(),
            dying: Vec::new(),
            frame: 0,
            stat: [0; 4],
        })
    }

    /// The finest level the micromaps may take on this device.
    pub fn max_level(&self) -> u32 {
        self.max_level.min(crate::micromap::MAX_LEVEL)
    }

    /// The next frame: frees what the GPU is done with. Call once a frame.
    pub fn frame(&mut self) {
        self.frame += 1;
        let now = self.frame;
        if let Ok(mut gone) = self.gone.lock() {
            for id in gone.drain(..) {
                if let Some(live) = self.live.remove(&id) {
                    self.dying.push((now, live));
                }
            }
        }
        let mut i = 0;
        while i < self.dying.len() {
            if self.dying[i].0 + KEEP_FRAMES <= now {
                let (_, live) = self.dying.swap_remove(i);
                // Safety: the structure that used it went `KEEP_FRAMES` frames ago.
                unsafe {
                    self.micromaps.destroy_micromap(live.micromap, None);
                }
                give(&mut self.free, live.range);
            } else {
                i += 1;
            }
        }
        self.old_scratch.retain(|(at, _)| at + KEEP_FRAMES > now);
    }

    /// The geometry `g`'s triangles data for a build.
    fn triangles_data<'a>(
        vertices: u64,
        g: &Geometry,
        indices: u64,
        stride: u64,
    ) -> vk::AccelerationStructureGeometryTrianglesDataKHR<'a> {
        vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_data(vk::DeviceOrHostAddressConstKHR {
                device_address: vertices + g.first_vertex * stride,
            })
            .vertex_stride(stride)
            .max_vertex(g.quads * 4)
            .index_type(vk::IndexType::UINT32)
            .index_data(vk::DeviceOrHostAddressConstKHR {
                device_address: indices,
            })
    }

    fn flags(g: &Geometry) -> vk::GeometryFlagsKHR {
        let mut flags = vk::GeometryFlagsKHR::empty();
        if g.opaque {
            flags |= vk::GeometryFlagsKHR::OPAQUE;
        }
        if g.once {
            flags |= vk::GeometryFlagsKHR::NO_DUPLICATE_ANY_HIT_INVOCATION;
        }
        flags
    }

    /// Builds `sections` with their micromaps, their vertices `stride` bytes apart and their
    /// triangles through the quad indices `indices`, in a command buffer of their own (submit it
    /// after the commands that wrote the vertices and before the top-level build). Each section
    /// gets its micromap's hold, or `None` where it doesn't fit (the pool, this frame's slot, the
    /// structure's size): build those through wgpu as before.
    pub fn build(
        &mut self,
        device: &wgpu::Device,
        indices: &wgpu::Buffer,
        stride: u64,
        sections: &mut [Section<'_>],
    ) -> (Vec<Option<Held>>, Option<wgpu::CommandBuffer>) {
        let mut held: Vec<Option<Held>> = (0..sections.len()).map(|_| None).collect();
        let Some(indices) = address(&self.device, indices) else {
            return (held, None);
        };
        let slot = (self.frame % SLOTS) * SLOT_BYTES;
        let mut written = 0u64;
        let mut plans = Vec::new();
        let (mut micromap_scratch, mut blas_scratch) = (0u64, 0u64);
        for (s, section) in sections.iter_mut().enumerate() {
            let m = section.micromaps;
            // The inputs: the states, the triangles (`VkMicromapTriangleEXT`, 8 bytes), each
            // alpha-tested geometry's indices.
            let index_bytes: u64 = m.geometries.iter().map(|(_, i)| i.len() as u64 * 4).sum();
            let need = (m.data.len() as u64).next_multiple_of(ALIGN)
                + (m.triangles.len() as u64 * 8).next_multiple_of(ALIGN)
                + index_bytes
                + ALIGN * m.geometries.len() as u64;
            let level_ok = m.triangles.iter().all(|t| t[1] & 0xffff <= self.max_level);
            if m.triangles.is_empty() || !level_ok || written + need > SLOT_BYTES {
                let why = if m.triangles.is_empty() {
                    "no micromap of its own (special indices only)".to_owned()
                } else if !level_ok {
                    format!("a level past the device's {}", self.max_level)
                } else {
                    format!("this frame's ring slot full ({written} + {need} bytes)")
                };
                self.left(&why);
                continue;
            }
            let Some(vertices) = address(&self.device, section.vertices) else {
                continue;
            };
            let base = slot + written;
            let mut at = 0u64;
            let mut put = |bytes: &[u8]| {
                let offset = base + at;
                // Safety: within this frame's slot of the mapped ring, which the GPU last read
                // `SLOTS` frames ago.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        self.mapped.add(offset as usize),
                        bytes.len(),
                    );
                }
                at = (at + bytes.len() as u64).next_multiple_of(ALIGN);
                self.ring.address + offset
            };
            let data = put(&m.data);
            let triangles = put(bytemuck::cast_slice(&m.triangles));
            let mut geometries: Vec<Option<(u64, Vec<vk::MicromapUsageEXT>)>> =
                vec![None; section.geometries.len()];
            for (g, idx) in &m.geometries {
                let Some(slot) = geometries.get_mut(*g as usize) else {
                    continue;
                };
                let mut levels = std::collections::BTreeMap::<u32, u32>::new();
                for &i in idx {
                    if i >= 0 {
                        *levels
                            .entry(m.triangles[i as usize][1] & 0xffff)
                            .or_default() += 1;
                    }
                }
                let usage = levels
                    .into_iter()
                    .map(|(level, count)| vk::MicromapUsageEXT {
                        count,
                        subdivision_level: level,
                        format: FORMAT_4_STATE,
                    })
                    .collect();
                *slot = Some((put(bytemuck::cast_slice(idx)), usage));
            }
            written += at;
            let usage: Vec<_> = m
                .usage
                .iter()
                .map(|&(count, level)| vk::MicromapUsageEXT {
                    count,
                    subdivision_level: level,
                    format: FORMAT_4_STATE,
                })
                .collect();
            // The micromap's size and scratch.
            let info = vk::MicromapBuildInfoEXT::default()
                .ty(vk::MicromapTypeEXT::OPACITY_MICROMAP)
                .flags(vk::BuildMicromapFlagsEXT::PREFER_FAST_TRACE)
                .mode(vk::BuildMicromapModeEXT::BUILD)
                .usage_counts(&usage);
            let mut sizes = vk::MicromapBuildSizesInfoEXT::default();
            // Safety: a query.
            unsafe {
                self.micromaps.get_micromap_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &info,
                    &mut sizes,
                );
            }
            let Some(offset) = take(&mut self.free, sizes.micromap_size) else {
                self.left(&format!(
                    "the pool full ({} bytes asked)",
                    sizes.micromap_size
                ));
                continue;
            };
            let range = (offset, sizes.micromap_size);
            // Safety: a range of the pool no live micromap holds.
            let Ok(micromap) = (unsafe {
                self.micromaps.create_micromap(
                    &vk::MicromapCreateInfoEXT::default()
                        .buffer(self.pool.buffer)
                        .offset(offset)
                        .size(sizes.micromap_size)
                        .ty(vk::MicromapTypeEXT::OPACITY_MICROMAP),
                    None,
                )
            }) else {
                give(&mut self.free, range);
                self.left("vkCreateMicromapEXT failed");
                continue;
            };
            // The structure's size with the micromap: it must fit in what wgpu made.
            let (blas, made) = {
                // Safety: the structure's own handle and size, read while it lives.
                let Some(hal) = (unsafe { section.blas.as_hal::<Vulkan>() }) else {
                    // Safety: just made, never used.
                    unsafe { self.micromaps.destroy_micromap(micromap, None) };
                    give(&mut self.free, range);
                    continue;
                };
                (unsafe { hal.raw_handle() }, hal.size())
            };
            let mut omm: Vec<_> = geometries
                .iter()
                .map(|g| {
                    g.as_ref().map(|(index, usage)| {
                        vk::AccelerationStructureTrianglesOpacityMicromapEXT::default()
                            .index_type(vk::IndexType::UINT32)
                            .index_buffer(vk::DeviceOrHostAddressConstKHR {
                                device_address: *index,
                            })
                            .index_stride(4)
                            .usage_counts(usage)
                            .micromap(micromap)
                    })
                })
                .collect();
            let vk_geometries: Vec<_> = section
                .geometries
                .iter()
                .zip(omm.iter_mut())
                .map(|(g, omm)| {
                    let mut triangles = Self::triangles_data(vertices, g, indices, stride);
                    if let Some(omm) = omm {
                        triangles = triangles.push_next(omm);
                    }
                    vk::AccelerationStructureGeometryKHR::default()
                        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
                        .flags(Self::flags(g))
                })
                .collect();
            let counts: Vec<u32> = section.geometries.iter().map(|g| g.quads * 2).collect();
            let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .geometries(&vk_geometries);
            let mut as_sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
            // Safety: a query.
            unsafe {
                self.structures.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &info,
                    &counts,
                    &mut as_sizes,
                );
            }
            if as_sizes.acceleration_structure_size > made {
                // Safety: just made, never used.
                unsafe { self.micromaps.destroy_micromap(micromap, None) };
                give(&mut self.free, range);
                self.left(&format!(
                    "its structure with the micromap takes {} bytes, wgpu made {made}",
                    as_sizes.acceleration_structure_size
                ));
                continue;
            }
            plans.push(Plan {
                section: s,
                micromap,
                range,
                usage,
                data,
                triangles,
                micromap_scratch,
                geometries,
                vertices,
                blas,
                blas_scratch,
            });
            micromap_scratch += sizes.build_scratch_size.next_multiple_of(ALIGN);
            blas_scratch += as_sizes.build_scratch_size.next_multiple_of(ALIGN);
            let (unknown, all) = m.unknown();
            self.stat[2] += u64::from(unknown);
            self.stat[3] += u64::from(all);
        }
        if plans.is_empty() {
            return (held, None);
        }
        // One scratch for both phases (a barrier between them), grown as needed.
        let need = micromap_scratch.max(blas_scratch) + ALIGN;
        if self.scratch.as_ref().is_none_or(|s| s.2 < need) {
            if let Some((old, _, _)) = self.scratch.take() {
                self.old_scratch.push((self.frame, old));
            }
            let size = need.next_power_of_two();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Horizon RTX: the micromaps' and sections' scratch"),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                mapped_at_creation: false,
            });
            let Some(at) = address(&self.device, &buffer) else {
                for p in plans {
                    // Safety: made above, never used.
                    unsafe { self.micromaps.destroy_micromap(p.micromap, None) };
                    give(&mut self.free, p.range);
                }
                return (held, None);
            };
            self.scratch = Some((buffer, at.next_multiple_of(ALIGN), size));
        }
        let scratch = self.scratch.as_ref().map_or(0, |s| s.1);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Horizon RTX: the sections' builds with micromaps"),
        });
        let barrier = |device: &ash::Device, cb: vk::CommandBuffer| {
            // Everything before (the vertices' copies, the micromaps' builds) before what
            // follows: a full barrier, once or twice a frame.
            let all = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)];
            // Safety: a barrier on the encoder's own command buffer.
            unsafe {
                device.cmd_pipeline_barrier(
                    cb,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &all,
                    &[],
                    &[],
                );
            }
        };
        // Safety: raw commands on the encoder's own command buffer, ordered by full barriers
        // against what comes before and after.
        unsafe {
            encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                let Some(hal) = hal else {
                    return;
                };
                let cb = hal.raw_handle();
                barrier(&self.device, cb);
                let infos: Vec<_> = plans
                    .iter()
                    .map(|p| {
                        vk::MicromapBuildInfoEXT::default()
                            .ty(vk::MicromapTypeEXT::OPACITY_MICROMAP)
                            .flags(vk::BuildMicromapFlagsEXT::PREFER_FAST_TRACE)
                            .mode(vk::BuildMicromapModeEXT::BUILD)
                            .dst_micromap(p.micromap)
                            .usage_counts(&p.usage)
                            .data(vk::DeviceOrHostAddressConstKHR {
                                device_address: p.data,
                            })
                            .scratch_data(vk::DeviceOrHostAddressKHR {
                                device_address: scratch + p.micromap_scratch,
                            })
                            .triangle_array(vk::DeviceOrHostAddressConstKHR {
                                device_address: p.triangles,
                            })
                            .triangle_array_stride(8)
                    })
                    .collect();
                self.micromaps.cmd_build_micromaps(cb, &infos);
                barrier(&self.device, cb);
                let mut omms: Vec<Vec<_>> = plans
                    .iter()
                    .map(|p| {
                        p.geometries
                            .iter()
                            .map(|g| {
                                g.as_ref().map(|(index, usage)| {
                                    vk::AccelerationStructureTrianglesOpacityMicromapEXT::default()
                                        .index_type(vk::IndexType::UINT32)
                                        .index_buffer(vk::DeviceOrHostAddressConstKHR {
                                            device_address: *index,
                                        })
                                        .index_stride(4)
                                        .usage_counts(usage)
                                        .micromap(p.micromap)
                                })
                            })
                            .collect()
                    })
                    .collect();
                let geometries: Vec<Vec<_>> = plans
                    .iter()
                    .zip(omms.iter_mut())
                    .map(|(p, omm)| {
                        sections[p.section]
                            .geometries
                            .iter()
                            .zip(omm.iter_mut())
                            .map(|(g, omm)| {
                                let mut triangles =
                                    Self::triangles_data(p.vertices, g, indices, stride);
                                if let Some(omm) = omm {
                                    triangles = triangles.push_next(omm);
                                }
                                vk::AccelerationStructureGeometryKHR::default()
                                    .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                                    .geometry(vk::AccelerationStructureGeometryDataKHR {
                                        triangles,
                                    })
                                    .flags(Self::flags(g))
                            })
                            .collect()
                    })
                    .collect();
                let ranges: Vec<Vec<_>> = plans
                    .iter()
                    .map(|p| {
                        sections[p.section]
                            .geometries
                            .iter()
                            .map(|g| {
                                vk::AccelerationStructureBuildRangeInfoKHR::default()
                                    .primitive_count(g.quads * 2)
                            })
                            .collect()
                    })
                    .collect();
                let infos: Vec<_> = plans
                    .iter()
                    .zip(&geometries)
                    .map(|(p, g)| {
                        vk::AccelerationStructureBuildGeometryInfoKHR::default()
                            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                            .dst_acceleration_structure(p.blas)
                            .geometries(g)
                            .scratch_data(vk::DeviceOrHostAddressKHR {
                                device_address: scratch + p.blas_scratch,
                            })
                    })
                    .collect();
                let range_refs: Vec<&[_]> = ranges.iter().map(Vec::as_slice).collect();
                self.structures
                    .cmd_build_acceleration_structures(cb, &infos, &range_refs);
                barrier(&self.device, cb);
            });
        }
        let built: Vec<usize> = plans.iter().map(|p| p.section).collect();
        for p in plans {
            let id = self.next_id;
            self.next_id += 1;
            self.live.insert(
                id,
                Live {
                    micromap: p.micromap,
                    range: p.range,
                },
            );
            held[p.section] = Some(Held {
                id,
                gone: self.gone.clone(),
            });
            self.stat[0] += 1;
            if self.stat[0] == 1 || (self.stat[0] + self.stat[1]) % STAT_EVERY == 0 {
                log::info!(
                    "Horizon RTX: {} sections built with opacity micromaps ({} left to wgpu), {:.0} % of their alpha-tested triangles left to the shader, {} KB of the pool free",
                    self.stat[0],
                    self.stat[1],
                    100.0 * self.stat[2] as f64 / self.stat[3].max(1) as f64,
                    self.free.iter().map(|f| f.1).sum::<u64>() / 1024
                );
            }
        }
        // Safety: built above, with nothing for wgpu to walk at submission.
        unsafe {
            encoder.mark_acceleration_structures_built(
                sections
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| built.contains(i))
                    .map(|(_, s)| &*s.blas),
                std::iter::empty(),
            );
        }
        (held, Some(encoder.finish()))
    }
}

impl Drop for RawSections {
    fn drop(&mut self) {
        // Safety: the device is idle when the renderer goes (wgpu waits before it drops).
        unsafe {
            let _ = self.device.device_wait_idle();
            for (_, live) in self.live.drain().chain(self.dying.drain(..)) {
                self.micromaps.destroy_micromap(live.micromap, None);
            }
            self.device.unmap_memory(self.ring.memory);
            for b in [&self.pool, &self.ring] {
                self.device.destroy_buffer(b.buffer, None);
                self.device.free_memory(b.memory, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pool_gives_ranges_back_joined() {
        let mut free = vec![(0, 4096)];
        let a = take(&mut free, 300).unwrap();
        let b = take(&mut free, 256).unwrap();
        assert_eq!((a, b), (0, 512));
        assert_eq!(free, vec![(768, 4096 - 768)]);
        give(&mut free, (a, 300));
        assert_eq!(free, vec![(0, 512), (768, 4096 - 768)]);
        give(&mut free, (b, 256));
        assert_eq!(free, vec![(0, 4096)]);
        assert_eq!(take(&mut free, 8192), None);
    }
}
