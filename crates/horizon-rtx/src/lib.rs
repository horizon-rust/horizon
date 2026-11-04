//! The RTX 4090 build's raw Vulkan pieces of RT lighting ([docs/rtx-4090.md](../../docs/rtx-4090.md),
//! phase 3). The game crate forbids `unsafe`; what must reach Vulkan under wgpu lives here, as the
//! HDR metadata does in `horizon-hdr`.
//!
//! First brick: **the top-level structure built outside wgpu's API.** wgpu's `build_acceleration_structures`
//! takes every instance through the CPU every time (its resolution, encoding and tracking, then the
//! submission's validation of every dependency), and RT lighting's structure holds thousands of
//! section instances and is built every frame the hand or an entity moves. Here the instances live in
//! a buffer of Horizon's own (`RawTlas::instances`), written only where they change
//! (`RawTlas::write`), and the build is one raw command on wgpu's own structure
//! (`RawTlas::build`), then marked built for wgpu (`mark_acceleration_structures_built`) with no
//! dependency list to walk.

/// One instance as Vulkan reads it (`VkAccelerationStructureInstanceKHR`): a 3x4 row-major
/// transform, the custom index (24 bits) and mask (8), the hit group offset (24) and flags (8), and
/// the bottom-level structure's device address (0: an inactive instance, which the build skips).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub transform: [f32; 12],
    custom_and_mask: u32,
    offset_and_flags: u32,
    pub address: u64,
}

/// An instance's bytes, and a row's in the instance buffer.
pub const INSTANCE_SIZE: u64 = std::mem::size_of::<Instance>() as u64;

impl Instance {
    /// An instance of the structure at `address`, placed by `transform` (as wgpu's
    /// `TlasInstance::new` takes it), with `custom` as its custom index (24 bits) and `mask`.
    pub fn new(address: u64, transform: [f32; 12], custom: u32, mask: u8) -> Self {
        Self {
            transform,
            custom_and_mask: (custom & 0x00ff_ffff) | (u32::from(mask) << 24),
            offset_and_flags: 0,
            address,
        }
    }

    /// An instance the build skips.
    pub const NONE: Self = Self {
        transform: [0.0; 12],
        custom_and_mask: 0,
        offset_and_flags: 0,
        address: 0,
    };
}

/// Whether the raw build is wanted: on with the crate (the `rtx` feature of the game), unless
/// `HORIZON_RAW_TLAS=0`.
pub fn wanted() -> bool {
    !std::env::var("HORIZON_RAW_TLAS").is_ok_and(|v| v == "0")
}

#[cfg(not(target_os = "macos"))]
mod vulkan {
    use super::{INSTANCE_SIZE, Instance};
    use ash::vk;
    use wgpu::hal::api::Vulkan;

    /// The scratch buffer's address alignment the build needs at most (the specification bounds
    /// `minAccelerationStructureScratchOffsetAlignment` by 256): the buffer gets that much slack
    /// and its address is rounded up.
    const SCRATCH_ALIGN: u64 = 256;

    /// wgpu's top-level structure, built by raw commands from `instances`.
    pub struct RawTlas {
        device: ash::Device,
        structures: ash::khr::acceleration_structure::Device,
        tlas: vk::AccelerationStructureKHR,
        max_instances: u32,
        /// The instances, `INSTANCE_SIZE` bytes a row, `max_instances` rows.
        pub instances: wgpu::Buffer,
        instances_address: u64,
        _scratch: wgpu::Buffer,
        scratch_address: u64,
    }

    // The handles are plain Vulkan handles wgpu keeps alive as long as its own objects, which
    // this holds (the buffers) or outlives nothing of (the structure: `build` takes it).
    unsafe impl Send for RawTlas {}
    unsafe impl Sync for RawTlas {}

