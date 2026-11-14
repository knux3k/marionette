//! A wgpu device that the OpenXR runtime accepts: the runtime creates the Vulkan instance and
//! device (`XR_KHR_vulkan_enable2`) with the extensions wgpu needs, and wgpu wraps them.

use crate::{XrError, codes};
use ash::vk::{self, Handle};
use marionette_render::Gpu;
use openxr as xr;
use std::ffi::c_char;
use wgpu::hal::api::Vulkan as HalVulkan;

/// What the session needs to know about the Vulkan objects behind [`Gpu`].
pub struct VulkanHandles {
    pub instance: u64,
    pub physical_device: u64,
    pub device: u64,
    pub queue_family: u32,
}

fn vk_err(what: &str, e: impl std::fmt::Display) -> XrError {
    XrError::new(codes::VULKAN, format!("{what}: {e}"))
}

/// Vulkan version to ask for: 1.3 if the runtime allows it, else its highest.
fn api_version(reqs: &xr::vulkan::Requirements) -> u32 {
    let want = xr::Version::new(1, 3, 0);
    let max = reqs.max_api_version_supported;
    let v = if want > max {
        xr::Version::new(max.major(), max.minor(), 0)
    } else {
        want
    };
    let v = if v < reqs.min_api_version_supported {
        reqs.min_api_version_supported
    } else {
        v
    };
    vk::make_api_version(0, v.major() as u32, v.minor() as u32, 0)
}

/// Creates the GPU through the runtime. The returned [`Gpu`] owns the Vulkan instance and
/// device (wgpu destroys them when dropped, after every session object is gone).
pub fn create(instance: &xr::Instance, system: xr::SystemId) -> Result<(Gpu, VulkanHandles), XrError> {
    let reqs = instance
        .graphics_requirements::<xr::Vulkan>(system)
        .map_err(|e| vk_err("runtime Vulkan requirements", e))?;
    let api = api_version(&reqs);
    // SAFETY: loading the system Vulkan loader; the entry lives as long as the instance.
    let entry = unsafe { ash::Entry::load() }.map_err(|e| {
        vk_err("cannot load the Vulkan loader", e)
            .fix("install the GPU's Vulkan driver (OpenXR here needs Vulkan)")
    })?;
    let flags = wgpu::InstanceFlags::from_build_config().with_env();
    let exts = wgpu::hal::vulkan::Instance::desired_extensions(&entry, api, flags)
        .map_err(|e| vk_err("Vulkan instance extensions", e))?;
    let ext_ptrs: Vec<*const c_char> = exts.iter().map(|e| e.as_ptr()).collect();
    let app = vk::ApplicationInfo::default()
        .application_name(c"marionette")
        .engine_name(c"Marionette")
        .api_version(api);
    let info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&ext_ptrs);
    // SAFETY: the create info and the strings it points to outlive the call; the function
    // pointer comes from the loaded entry and has the signature OpenXR expects.
    let raw_instance = unsafe {
        instance.create_vulkan_instance(
            system,
            std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
                entry.static_fn().get_instance_proc_addr,
            ),
            &info as *const _ as *const _,
        )
    }
    .map_err(|e| vk_err("runtime could not create a Vulkan instance", e))?
    .map_err(|e| vk_err("Vulkan instance", vk::Result::from_raw(e)))?;
    // SAFETY: a valid instance just created from this entry.
    let vk_instance =
        unsafe { ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(raw_instance as _)) };
    // SAFETY: the instance was created from `entry` with `exts`, `api` and `flags`; wgpu-hal
    // takes ownership (no drop callback) and destroys it last.
    let hal_instance = unsafe {
        wgpu::hal::vulkan::Instance::from_raw(
            entry,
            vk_instance.clone(),
            api,
            0,
            None,
            exts,
            flags,
            Default::default(),
            false,
            None,
        )
    }
    .map_err(|e| vk_err("wgpu cannot use the Vulkan instance", e))?;
    // SAFETY: `raw_instance` is the instance the runtime created for this system.
    let phys = unsafe { instance.vulkan_graphics_device(system, raw_instance) }
        .map_err(|e| vk_err("runtime did not name a GPU", e))?;
    let phys = vk::PhysicalDevice::from_raw(phys as _);
    let exposed = hal_instance
        .expose_adapter(phys)
        .ok_or_else(|| vk_err("wgpu cannot use the headset's GPU", "adapter not supported"))?;
    // SAFETY: plain query on a valid physical device.
    let families = unsafe { vk_instance.get_physical_device_queue_family_properties(phys) };
    let family = families
        .iter()
        .position(|f| {
            f.queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        })
        .ok_or_else(|| vk_err("the headset's GPU", "no graphics queue"))? as u32;

    let features = wgpu::Features::empty();
    let limits = wgpu::Limits::default();
    let dev_exts = exposed.adapter.required_device_extensions(features);
    let mut phd_features = exposed.adapter.physical_device_features(&dev_exts, features);
    let dev_ptrs: Vec<*const c_char> = dev_exts.iter().map(|e| e.as_ptr()).collect();
    let priorities = [1.0f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family)
        .queue_priorities(&priorities)];
    let dinfo = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queues)
        .enabled_extension_names(&dev_ptrs);
    let dinfo = phd_features.add_to_device_create(dinfo);
    let gipa = vk_instance_gipa(&hal_instance);
    // SAFETY: the create info and everything it points to outlive the call.
    let raw_device = unsafe {
        instance.create_vulkan_device(system, gipa, phys.as_raw() as _, &dinfo as *const _ as *const _)
    }
    .map_err(|e| vk_err("runtime could not create a Vulkan device", e))?
    .map_err(|e| vk_err("Vulkan device", vk::Result::from_raw(e)))?;
    // SAFETY: a valid device just created on `phys` with `dev_exts` and `phd_features`.
    let device = unsafe { ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(raw_device as _)) };
    // SAFETY: as above; wgpu-hal takes ownership of the device.
    let open = unsafe {
        exposed.adapter.device_from_raw(
            device,
            None,
            &dev_exts,
            features,
            &limits,
            &wgpu::MemoryHints::Performance,
            family,
            0,
        )
    }
    .map_err(|e| vk_err("wgpu cannot use the Vulkan device", e))?;
    // SAFETY: the hal objects come from this instance and adapter.
    let wgpu_instance = unsafe { wgpu::Instance::from_hal::<HalVulkan>(hal_instance) };
    let adapter = unsafe { wgpu_instance.create_adapter_from_hal(exposed) };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal(
            open,
            &wgpu::DeviceDescriptor {
                label: Some("marionette xr"),
                required_features: features,
                required_limits: limits,
                ..Default::default()
            },
        )
    }
    .map_err(|e| vk_err("wgpu device", e))?;
    let handles = VulkanHandles {
        instance: raw_instance as u64,
        physical_device: phys.as_raw(),
        device: raw_device as u64,
        queue_family: family,
    };
    Ok((
        Gpu {
            instance: wgpu_instance,
            adapter,
            device,
            queue,
        },
        handles,
    ))
}

