//! Desktop tiles through dma-bufs: the Vulkan readback tier first, then a
//! LINEAR dma-buf read by the CPU, then shm (plan tiers 2, 1 and 3; the order
//! is a recorded deviation). Without the `vulkan` feature this module does
//! not exist and nothing changes.
use super::*;
use crate::gpu::{
    GpuAllocator, GpuBuffer,
    linear::{LinearLayout, read_linear},
    nodes::{drm_nodes, render_node_of},
    vulkan::VulkanReadback,
};

/// The formats both GPU tiers read back: the bytes of today's shm tiles.
const FORMATS: [PixelFormat; 2] = [PixelFormat::Argb8888, PixelFormat::Xrgb8888];

/// When a GPU tier is skipped: off until a reset, or for one compositor device.
#[derive(Default)]
struct Switch {
    off: bool,
    /// The compositor device a `DeviceMismatch` was decided for: the tier is
    /// skipped until the connection learns another one.
    mismatch: Option<Option<u64>>,
}

impl Switch {
    fn skips(&self, device: Option<u64>) -> bool {
        self.off || self.mismatch == Some(device)
    }

    /// Note a cause: a mismatch skips this device only, another structural
    /// cause turns the tier off.
    fn record(&mut self, cause: &ReadbackFallback, device: Option<u64>) {
        if matches!(cause, ReadbackFallback::DeviceMismatch) {
            self.mismatch = Some(device);
        } else if cause.is_structural() {
            self.off = true;
        }
    }
}

/// Per-connection GPU state for fast tiles, kept across calls.
#[derive(Default)]
pub(crate) struct ReadbackState {
    vulkan: Option<VulkanReadback>,
    allocator: Option<GpuAllocator>,
    /// The LINEAR tier's allocator and the compositor device it was opened for.
    linear: Option<(u64, GpuAllocator)>,
    vulkan_tier: Switch,
    linear_tier: Switch,
    last: Option<TileReadback>,
    cause: Option<ReadbackFallback>,
}

impl ReadbackState {
    /// Record why the Vulkan tier was not used for a compositor on `device`;
    /// keep the first cause.
    fn fall(&mut self, cause: ReadbackFallback, device: Option<u64>) {
        if cause.is_structural() {
            // Rebuilt after a reset or, for a mismatch, on another device.
            self.vulkan = None;
            self.allocator = None;
        }
        self.vulkan_tier.record(&cause, device);
        self.cause.get_or_insert(cause);
    }

    /// The same for the LINEAR tier. An allocation failure turns it off too:
    /// a driver that cannot allocate LINEAR will not start to.
    fn fall_linear(&mut self, cause: ReadbackFallback, device: Option<u64>) {
        let allocation = matches!(cause, ReadbackFallback::Allocation(_));
        if cause.is_structural() || allocation {
            self.linear = None;
        }
        self.linear_tier.off |= allocation;
        self.linear_tier.record(&cause, device);
        self.cause.get_or_insert(cause);
    }
}

/// The result of one tier attempt.
enum Attempt {
    Tiles(Option<DesktopTiles>),
    Fallback,
}

/// Reads one completed dma-buf back as `width * height * 4` packed bytes.
type Read<'a> = dyn FnMut(&GpuBuffer, u32, u32, Instant) -> std::result::Result<CpuBuffer, ReadbackFallback>
    + 'a;

impl Connection {
    /// Like [`Connection::capture_desktop_tiles`], but reads the outputs back
    /// through dma-bufs when the system allows it: through Vulkan, else as a
    /// LINEAR dma-buf through `mmap`, else through shm. Same `Ok(None)` rules
    /// and the same tile layout: CPU bytes in the server's format with
    /// `stride == width * 4`. `DesktopTiles::readback` says which path was
    /// used, and [`Connection::tile_readback`] why a faster one was not. GPU
    /// state is kept on this connection across calls and rebuilt when the
    /// device changes. The LINEAR tier needs the compositor's device, which
    /// the connection learns from its first capture. Requires the `vulkan`
    /// feature; `libvulkan` is loaded at runtime, and its absence is no error.
    pub fn capture_desktop_tiles_fast(
        &mut self,
        options: &CaptureOptions,
    ) -> Result<Option<DesktopTiles>> {
        check_options(options)?;
        let end = deadline(options)?;
        if let Attempt::Tiles(tiles) = self.vulkan_tiles(options, end)? {
            self.state.readback.last = Some(TileReadback::Vulkan);
            return Ok(tiles);
        }
        if let Attempt::Tiles(tiles) = self.linear_tiles(options, end)? {
            self.state.readback.last = Some(TileReadback::LinearDmabuf);
            return Ok(tiles);
        }
        let mut rest = options.clone();
        rest.timeout = end.saturating_duration_since(Instant::now());
        if rest.timeout.is_zero() {
            return Err(Error::Timeout);
        }
        let tiles = self.capture_desktop_tiles(&rest)?;
        self.state.readback.last = Some(TileReadback::Shm);
        Ok(tiles)
    }

