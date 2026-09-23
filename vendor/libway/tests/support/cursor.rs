use super::*;
use wayland_protocols::ext::image_copy_capture::v1::server::ext_image_copy_capture_cursor_session_v1;
use wayland_server::protocol::{wl_pointer, wl_seat};
pub type CursorSession =
    ext_image_copy_capture_cursor_session_v1::ExtImageCopyCaptureCursorSessionV1;
#[derive(Clone, Copy)]
pub enum Fault {
    None,
    IdleImage,
    Flood,
    NoPointer,
    Stopped,
    Resize,
}
pub fn advertise(dh: &DisplayHandle) {
    dh.create_global::<State, wl_seat::WlSeat, _>(5, ());
}
impl GlobalDispatch<wl_seat::WlSeat, ()> for State {
    fn bind(
        s: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        r: New<wl_seat::WlSeat>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        s.metrics.seats.fetch_add(1, Ordering::SeqCst);
        let seat = init.init(r, ());
        seat.capabilities(if matches!(s.config.cursor, Some(Fault::NoPointer)) {
            wl_seat::Capability::Keyboard
        } else {
            wl_seat::Capability::Pointer
        });
        seat.name("test-seat".into());
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        r: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_seat::Request::GetPointer { id } = r {
            s.metrics.pointers.fetch_add(1, Ordering::SeqCst);
            init.init(id, ());
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &wl_seat::WlSeat, _: &()) {
        s.metrics.seats.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_pointer::WlPointer,
        _: wl_pointer::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &wl_pointer::WlPointer, _: &()) {
        s.metrics.pointers.fetch_sub(1, Ordering::SeqCst);
    }
}
pub fn create(s: &mut State, init: &mut DataInit<'_, State>, id: New<CursorSession>, index: usize) {
    s.metrics.cursor_sessions.fetch_add(1, Ordering::SeqCst);
    let cursor = init.init(id, index);
    cursor.enter();
    cursor.position(-2, 3);
    cursor.hotspot(1, 1);
    if matches!(s.config.cursor, Some(Fault::Flood)) {
        for n in 0..300 {
            cursor.position(n, n);
        }
    }
}
impl Dispatch<CursorSession, usize> for State {
    fn request(
        s: &mut Self,
        _: &Client,
        cursor: &CursorSession,
        r: ext_image_copy_capture_cursor_session_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } = r
        {
            s.metrics.sessions.fetch_add(1, Ordering::SeqCst);
            let session = init.init(
                session,
                Arc::new(SessionData {
                    index: *index,
                    cursor: Some(cursor.clone()),
                    ..Default::default()
                }),
            );
            session.buffer_size(4, 2);
            session.shm_format(wl_shm::Format::Argb8888);
            session.done();
        }
    }
    fn destroyed(s: &mut Self, _: ClientId, _: &CursorSession, _: &usize) {
        s.metrics.cursor_sessions.fetch_sub(1, Ordering::SeqCst);
    }
}
pub fn capture(
    s: &mut State,
    f: &ext_frame::ExtImageCopyCaptureFrameV1,
    data: &FrameData,
    n: usize,
) {
    let cursor = data.session.cursor.as_ref().unwrap();
    if n > 0 && matches!(s.config.cursor, Some(Fault::IdleImage)) {
        // Metadata must be delivered while the image request remains pending.
        cursor.position(41, -3);
        cursor.leave();
        cursor.enter();
        cursor.position(5, 6);
        return;
    }
    if matches!(s.config.cursor, Some(Fault::Stopped)) {
        data.owner.stopped();
        return;
    }
    let guard = data.buffer.lock().unwrap();
    let b = guard.as_ref().unwrap().data::<Buffer>().unwrap();
    let mut map = unsafe { MmapOptions::new().map_mut(&*b.file).unwrap() };
    for y in 0..b.height {
        for x in 0..b.width {
            let at = b.offset + (y * b.stride + x * 4) as usize;
            map[at..at + 4].copy_from_slice(&0x80102030u32.to_le_bytes());
        }
    }
    if n == 0 && matches!(s.config.cursor, Some(Fault::Resize)) {
        data.owner.buffer_size(2, 3);
        data.owner.shm_format(wl_shm::Format::Argb8888);
        data.owner.done();
    }
    s.metrics.captures.fetch_add(1, Ordering::SeqCst);
    f.transform(wl_output::Transform::Normal);
    f.damage(0, 0, b.width as i32, b.height as i32);
    f.presentation_time(0, 123, n as u32);
    f.ready();
    // Sent AFTER ready, in the same dispatch batch. Must affect only the next image.
    cursor.hotspot(9, 7);
}
