//! Hardware tests of the Vulkan readback. They allocate and fill their own
//! buffers on an explicit render node and never capture the desktop. Run with
//! `LIBWAY_TEST_RENDER_NODE=/dev/dri/renderD128 cargo test --features vulkan
//! --lib -- --ignored --test-threads=1 gpu::vulkan`.
use super::{VulkanReadback, host::HostBuffer, readback::COLOR};
use crate::{
    Limits, PixelFormat,
    capture::{Constraints, ReadbackFallback},
    gpu::{GpuAllocator, GpuBuffer},
};
use ash::vk;
use std::{
    os::{fd::OwnedFd, unix::fs::MetadataExt},
    path::PathBuf,
    time::{Duration, Instant},
};

fn open() -> (VulkanReadback, GpuAllocator) {
    let node = PathBuf::from(
        std::env::var_os("LIBWAY_TEST_RENDER_NODE").expect("set an explicit render node"),
    );
    let dev = std::fs::metadata(&node).unwrap().rdev();
    let readback = VulkanReadback::new(Some(dev)).unwrap();
    assert_eq!(readback.render_node(), node);
    let allocator = GpuAllocator::open(readback.render_node()).unwrap();
    (readback, allocator)
}

fn allocate(allocator: &GpuAllocator, (w, h): (u32, u32), modifier: u64) -> Option<GpuBuffer> {
    let format = PixelFormat::Argb8888 as u32;
    let constraints = Constraints {
        size: (w, h),
        dma: vec![(format, vec![modifier])],
        device: Some(allocator.device_id()),
        ..Default::default()
    };
    allocator
        .allocate(&constraints, &[(format, modifier)], Limits::default())
        .ok()
}

/// Bytes that differ per pixel and per channel, so a wrong stride, tiling or
/// swizzle shows.
fn pattern((w, h): (u32, u32)) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            bytes.extend_from_slice(&[
                x as u8,
                y as u8,
                ((x >> 8) | ((y >> 8) << 4)) as u8,
                (x.wrapping_mul(7) ^ y.wrapping_mul(13)) as u8,
            ]);
        }
    }
    bytes
}

/// Write `pixels` into the dma-buf through a second Vulkan import (GBM
/// cannot map tiled NVIDIA buffers), then hand it back to the foreign queue.
fn fill(readback: &VulkanReadback, buffer: &GpuBuffer, (w, h): (u32, u32), pixels: &[u8]) {
    let usage = vk::ImageUsageFlags::TRANSFER_DST;
    let image = readback.import(buffer, w, h, usage).unwrap();
    let host = HostBuffer::new(
        &readback.device,
        &readback.memory,
        pixels.len() as u64,
        vk::BufferUsageFlags::TRANSFER_SRC,
    )
    .unwrap();
    host.write(&readback.device, pixels).unwrap();
    let family = readback.family;
    let result = readback.submit(
        |device, command| {
            let acquire = vk::ImageMemoryBarrier::default()
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(super::readback::FRESH_LAYOUT)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                .dst_queue_family_index(family)
                .image(image.image)
                .subresource_range(COLOR);
            let release = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(family)
                .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                .image(image.image)
                .subresource_range(COLOR);
            let region = vk::BufferImageCopy::default()
                .buffer_row_length(w)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(vk::Extent3D {
                    width: w,
                    height: h,
                    depth: 1,
                });
            // SAFETY: recording; image imported with TRANSFER_DST, the host
            // buffer holds w*h*4 bytes with TRANSFER_SRC.
            unsafe {
                let none = vk::DependencyFlags::empty();
                device.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    none,
                    &[],
                    &[],
                    &[acquire],
                );
                device.cmd_copy_buffer_to_image(
                    command,
                    host.buffer,
                    image.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                device.cmd_pipeline_barrier(
                    command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    none,
                    &[],
                    &[],
                    &[release],
                );
            }
        },
        Instant::now() + Duration::from_secs(5),
    );
    drop(image);
    host.destroy(&readback.device);
    result.unwrap();
}

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