    /// The path of the last fast capture, and the first reason a faster one
    /// was not used on this connection.
    pub fn tile_readback(&self) -> (Option<TileReadback>, Option<&ReadbackFallback>) {
        let state = &self.state.readback;
        (state.last, state.cause.as_ref())
    }

    /// Drop cached GPU state and remembered causes; the next fast capture
    /// starts over with the fastest tier.
    pub fn reset_readback(&mut self) {
        self.state.readback = ReadbackState::default();
    }

    fn vulkan_tiles(&mut self, options: &CaptureOptions, end: Instant) -> Result<Attempt> {
        let device = self.state.dmabuf_device;
        let has_dmabuf = self.state.dma.is_some();
        let state = &mut self.state.readback;
        if state.vulkan_tier.skips(device) {
            return Ok(Attempt::Fallback);
        }
        if !has_dmabuf {
            state.fall(ReadbackFallback::NoDmabuf, device);
            return Ok(Attempt::Fallback);
        }
        if state.vulkan.is_none() {
            match VulkanReadback::new(device) {
                Ok(vulkan) => state.vulkan = Some(vulkan),
                Err(cause) => {
                    state.fall(cause, device);
                    return Ok(Attempt::Fallback);
                }
            }
        }
        let vulkan = state.vulkan.as_ref().expect("created above");
        if state.allocator.is_none() {
            match GpuAllocator::open(vulkan.render_node()) {
                Ok(allocator) => state.allocator = Some(allocator),
                Err(error) => {
                    state.fall(ReadbackFallback::Allocation(error.to_string()), device);
                    state.vulkan = None;
                    state.vulkan_tier.off = true;
                    return Ok(Attempt::Fallback);
                }
            }
        }
        // ARGB8888 and XRGB8888 share one Vulkan format and so one list.
        // LINEAR belongs to the linear tier: with it in the list, NVIDIA's GBM
        // picks it and reports a zero stride (live finding, 2026-11-01).
        let mut allowed = vulkan.modifiers(PixelFormat::Argb8888);
        allowed.retain(|modifier| *modifier != 0);
        if allowed.is_empty() {
            state.fall(ReadbackFallback::NoUsableModifier, device);
            return Ok(Attempt::Fallback);
        }
        let allocator = state.allocator.as_ref().expect("opened above");
        let kind = BufferKind::Gpu(allocator.restricted(FORMATS.to_vec(), allowed));
        let mut vulkan = state.vulkan.take().expect("created above");
        let result = self.gpu_batch(
            options,
            end,
            TileReadback::Vulkan,
            kind,
            &mut |buffer, width, height, end| vulkan.read(buffer, width, height, end),
        );
        if !vulkan.is_lost() {
            self.state.readback.vulkan = Some(vulkan);
        }
        match result? {
            Ok(tiles) => Ok(Attempt::Tiles(tiles)),
            Err(cause) => {
                self.state.readback.fall(cause, device);
                Ok(Attempt::Fallback)
            }
        }
    }