    impl RawTlas {
        /// The raw build of `tlas` (made by wgpu for `max_instances` instances, with
        /// `PREFER_FAST_TRACE`), on a Vulkan device; `None` on another backend.
        pub fn new(
            device: &wgpu::Device,
            tlas: &mut wgpu::Tlas,
            max_instances: u32,
        ) -> Option<Self> {
            if !super::wanted() {
                return None;
            }
            let (raw_device, raw_instance) = {
                // Safety: read, cloned (the handles and function tables), dropped before wgpu
                // takes the device back.
                let hal = unsafe { device.as_hal::<Vulkan>() }?;
                (
                    hal.raw_device().clone(),
                    hal.shared_instance().raw_instance().clone(),
                )
            };
            let structures =
                ash::khr::acceleration_structure::Device::new(&raw_instance, &raw_device);
            let raw_tlas = {
                // Safety: the handle is only used while `tlas` lives (`build` takes it).
                let hal = unsafe { tlas.as_hal::<Vulkan>() }?;
                unsafe { hal.raw_handle() }
            };
            // What the build of `max_instances` instances needs in scratch.
            let geometry = Self::geometry(0);
            let geometries = [geometry];
            let info = Self::build_info(&geometries, raw_tlas, 0);
            // Safety: a query, the structure and the function table valid.
            let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
            unsafe {
                structures.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &info,
                    &[max_instances],
                    &mut sizes,
                );
            }
            // `TLAS_INPUT` gives both buffers a device address; the scratch is a storage buffer
            // for the build to write.
            let scratch = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT lighting's raw top-level scratch"),
                size: sizes.build_scratch_size.max(1) + SCRATCH_ALIGN,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::TLAS_INPUT,
                mapped_at_creation: false,
            });
            let instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("RT lighting's raw top-level instances"),
                size: u64::from(max_instances) * INSTANCE_SIZE,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::TLAS_INPUT,
                mapped_at_creation: false,
            });
            let address = |buffer: &wgpu::Buffer| -> Option<u64> {
                // Safety: the buffer's own handle, the device that made it.
                let hal = unsafe { buffer.as_hal::<Vulkan>() }?;
                let raw = unsafe { hal.raw_handle() };
                Some(unsafe {
                    raw_device.get_buffer_device_address(
                        &vk::BufferDeviceAddressInfo::default().buffer(raw),
                    )
                })
            };
            let instances_address = address(&instances)?;
            let scratch_address = address(&scratch)?.next_multiple_of(SCRATCH_ALIGN);
            log::info!(
                "Horizon RTX: the top-level structure built by raw Vulkan commands, {max_instances} instances, {} KB of scratch",
                sizes.build_scratch_size / 1024
            );
            Some(Self {
                device: raw_device,
                structures,
                tlas: raw_tlas,
                max_instances,
                instances,
                instances_address,
                _scratch: scratch,
                scratch_address,
            })
        }

        /// The device address of `blas`, for its instances; `None` on another backend.
        pub fn blas_address(&self, blas: &mut wgpu::Blas) -> Option<u64> {
            // Safety: the structure's own handle; the address stays valid as long as it lives.
            let hal = unsafe { blas.as_hal::<Vulkan>() }?;
            let raw = unsafe { hal.raw_handle() };
            Some(unsafe {
                self.structures.get_acceleration_structure_device_address(
                    &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                        .acceleration_structure(raw),
                )
            })
        }

        /// Writes `rows` from row `first` (copied before the frame's commands run, as every
        /// `write_buffer` is).
        pub fn write(&self, queue: &wgpu::Queue, first: u32, rows: &[Instance]) {
            debug_assert!(first as usize + rows.len() <= self.max_instances as usize);
            if !rows.is_empty() {
                queue.write_buffer(
                    &self.instances,
                    u64::from(first) * INSTANCE_SIZE,
                    bytemuck::cast_slice(rows),
                );
            }
        }

        fn geometry<'a>(address: u64) -> vk::AccelerationStructureGeometryKHR<'a> {
            vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
                        .array_of_pointers(false)
                        .data(vk::DeviceOrHostAddressConstKHR {
                            device_address: address,
                        }),
                })
        }

        fn build_info<'a>(
            geometries: &'a [vk::AccelerationStructureGeometryKHR<'a>],
            tlas: vk::AccelerationStructureKHR,
            scratch: u64,
        ) -> vk::AccelerationStructureBuildGeometryInfoKHR<'a> {
            vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .dst_acceleration_structure(tlas)
                .geometries(geometries)
                .scratch_data(vk::DeviceOrHostAddressKHR {
                    device_address: scratch,
                })
        }

        /// Records the build of the first `count` instances into `tlas` (the structure given to
        /// `new`), after wgpu's own commands so far in `encoder` (the bottom-level builds and the
        /// instance writes among them), and marks it built for wgpu.
        pub fn build(&self, encoder: &mut wgpu::CommandEncoder, tlas: &wgpu::Tlas, count: u32) {
            let count = count.min(self.max_instances);
            // Safety: raw commands on the encoder's own command buffer, between wgpu's commands:
            // the barriers below order them against the transfer of the instances, the
            // bottom-level builds before and the shaders after.
            unsafe {
                encoder.as_hal_mut::<Vulkan, _, _>(|hal| {
                    let Some(hal) = hal else {
                        return;
                    };
                    let cb = hal.raw_handle();
                    let before = [vk::MemoryBarrier::default()
                        .src_access_mask(
                            vk::AccessFlags::TRANSFER_WRITE
                                | vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR,
                        )
                        .dst_access_mask(
                            vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR
                                | vk::AccessFlags::SHADER_READ,
                        )];
                    self.device.cmd_pipeline_barrier(
                        cb,
                        vk::PipelineStageFlags::TRANSFER
                            | vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                        vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                        vk::DependencyFlags::empty(),
                        &before,
                        &[],
                        &[],
                    );
                    let geometries = [Self::geometry(self.instances_address)];
                    let info = Self::build_info(&geometries, self.tlas, self.scratch_address);
                    let range = [vk::AccelerationStructureBuildRangeInfoKHR::default()
                        .primitive_count(count)];
                    self.structures
                        .cmd_build_acceleration_structures(cb, &[info], &[&range]);
                    // The shaders read it next; the next frame's transfer rewrites the instances.
                    let after = [vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                        .dst_access_mask(
                            vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR
                                | vk::AccessFlags::SHADER_READ
                                | vk::AccessFlags::TRANSFER_WRITE,
                        )];
                    self.device.cmd_pipeline_barrier(
                        cb,
                        vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                        vk::PipelineStageFlags::COMPUTE_SHADER
                            | vk::PipelineStageFlags::FRAGMENT_SHADER
                            | vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &after,
                        &[],
                        &[],
                    );
                });
                // Built, with nothing for wgpu to walk at submission.
                encoder.mark_acceleration_structures_built(&[], [tlas]);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use vulkan::RawTlas;

/// No raw Vulkan on macOS: the structure is wgpu's to build.
#[cfg(target_os = "macos")]
pub struct RawTlas {
    pub instances: wgpu::Buffer,
}

#[cfg(target_os = "macos")]
impl RawTlas {
    pub fn new(_: &wgpu::Device, _: &mut wgpu::Tlas, _: u32) -> Option<Self> {
        None
    }
    pub fn blas_address(&self, _: &mut wgpu::Blas) -> Option<u64> {
        None
    }
    pub fn write(&self, _: &wgpu::Queue, _: u32, _: &[Instance]) {}
    pub fn build(&self, _: &mut wgpu::CommandEncoder, _: &wgpu::Tlas, _: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instance_is_laid_out_as_vulkan_reads_it() {
        assert_eq!(INSTANCE_SIZE, 64);
        let transform = [1.0, 0.0, 0.0, 4.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0, 1.0, 6.0];
        let instance = Instance::new(
            0x1234_5678_9abc_def0,
            transform,
            0x00ab_cdef | 0xff00_0000,
            0x3c,
        );
        let bytes: &[u8] = bytemuck::bytes_of(&instance);
        // The transform first, as 12 floats.
        assert_eq!(&bytes[..48], bytemuck::cast_slice::<f32, u8>(&transform));
        // The custom index kept to 24 bits, the mask in the top byte.
        assert_eq!(
            u32::from_le_bytes(bytes[48..52].try_into().unwrap()),
            0x3cab_cdef
        );
        assert_eq!(u32::from_le_bytes(bytes[52..56].try_into().unwrap()), 0);
        assert_eq!(
            u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
            0x1234_5678_9abc_def0
        );
        assert_eq!(Instance::NONE.address, 0);
    }
}