/// Modifiers that read back and can also be written for the test fill.
fn testable(readback: &VulkanReadback) -> Vec<u64> {
    let format = vk::Format::B8G8R8A8_UNORM;
    readback
        .modifiers(PixelFormat::Argb8888)
        .into_iter()
        .filter(|&m| {
            readback
                .max_extent(format, m, vk::ImageUsageFlags::TRANSFER_DST)
                .is_some()
        })
        .collect()
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; prints the device's dma-buf support"]
fn describe_device() {
    let (readback, _) = open();
    // SAFETY: the physical device belongs to the readback's live instance.
    let props = unsafe {
        readback
            .instance
            .get_physical_device_properties(readback.physical)
    };
    let name = props.device_name_as_c_str().unwrap_or(c"?");
    eprintln!(
        "device {name:?}, api {}.{}.{}, driver {:#x}, family {}, node {:?}",
        vk::api_version_major(props.api_version),
        vk::api_version_minor(props.api_version),
        vk::api_version_patch(props.api_version),
        props.driver_version,
        readback.family,
        readback.render_node()
    );
    let usable = readback.modifiers(PixelFormat::Argb8888);
    for m in readback.modifier_properties(vk::Format::B8G8R8A8_UNORM) {
        let modifier = m.drm_format_modifier;
        let extent = readback.max_extent(
            vk::Format::B8G8R8A8_UNORM,
            modifier,
            vk::ImageUsageFlags::TRANSFER_SRC,
        );
        eprintln!(
            "modifier {modifier:#018x}: planes {}, features {:#x}, import max {:?}, usable {}",
            m.drm_format_modifier_plane_count,
            m.drm_format_modifier_tiling_features.as_raw(),
            extent.map(|e| (e.width, e.height)),
            usable.contains(&modifier)
        );
    }
    assert!(!usable.is_empty(), "no usable modifier");
    assert!(readback.modifiers(PixelFormat::Abgr8888).is_empty());
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates and fills its own buffers"]
fn read_returns_the_written_pattern() {
    let (mut readback, allocator) = open();
    let modifiers = testable(&readback);
    assert!(!modifiers.is_empty(), "no modifier to test");
    let fds = open_fds();
    let mut tested = 0;
    for &modifier in &modifiers {
        // An odd size checks row padding and partial tiles.
        let size = (257, 131);
        let Some(buffer) = allocate(&allocator, size, modifier) else {
            eprintln!("GBM cannot allocate {modifier:#x}");
            continue;
        };
        assert_eq!(buffer.modifier(), modifier);
        let pixels = pattern(size);
        fill(&readback, &buffer, size, &pixels);
        let end = Instant::now() + Duration::from_secs(5);
        let read = readback.read(&buffer, size.0, size.1, end).unwrap();
        assert!(read == pixels, "{modifier:#x}: bytes differ");
        eprintln!(
            "{modifier:#x}: {}x{} matches, stride {}",
            size.0,
            size.1,
            buffer.planes()[0].stride()
        );
        tested += 1;
    }
    assert!(tested > 0);
    let size = (1920, 1080);
    let buffer = allocate(&allocator, size, modifiers[0]).unwrap();
    let pixels = pattern(size);
    fill(&readback, &buffer, size, &pixels);
    let mut times = Vec::new();
    for _ in 0..21 {
        let start = Instant::now();
        let end = start + Duration::from_secs(5);
        let read = readback.read(&buffer, size.0, size.1, end).unwrap();
        times.push(start.elapsed());
        assert!(read == pixels, "1920x1080 bytes differ");
    }
    times.sort();
    eprintln!(
        "1920x1080 {:#x}: median {:?}, min {:?}, max {:?} over {} reads",
        modifiers[0],
        times[times.len() / 2],
        times[0],
        times[times.len() - 1],
        times.len()
    );
    drop(buffer);
    assert_eq!(open_fds(), fds, "descriptors leaked");
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates its own buffers"]
fn failed_reads_leak_nothing() {
    let (mut readback, allocator) = open();
    let modifier = testable(&readback)[0];
    let size = (64, 64);
    let mut buffer = allocate(&allocator, size, modifier).unwrap();
    let pixels = pattern(size);
    fill(&readback, &buffer, size, &pixels);
    let end = || Instant::now() + Duration::from_secs(5);
    // Warm the host buffer cache so it does not count as a leak.
    readback.read(&buffer, size.0, size.1, end()).unwrap();
    let fds = open_fds();

    buffer.modifier = 0x0300_0000_dead_beef;
    let error = readback.read(&buffer, size.0, size.1, end()).unwrap_err();
    eprintln!("bogus modifier: {error}");
    assert!(matches!(error, ReadbackFallback::Import(_)));
    buffer.modifier = modifier;

    // A memfd is no dma-buf: it fails after the fd was duplicated and the
    // image created, which is the cleanup path that matters.
    // SAFETY: plain syscall with a static name; the result is checked.
    let raw = unsafe { libc::memfd_create(c"libway-not-dmabuf".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0);
    // SAFETY: `raw` is a fresh descriptor owned by nobody else.
    let memfd = unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw) };
    let file = std::fs::File::from(memfd);
    file.set_len(1 << 20).unwrap();
    let dmabuf = std::mem::replace(&mut buffer.planes[0].fd, file.into());
    let error = readback.read(&buffer, size.0, size.1, end()).unwrap_err();
    eprintln!("memfd: {error}");
    assert!(matches!(error, ReadbackFallback::Import(_)));
    drop(std::mem::replace(&mut buffer.planes[0].fd, dmabuf));

    // A deadline that already passed: the fence wait times out (or the GPU
    // was faster), and the next read still works.
    match readback.read(&buffer, size.0, size.1, Instant::now()) {
        Ok(read) => {
            eprintln!("past deadline: the GPU finished first");
            assert!(read == pixels);
        }
        Err(error) => {
            eprintln!("past deadline: {error}");
            assert!(matches!(error, ReadbackFallback::Timeout), "{error}");
        }
    }
    let read = readback.read(&buffer, size.0, size.1, end()).unwrap();
    assert!(read == pixels);
    assert_eq!(open_fds(), fds, "descriptors leaked");
}

#[test]
fn the_readback_is_send_but_not_sync() {
    fn send<T: Send>() {}
    send::<VulkanReadback>();
    // Resolves only while `VulkanReadback` is not `Sync`: with `Sync`, both
    // impls apply and the call becomes ambiguous.
    trait AmbiguousIfSync<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}
    <VulkanReadback as AmbiguousIfSync<_>>::check();
}

#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates its own buffers"]
fn a_lost_readback_submits_nothing_more() {
    let (mut readback, allocator) = open();
    let modifier = testable(&readback)[0];
    let size = (64, 64);
    let buffer = allocate(&allocator, size, modifier).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    readback.read(&buffer, size.0, size.1, end).unwrap();
    readback.lost.set(true);
    assert!(readback.is_lost());
    let error = readback.read(&buffer, size.0, size.1, end).unwrap_err();
    assert!(matches!(error, ReadbackFallback::Copy(_)), "{error}");
}