    fn linear_tiles(&mut self, options: &CaptureOptions, end: Instant) -> Result<Attempt> {
        let device = self.state.dmabuf_device;
        let has_dmabuf = self.state.dma.is_some();
        let state = &mut self.state.readback;
        if state.linear_tier.skips(device) {
            return Ok(Attempt::Fallback);
        }
        if !has_dmabuf {
            state.fall_linear(ReadbackFallback::NoDmabuf, device);
            return Ok(Attempt::Fallback);
        }
        let Some(dev) = device else {
            state.fall_linear(ReadbackFallback::DeviceMismatch, device);
            return Ok(Attempt::Fallback);
        };
        if state
            .linear
            .as_ref()
            .is_none_or(|(opened, _)| *opened != dev)
        {
            let Some(node) = render_node_of(&drm_nodes(dev)) else {
                state.fall_linear(ReadbackFallback::DeviceMismatch, device);
                return Ok(Attempt::Fallback);
            };
            match GpuAllocator::open(node) {
                Ok(allocator) => state.linear = Some((dev, allocator)),
                Err(error) => {
                    state.fall_linear(ReadbackFallback::Allocation(error.to_string()), device);
                    return Ok(Attempt::Fallback);
                }
            }
        }
        let (_, allocator) = state.linear.as_ref().expect("opened above");
        let kind = BufferKind::Gpu(allocator.restricted(FORMATS.to_vec(), vec![0]));
        let result = self.gpu_batch(
            options,
            end,
            TileReadback::LinearDmabuf,
            kind,
            &mut |buffer, width, height, end| {
                let plane = buffer
                    .planes()
                    .first()
                    .ok_or_else(|| ReadbackFallback::Import("no plane".into()))?;
                let layout = LinearLayout {
                    offset: plane.offset(),
                    stride: plane.stride(),
                    width,
                    height,
                };
                read_linear(plane.fd(), layout, end).map(CpuBuffer::heap)
            },
        )?;
        match result {
            Ok(tiles) => Ok(Attempt::Tiles(tiles)),
            Err(cause) => {
                self.state.readback.fall_linear(cause, device);
                Ok(Attempt::Fallback)
            }
        }
    }

    /// One GPU tier's batch, each frame read back by `read` into a heap tile.
    /// On failure, `Ok(Err(cause))` says why the next tier should run; errors
    /// no tier can fix are returned as they are.
    fn gpu_batch(
        &mut self,
        options: &CaptureOptions,
        end: Instant,
        tier: TileReadback,
        kind: BufferKind,
        read: &mut Read<'_>,
    ) -> Result<std::result::Result<Option<DesktopTiles>, ReadbackFallback>> {
        let mut cause = None;
        let result = self.desktop_tiles(options, end, tier, |connection, outputs, next, end| {
            let mut frames: Vec<Option<Frame>> = outputs.iter().map(|_| None).collect();
            connection.capture_batch_each(outputs, next, end, kind, &mut |index, frame| {
                match to_cpu(frame, end, &mut *read) {
                    Ok(frame) => {
                        frames[index] = Some(frame);
                        Ok(())
                    }
                    Err(why) => {
                        cause = Some(why);
                        Err(Error::CaptureFailed("dma-buf readback failed".into()))
                    }
                }
            })?;
            frames
                .into_iter()
                .map(|frame| frame.ok_or(Error::SessionStopped))
                .collect()
        });
        let error = match result {
            Ok(tiles) => return Ok(Ok(tiles)),
            Err(error) => error,
        };
        Ok(Err(match (cause, &error) {
            (Some(cause), _) => cause,
            (None, Error::LayoutChanged | Error::Cancelled | Error::NoOutputs) => {
                return Err(error);
            }
            (None, Error::SessionStopped | Error::Wayland(_)) => return Err(error),
            (None, Error::Timeout) => ReadbackFallback::Timeout,
            (None, Error::Unsupported(_)) => ReadbackFallback::DeviceMismatch,
            (None, Error::UnsupportedFormat) => ReadbackFallback::NoUsableModifier,
            (None, Error::CaptureFailed(message)) => ReadbackFallback::Import(message.clone()),
            (None, other) => ReadbackFallback::Allocation(other.to_string()),
        }))
    }
}

