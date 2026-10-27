//! Dragging a card out of the shelf, and the data it hands the drop target.

use smithay_client_toolkit::data_device_manager::WritePipe;
use smithay_client_toolkit::data_device_manager::data_device::DataDeviceHandler;
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::DataSourceHandler;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_source::WlDataSource;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::shelf::model::CardKind;

use super::Daemon;

/// Stream a dragged file to the drop target without holding it in memory. A
/// receiver that closes early ends the copy with an error.
pub(super) fn send_file(
    path: &std::path::Path,
    out: &mut impl std::io::Write,
) -> std::io::Result<u64> {
    std::io::copy(&mut std::fs::File::open(path)?, out)
}

/// What a drag offers: the bytes under the MIME type of the file, and always a
/// file reference. A video of unknown type goes as a reference only.
pub(super) fn drag_mime_types(kind: CardKind, path: &std::path::Path) -> Vec<&'static str> {
    let bytes = match kind {
        CardKind::Image => Some("image/png"),
        CardKind::Video => match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("mp4") => Some("video/mp4"),
            Some("mkv") => Some("video/x-matroska"),
            Some("webm") => Some("video/webm"),
            _ => None,
        },
    };
    bytes.into_iter().chain(["text/uri-list"]).collect()
}

impl DataSourceHandler for Daemon {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        fd: WritePipe,
    ) {
        use std::io::Write;
        use std::os::fd::OwnedFd;
        let is_ours = self
            .drag_source
            .as_ref()
            .map(|d| d.inner() == source)
            .unwrap_or(false);
        if !is_ours {
            return;
        }
        let Some((path, _)) = self.drag_card.clone() else {
            return;
        };
        // Write the requested data on a DETACHED THREAD, never on the calloop
        // event-loop thread. The drop target reads the pipe at its own pace
        // (Electron apps like Discord request the full multi-MB PNG and drain it
        // lazily), and the pipe buffer is only ~64 KiB. A synchronous
        // `write_all` here would block the entire daemon until the reader drains
        // or the pipe breaks — freezing the shelf ("card is there but dead",
        // clicks/remove/drag all stop, and the drag never reaches dnd_finished so
        // it can never recover). Offloading keeps the daemon responsive.
        let fd = OwnedFd::from(fd);
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(fd);
            match mime.as_str() {
                "text/uri-list" => {
                    let uri = crate::clipboard::uri_list_for(&path);
                    let _ = file.write_all(uri.as_bytes());
                }
                _ => {
                    let _ = send_file(&path, &mut file);
                }
            }
        });
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        // A cancel without a successful drop = the user dropped onto nothing (or
        // the target rejected it): fall back to copying the image to the clipboard
        // so the drag is never wasted.
        if !self.drop_ok
            && let Some((path, kind)) = self.drag_card.clone()
        {
            // Only screenshots fall back to a clipboard image copy; a video's
            // raw bytes aren't meaningful on the clipboard (mirrors copy_card).
            if kind == CardKind::Video {
                eprintln!(
                    "boltsnap daemon: drag cancelled for a video card; not copying to clipboard (use Save)"
                );
            } else if let Err(e) =
                crate::clipboard::copy_to_clipboard(&path, crate::Backend::Wayland)
            {
                eprintln!("boltsnap daemon: fallback copy failed: {e}");
            }
        }
        self.clear_drag();
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.drop_ok = true;
    }

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {
        self.clear_drag();
    }

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl DataDeviceHandler for Daemon {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _x: f64,
        _y: f64,
    ) {
    }
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for Daemon {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl Daemon {
    /// Start a Wayland drag for thumbnail `id`, offering image/png (or video/mp4)
    /// and a file URI, with the thumbnail itself as the drag icon. Returns true if
    /// the drag was actually started.
    pub(super) fn begin_drag(&mut self, id: u64, serial: u32) -> bool {
        let (path, kind) = match self.model.get(id) {
            Some(t) => (t.png_path.clone(), t.kind),
            None => return false,
        };
        let qh = match self.qh.clone() {
            Some(qh) => qh,
            None => return false,
        };
        if self.data_device.is_none() {
            return false;
        }
        let origin = match self.layer.as_ref() {
            Some(l) => l.wl_surface().clone(),
            None => return false,
        };

        // wl_data_device assigns the dnd_icon ROLE to the surface inside
        // start_drag, and a surface must not carry a buffer on its role-assigning
        // commit. So: create the source + surface, call start_drag FIRST (assigns
        // the role), and only THEN attach + damage + commit the icon buffer.
        // Doing it the other way (buffer committed before the role existed) made
        // the icon flash and then vanish inconsistently — and reliably broke when
        // the drag crossed to another output, where the compositor re-maps the
        // icon and found it in a bad state.
        let source = self.ddm.create_drag_and_drop_source(
            &qh,
            drag_mime_types(kind, &path),
            DndAction::Copy,
        );
        let icon = self.compositor.create_surface(&qh);
        {
            let device = self.data_device.as_ref().unwrap();
            source.start_drag(device, &origin, Some(&icon), serial);
        }

        // Build the icon in its OWN pool, and retain BOTH the pool and the Buffer
        // for the whole drag (dropping the Buffer destroys the wl_buffer, which is
        // what made the icon disappear mid-drag / on monitor change).
        if let Some(t) = self.model.get(id) {
            let (iw, ih) = t.thumb.dimensions();
            let stride = (iw * 4) as i32;
            let bytes = crate::shelf::paint::build_drag_icon(&t.thumb, iw, ih, 10.0, 0.85);
            if let Ok(mut pool) = SlotPool::new((iw * ih * 4) as usize, &self.shm) {
                if let Ok((buf, canvas)) = pool.create_buffer(
                    iw as i32,
                    ih as i32,
                    stride,
                    wayland_client::protocol::wl_shm::Format::Argb8888,
                ) {
                    // SlotPool rounds the canvas length up to a 64-byte boundary,
                    // so `canvas` can be a few bytes longer than the tightly-packed
                    // icon (`iw*ih*4`). The wl_buffer uses our exact stride, so the
                    // trailing slack is unused — copy only the real bytes (a plain
                    // copy_from_slice panics on the length mismatch).
                    canvas[..bytes.len()].copy_from_slice(&bytes);
                    let _ = buf.attach_to(&icon);
                    icon.damage_buffer(0, 0, iw as i32, ih as i32);
                    icon.commit();
                    self.drag_icon_buffer = Some(buf);
                }
                self.drag_icon_pool = Some(pool);
            }
        }

        self.drag_card = Some((path, kind));
        self.drop_ok = false;
        self.icon_surface = Some(icon);
        self.drag_source = Some(source);
        true
    }

    /// Reset all in-flight drag state in one place (kept in sync between the
    /// `cancelled` and `dnd_finished` paths), then repaint the shelf once.
    pub(super) fn clear_drag(&mut self) {
        self.forget_drag();
        // draw() is suppressed while a drag is active; now that it's cleared,
        // repaint once to restore a clean shelf frame.
        if let Some(qh) = self.qh.clone() {
            self.draw(&qh);
        }
    }
}
