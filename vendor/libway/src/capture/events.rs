// Protocol callbacks only; capture sequencing and ownership live in the parent.
use super::*;

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &WlConnection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == "zwp_linux_dmabuf_v1" && version < 3 {
                    return;
                }
                if matches!(
                    interface.as_str(),
                    "wl_shm"
                        | "zxdg_output_manager_v1"
                        | "ext_image_copy_capture_manager_v1"
                        | "ext_output_image_capture_source_manager_v1"
                        | "zwlr_screencopy_manager_v1"
                        | "zwp_linux_dmabuf_v1"
                ) {
                    // Bind one instance per interface. An unselected duplicate's
                    // removal must not invalidate the manager we actually use.
                    if s.globals.values().any(|selected| selected == &interface) {
                        return;
                    }
                    if s.globals.len() >= 64 {
                        s.global_error = Some(Error::LimitExceeded);
                        return;
                    }
                    s.globals.insert(name, interface.clone());
                }
                match interface.as_str() {
                    "wl_seat" => {
                        if s.seats.len() >= 16 {
                            s.global_error = Some(Error::LimitExceeded);
                            return;
                        }
                        s.seats.insert(name, version);
                    }
                    "wl_output" => {
                        if s.outputs.len() >= s.limits.max_outputs {
                            s.global_error = Some(Error::LimitExceeded);
                            return;
                        }
                        s.next_id += 1;
                        let wl = registry.bind(name, version.min(4), qh, name);
                        let info = Output {
                            id: OutputId(s.connection_id, s.next_id),
                            name: format!("output-{name}"),
                            description: String::new(),
                            logical: Rect {
                                x: 0,
                                y: 0,
                                width: 0,
                                height: 0,
                            },
                            mode_size: (0, 0),
                            transform: Transform::Normal,
                        };
                        s.outputs.insert(
                            name,
                            OutputRecord {
                                wl,
                                xdg: None,
                                info,
                                position: None,
                                logical_size: None,
                                scale: 1,
                            },
                        );
                        s.attach_xdg(qh);
                    }
                    "wl_shm" => s.shm = Some(registry.bind(name, 1, qh, ())),
                    "zxdg_output_manager_v1" => {
                        s.output_manager = Some(registry.bind(name, version.min(3), qh, ()));
                        s.attach_xdg(qh);
                    }
                    "ext_image_copy_capture_manager_v1" => {
                        s.ext = Some(registry.bind(name, 1, qh, ()))
                    }
                    "ext_output_image_capture_source_manager_v1" => {
                        s.source = Some(registry.bind(name, 1, qh, ()))
                    }
                    "zwlr_screencopy_manager_v1" => {
                        s.wlr = Some(registry.bind(name, version.min(3), qh, ()))
                    }
                    "zwp_linux_dmabuf_v1" if version >= 3 => {
                        s.dma = Some(registry.bind(name, 3, qh, ()))
                    }
                    _ => {}
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                if s.seats.remove(&name).is_some()
                    && s.cursor.as_ref().is_some_and(|c| c.seat_global == name)
                {
                    s.global_error = Some(Error::SessionStopped);
                }
                match s.globals.remove(&name).as_deref() {
                    Some("wl_shm") => {
                        s.shm = None;
                    }
                    Some("zxdg_output_manager_v1") => {
                        if let Some(m) = s.output_manager.take() {
                            m.destroy();
                        }
                    }
                    Some("ext_image_copy_capture_manager_v1") => {
                        if let Some(m) = s.ext.take() {
                            m.destroy();
                        }
                    }
                    Some("ext_output_image_capture_source_manager_v1") => {
                        if let Some(m) = s.source.take() {
                            m.destroy();
                        }
                    }
                    Some("zwlr_screencopy_manager_v1") => {
                        if let Some(m) = s.wlr.take() {
                            m.destroy();
                        }
                    }
                    Some("zwp_linux_dmabuf_v1") => {
                        if let Some(m) = s.dma.take() {
                            m.destroy();
                        }
                        s.dma_formats.clear();
                    }
                    _ => {}
                }
                if let Some(o) = s.outputs.remove(&name) {
                    if let Some(x) = o.xdg {
                        x.destroy();
                    }
                    if o.wl.version() >= 3 {
                        o.wl.release();
                    }
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, u64> for State {
    fn event(
        s: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        serial: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        s.sync = s.sync.max(*serial);
    }
}
impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        s: &mut Self,
        _: &wl_output::WlOutput,
        e: wl_output::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = s.outputs.get_mut(name) else {
            return;
        };
        match e {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                o.info.logical.x = x;
                o.info.logical.y = y;
                match Transform::from_wire(transform.into()) {
                    Ok(t) => o.info.transform = t,
                    Err(e) => s.global_error = Some(e),
                }
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                if width <= 0 || height <= 0 {
                    s.global_error = Some(Error::InvalidDimensions);
                } else {
                    o.info.mode_size = (width as u32, height as u32);
                }
            }
            wl_output::Event::Scale { factor } => {
                if factor <= 0 {
                    s.global_error = Some(Error::InvalidDimensions);
                } else {
                    o.scale = factor as u32;
                }
            }
            wl_output::Event::Name { name } => o.info.name = name,
            wl_output::Event::Description { description } => o.info.description = description,
            _ => {}
        }
    }
}
impl Dispatch<xdg_output::ZxdgOutputV1, u32> for State {
    fn event(
        s: &mut Self,
        _: &xdg_output::ZxdgOutputV1,
        e: xdg_output::Event,
        name: &u32,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(o) = s.outputs.get_mut(name) else {
            return;
        };
        match e {
            xdg_output::Event::LogicalPosition { x, y } => o.position = Some((x, y)),
            xdg_output::Event::LogicalSize { width, height } => {
                if width <= 0 || height <= 0 {
                    s.global_error = Some(Error::InvalidDimensions);
                } else {
                    o.logical_size = Some((width as u32, height as u32));
                }
            }
            xdg_output::Event::Name { name } => o.info.name = name,
            xdg_output::Event::Description { description } => o.info.description = description,
            _ => {}
        }
    }
}
impl Dispatch<ext_session::ExtImageCopyCaptureSessionV1, u64> for State {
    fn event(
        s: &mut Self,
        _: &ext_session::ExtImageCopyCaptureSessionV1,
        e: ext_session::Event,
        token: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(p) = s.pending(*token) else { return };
        match e {
            ext_session::Event::BufferSize { width, height } => p.batch().size = (width, height),
            ext_session::Event::ShmFormat {
                format: WEnum::Value(format),
            } => {
                if p.batch().shm.len() < 64 {
                    p.batch().shm.push(format);
                } else {
                    p.error = Some(Error::LimitExceeded);
                }
            }
            ext_session::Event::DmabufFormat { format, modifiers } => {
                if modifiers.len() % 8 != 0 || modifiers.len() > 8192 || p.batch().dma.len() >= 256
                {
                    p.error = Some(Error::LimitExceeded);
                    return;
                }
                let mods = modifiers
                    .chunks_exact(8)
                    .map(|v| u64::from_ne_bytes(v.try_into().expect("eight byte chunk")))
                    .collect();
                p.batch().dma.push((format, mods));
            }
            ext_session::Event::DmabufDevice { device } => {
                if device.len() == std::mem::size_of::<libc::dev_t>() && device.len() == 8 {
                    p.batch().device = Some(u64::from_ne_bytes(
                        device.as_slice().try_into().expect("checked length"),
                    ));
                } else {
                    p.error = Some(Error::CaptureFailed(
                        "invalid DMA-BUF device identifier".into(),
                    ));
                }
            }
            ext_session::Event::Done => {
                p.constraints = std::mem::take(&mut p.batch);
                p.batch_open = false;
                p.formats_done = true;
            }
            ext_session::Event::Stopped => p.error = Some(Error::SessionStopped),
            _ => {}
        }
    }
}
fn timestamp(p: &mut Pending, hi: u32, lo: u32, ns: u32) {
    if ns >= 1_000_000_000 {
        p.error = Some(Error::CaptureFailed(
            "invalid presentation timestamp".into(),
        ));
    } else {
        p.timestamp = Some(Duration::new((u64::from(hi) << 32) | u64::from(lo), ns));
    }
}
fn damage(p: &mut Pending, x: u32, y: u32, w: u32, h: u32) {
    if p.damage.len() >= 4096 {
        p.error = Some(Error::LimitExceeded);
    } else if u64::from(x) + u64::from(w) > u64::from(p.frame_size.0)
        || u64::from(y) + u64::from(h) > u64::from(p.frame_size.1)
    {
        p.error = Some(Error::InvalidDimensions);
    } else {
        p.damage.push((x, y, w, h));
    }
}
impl Dispatch<ext_frame::ExtImageCopyCaptureFrameV1, u64> for State {
    fn event(
        s: &mut Self,
        _: &ext_frame::ExtImageCopyCaptureFrameV1,
        e: ext_frame::Event,
        token: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(p) = s.pending(*token) else { return };
        match e {
            ext_frame::Event::Ready => {
                p.ready = true;
                if let Some(cursor) = &mut s.cursor {
                    cursor.image_ready();
                }
            }
            ext_frame::Event::Failed { reason } => {
                p.error = Some(Error::CaptureFailed(format!("EXT {reason:?}")))
            }
            ext_frame::Event::Transform { transform } => {
                match Transform::from_wire(transform.into()) {
                    Ok(t) => p.transform = Some(t),
                    Err(e) => p.error = Some(e),
                }
            }
            ext_frame::Event::PresentationTime {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => timestamp(p, tv_sec_hi, tv_sec_lo, tv_nsec),
            ext_frame::Event::Damage {
                x,
                y,
                width,
                height,
            } => {
                if x < 0 || y < 0 || width < 0 || height < 0 {
                    p.error = Some(Error::InvalidDimensions);
                } else {
                    damage(p, x as u32, y as u32, width as u32, height as u32);
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wlr_frame::ZwlrScreencopyFrameV1, u64> for State {
    fn event(
        s: &mut Self,
        frame: &wlr_frame::ZwlrScreencopyFrameV1,
        e: wlr_frame::Event,
        token: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        let Some(p) = s.pending(*token) else { return };
        match e {
            wlr_frame::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                p.constraints.size = (width, height);
                p.constraints.stride = Some(stride);
                if let WEnum::Value(f) = format {
                    if p.constraints.shm.len() < 64 {
                        p.constraints.shm.push(f);
                    } else {
                        p.error = Some(Error::LimitExceeded);
                    }
                }
                if frame.version() < 3 {
                    p.formats_done = true;
                }
            }
            wlr_frame::Event::LinuxDmabuf {
                format,
                width,
                height,
            } => {
                p.constraints.size = (width, height);
                p.constraints.dma_any_modifier = true;
                if p.constraints.dma.len() < 256 {
                    p.constraints.dma.push((format, Vec::new()));
                } else {
                    p.error = Some(Error::LimitExceeded);
                }
            }
            wlr_frame::Event::BufferDone => p.formats_done = true,
            wlr_frame::Event::Ready {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                timestamp(p, tv_sec_hi, tv_sec_lo, tv_nsec);
                p.ready = true;
            }
            wlr_frame::Event::Failed => {
                p.error = Some(Error::CaptureFailed("WLR copy failed".into()))
            }
            wlr_frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => p.inverted = flags.contains(wlr_frame::Flags::YInvert),
            wlr_frame::Event::Damage {
                x,
                y,
                width,
                height,
            } => damage(p, x, y, width, height),
            _ => {}
        }
    }
}
impl Dispatch<dmabuf::ZwpLinuxDmabufV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &dmabuf::ZwpLinuxDmabufV1,
        e: dmabuf::Event,
        _: &(),
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        if let dmabuf::Event::Modifier {
            format,
            modifier_hi,
            modifier_lo,
        } = e
        {
            if s.dma_formats.len() < 8192 {
                s.dma_formats.push((
                    format,
                    (u64::from(modifier_hi) << 32) | u64::from(modifier_lo),
                ));
            } else {
                s.global_error = Some(Error::LimitExceeded);
            }
        }
    }
}
impl Dispatch<params::ZwpLinuxBufferParamsV1, u64> for State {
    fn event(
        s: &mut Self,
        _: &params::ZwpLinuxBufferParamsV1,
        e: params::Event,
        token: &u64,
        _: &WlConnection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            params::Event::Created { buffer } => {
                if let Some(p) = s.pending(*token) {
                    p.imported = Some(buffer);
                    p.import_done = true;
                } else {
                    buffer.destroy();
                }
            }
            params::Event::Failed => {
                if let Some(p) = s.pending(*token) {
                    p.import_done = true;
                }
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State,params::ZwpLinuxBufferParamsV1,[0=>(wl_buffer::WlBuffer,())]);
}
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore output_manager::ZxdgOutputManagerV1);
delegate_noop!(State: ignore ext_manager::ExtImageCopyCaptureManagerV1);
delegate_noop!(State: ignore source_manager::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore source::ExtImageCaptureSourceV1);
delegate_noop!(State: ignore wlr_manager::ZwlrScreencopyManagerV1);