/// Replace a dma-buf frame by a CPU frame with the same pixels, packed to
/// `width * 4` bytes per row and kept on the heap.
fn to_cpu(
    frame: Frame,
    end: Instant,
    read: &mut Read<'_>,
) -> std::result::Result<Frame, ReadbackFallback> {
    let FrameStorage::Gpu(buffer) = &frame.storage else {
        return Ok(frame);
    };
    let bytes = read(buffer, frame.width, frame.height, end)?;
    Ok(Frame {
        stride: frame.width * 4,
        storage: FrameStorage::Cpu(bytes),
        ..frame
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::{Config, Server, pixel};
    use std::os::unix::fs::MetadataExt;

    #[test]
    #[ignore = "requires LIBWAY_TEST_RENDER_NODE (for the GBM device handle) and /dev/udmabuf"]
    fn linear_tiles_carry_the_server_pixels_for_two_outputs_in_flight() {
        let node = std::env::var_os("LIBWAY_TEST_RENDER_NODE").expect("set a render node");
        let dev = std::fs::metadata(&node).unwrap().rdev();
        let (server, socket) = Server::start(Config {
            outputs: 2,
            dma: true,
            dma_device: Some(dev),
            hold_ready: 2,
            ..Default::default()
        });
        let opts = CaptureOptions {
            timeout: Duration::from_secs(2),
            ..Default::default()
        };
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        let mut allocator = GpuAllocator::open(&node).unwrap();
        allocator.udmabuf = true;
        // No Vulkan in this test; the device is otherwise learned on capture.
        c.state.readback.vulkan_tier.off = true;
        c.state.readback.linear = Some((dev, allocator));
        c.state.dmabuf_device = Some(dev);
        let desktop = c.capture_desktop_tiles_fast(&opts).unwrap().unwrap();
        assert_eq!(desktop.readback, TileReadback::LinearDmabuf);
        assert_eq!(c.tile_readback(), (Some(TileReadback::LinearDmabuf), None));
        assert_eq!(desktop.tiles.len(), 2);
        for (index, tile) in desktop.tiles.iter().enumerate() {
            let frame = &tile.frame;
            assert_eq!(
                frame.stride,
                frame.width * 4,
                "packed despite the padded stride"
            );
            let FrameStorage::Cpu(cpu) = &frame.storage else {
                panic!("tiles are CPU frames");
            };
            let expected: Vec<u8> = (0..frame.height)
                .flat_map(|y| (0..frame.width).map(move |x| (x, y)))
                .flat_map(|(x, y)| pixel(index, x, y).to_le_bytes())
                .collect();
            assert!(cpu.bytes() == expected, "output {index}");
        }
        drop(desktop);
        c.outputs(&opts).unwrap();
        server.settle();
        assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn transient_causes_retry_and_structural_ones_stay_off() {
        let mut state = ReadbackState::default();
        state.fall(ReadbackFallback::Timeout, Some(1));
        assert!(!state.vulkan_tier.skips(Some(1)));
        state.fall(ReadbackFallback::NoDmabuf, Some(1));
        let tier = &state.vulkan_tier;
        assert!(tier.skips(Some(1)) && tier.skips(Some(2)) && tier.skips(None));
        assert!(!state.linear_tier.skips(Some(1)), "tiers switch off apart");
        assert_eq!(
            state.cause,
            Some(ReadbackFallback::Timeout),
            "the first cause stays"
        );
    }

    #[test]
    fn a_device_mismatch_is_retried_only_for_another_device() {
        let mut state = ReadbackState::default();
        state.fall(ReadbackFallback::DeviceMismatch, None);
        assert!(state.vulkan_tier.skips(None));
        assert!(!state.vulkan_tier.skips(Some(7)));
        state.fall(ReadbackFallback::DeviceMismatch, Some(7));
        assert!(state.vulkan_tier.skips(Some(7)) && !state.vulkan_tier.skips(None));
        assert!(!state.vulkan_tier.off);
    }

    #[test]
    fn a_failed_linear_allocation_turns_the_linear_tier_off() {
        let mut state = ReadbackState::default();
        state.fall_linear(ReadbackFallback::Import("refused".into()), Some(1));
        assert!(!state.linear_tier.skips(Some(1)));
        state.fall_linear(ReadbackFallback::Allocation("zero stride".into()), Some(1));
        assert!(state.linear_tier.skips(Some(1)) && state.linear_tier.skips(Some(2)));
        assert!(!state.vulkan_tier.skips(Some(1)));
    }
}
