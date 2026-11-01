use super::*;
use std::os::fd::OwnedFd;
use wayland_protocols::wp::linux_dmabuf::zv1::server::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf,
};
const FORMAT: u32 = 0x34325258;
const IMPLICIT: u64 = 0x00ff_ffff_ffff_ffff;
pub(super) fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, dmabuf::ZwpLinuxDmabufV1, _>(3, ());
}
pub(super) fn constraints(session: &session::ExtImageCopyCaptureSessionV1, device: Option<u64>) {
    if let Some(device) = device {
        session.dmabuf_device(device.to_ne_bytes().to_vec());
    }
    session.dmabuf_format(
        FORMAT,
        [0u64.to_ne_bytes(), IMPLICIT.to_ne_bytes()].concat(),
    );
}
impl GlobalDispatch<dmabuf::ZwpLinuxDmabufV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        new: New<dmabuf::ZwpLinuxDmabufV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let m = init.init(new, ());
        m.modifier(FORMAT, 0, 0);
        m.modifier(FORMAT, (IMPLICIT >> 32) as u32, IMPLICIT as u32);
    }
}
struct Plane {
    fd: OwnedFd,
    offset: u32,
    stride: u32,
    modifier: u64,
}
#[derive(Default)]
struct Params(Mutex<Vec<Plane>>);
pub(super) struct GpuBufferData {
    pub width: u32,
    pub height: u32,
    planes: Vec<Plane>,
}

/// Paint a LINEAR buffer through a CPU mapping of its first plane, at its
/// offset and stride; other layouts are left alone.
pub(super) fn paint(b: &GpuBufferData, index: usize) {
    let plane = &b.planes[0];
    if plane.modifier != 0 {
        return;
    }
    let (offset, stride) = (plane.offset as usize, plane.stride as usize);
    let len = offset + stride * b.height as usize;
    let file = File::from(plane.fd.try_clone().unwrap());
    // SAFETY: the client handed this LINEAR buffer over for the capture and
    // reads it only after `ready`; the server writes before sending it.
    let mut map = unsafe { MmapOptions::new().len(len).map_mut(&file).unwrap() };
    for y in 0..b.height {
        for x in 0..b.width {
            let at = offset + y as usize * stride + x as usize * 4;
            map[at..at + 4].copy_from_slice(&super::pixel(index, x, y).to_le_bytes());
        }
    }
}
impl Dispatch<dmabuf::ZwpLinuxDmabufV1, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &dmabuf::ZwpLinuxDmabufV1,
        r: dmabuf::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let dmabuf::Request::CreateParams { params_id } = r {
            s.metrics.params.fetch_add(1, Ordering::SeqCst);
            init.init(params_id, Params::default());
        }
    }
}
impl Dispatch<params::ZwpLinuxBufferParamsV1, Params> for State {
    fn request(
        s: &mut Self,
        client: &Client,
        p: &params::ZwpLinuxBufferParamsV1,
        r: params::Request,
        data: &Params,
        dh: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match r {
            params::Request::Add {
                fd,
                plane_idx,
                offset,
                stride,
                modifier_hi,
                modifier_lo,
            } => {
                let mut planes = data.0.lock().unwrap();
                assert_eq!(plane_idx as usize, planes.len());
                assert!(plane_idx < 4 && stride > 0);
                planes.push(Plane {
                    fd,
                    offset,
                    stride,
                    modifier: (u64::from(modifier_hi) << 32) | u64::from(modifier_lo),
                });
            }
            params::Request::Create {
                width,
                height,
                format,
                ..
            } => {
                assert_eq!(format, FORMAT);
                assert!(width > 0 && height > 0);
                let planes = std::mem::take(&mut *data.0.lock().unwrap());
                assert!(!planes.is_empty());
                for plane in &planes {
                    assert!(plane.modifier == 0 || plane.modifier == IMPLICIT);
                    assert!(plane.stride > 0);
                    let file = File::from(plane.fd.try_clone().unwrap());
                    assert!(file.metadata().is_ok());
                    let _ = plane.offset;
                }
                if s.config.reject_dma {
                    p.failed();
                    return;
                }
                let buffer = client
                    .create_resource::<wl_buffer::WlBuffer, GpuBufferData, State>(
                        dh,
                        1,
                        GpuBufferData {
                            width: width as u32,
                            height: height as u32,
                            planes,
                        },
                    )
                    .unwrap();
                s.metrics.buffers.fetch_add(1, Ordering::SeqCst);
                s.metrics.imports.fetch_add(1, Ordering::SeqCst);
                p.created(&buffer);
            }
            _ => {}
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &params::ZwpLinuxBufferParamsV1, _: &Params) {
        s.metrics.params.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatch<wl_buffer::WlBuffer, GpuBufferData> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Request,
        _: &GpuBufferData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &wl_buffer::WlBuffer, _: &GpuBufferData) {
        s.metrics.buffers.fetch_sub(1, Ordering::SeqCst);
    }
}