fn vk_instance_gipa(hal: &wgpu::hal::vulkan::Instance) -> xr::sys::platform::VkGetInstanceProcAddr {
    // SAFETY: same signature, different binding crates.
    unsafe {
        std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
            hal.shared_instance().entry().static_fn().get_instance_proc_addr,
        )
    }
}

/// Wraps one swapchain image (owned by the runtime) as a wgpu texture.
pub fn wrap_image(
    gpu: &Gpu,
    image: u64,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> Result<wgpu::Texture, XrError> {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    // SAFETY: the device is a Vulkan device (made by `create`).
    let hal_device = unsafe { gpu.device.as_hal::<HalVulkan>() }
        .ok_or_else(|| vk_err("swapchain", "the device is not a Vulkan device"))?;
    // SAFETY: the image was created by the runtime with this size, format and usage; the
    // runtime owns it and its memory (External, empty drop callback), and every wgpu
    // texture wrapping it is dropped before the swapchain.
    let hal_texture = unsafe {
        hal_device.texture_from_raw(
            vk::Image::from_raw(image),
            &wgpu::hal::TextureDescriptor {
                label: Some("xr swapchain image"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUses::COLOR_TARGET,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            },
            Some(Box::new(|| {})),
            wgpu::hal::vulkan::TextureMemory::External,
        )
    };
    drop(hal_device);
    // SAFETY: as above.
    Ok(unsafe {
        gpu.device.create_texture_from_hal::<HalVulkan>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("xr swapchain image"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
            wgpu::TextureUses::UNINITIALIZED,
        )
    })
}

/// The Vulkan format number of a wgpu color format we can render to, and back.
pub fn vk_format(f: wgpu::TextureFormat) -> u32 {
    match f {
        wgpu::TextureFormat::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB.as_raw() as u32,
        wgpu::TextureFormat::Bgra8UnormSrgb => vk::Format::B8G8R8A8_SRGB.as_raw() as u32,
        _ => 0,
    }
}

/// Picks the swapchain format: 8-bit sRGB (the renderer writes linear light and lets the
/// hardware encode, as in a window).
pub fn pick_format(offered: &[u32]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8UnormSrgb,
    ]
    .into_iter()
    .find(|f| offered.contains(&vk_format(*f)))
}
