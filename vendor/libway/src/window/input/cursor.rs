//! Cursor shapes through the optional `wp_cursor_shape_manager_v1`.
//!
//! One shape device exists per live `wl_pointer`, created right after `get_pointer`
//! and destroyed before the pointer is released. Shapes are stored per window until
//! it closes and applied with the seat's latest enter serial: immediately for seats
//! whose pointer is inside the window, and on every later enter. There is no xcursor
//! fallback; without the manager [`Host::set_cursor_shape`] returns `Unsupported`.
use super::*;
use crate::window::Host;
use wayland_client::{QueueHandle, delegate_noop, protocol::wl_registry::WlRegistry};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};

/// A named cursor image the compositor draws while the pointer is over a window.
/// Mirrors the shapes of cursor-shape-v1 version 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CursorShape {
    /// The default arrow.
    Default,
    /// A context menu is available.
    ContextMenu,
    /// Help is available.
    Help,
    /// A link or other interactive element.
    Pointer,
    /// Work in the background; the pointer stays usable.
    Progress,
    /// The program is busy.
    Wait,
    /// Cells may be selected.
    Cell,
    /// A simple crosshair.
    Crosshair,
    /// Text may be selected.
    Text,
    /// Vertical text may be selected.
    VerticalText,
    /// Drag and drop: an alias or shortcut will be created.
    Alias,
    /// Drag and drop: the item will be copied.
    Copy,
    /// Drag and drop: the item will be moved.
    Move,
    /// Drag and drop: the item cannot be dropped here.
    NoDrop,
    /// The requested action will not be carried out.
    NotAllowed,
    /// Something can be grabbed.
    Grab,
    /// Something is being grabbed.
    Grabbing,
    /// The east border is moved.
    EResize,
    /// The north border is moved.
    NResize,
    /// The north-east corner is moved.
    NeResize,
    /// The north-west corner is moved.
    NwResize,
    /// The south border is moved.
    SResize,
    /// The south-east corner is moved.
    SeResize,
    /// The south-west corner is moved.
    SwResize,
    /// The west border is moved.
    WResize,
    /// The east and west borders are moved.
    EwResize,
    /// The north and south borders are moved.
    NsResize,
    /// The north-east and south-west corners are moved.
    NeswResize,
    /// The north-west and south-east corners are moved.
    NwseResize,
    /// A column can be resized horizontally.
    ColResize,
    /// A row can be resized vertically.
    RowResize,
    /// Something can be scrolled in any direction.
    AllScroll,
    /// Something can be zoomed in.
    ZoomIn,
    /// Something can be zoomed out.
    ZoomOut,
}

impl CursorShape {
    fn native(self) -> Shape {
        match self {
            Self::Default => Shape::Default,
            Self::ContextMenu => Shape::ContextMenu,
            Self::Help => Shape::Help,
            Self::Pointer => Shape::Pointer,
            Self::Progress => Shape::Progress,
            Self::Wait => Shape::Wait,
            Self::Cell => Shape::Cell,
            Self::Crosshair => Shape::Crosshair,
            Self::Text => Shape::Text,
            Self::VerticalText => Shape::VerticalText,
            Self::Alias => Shape::Alias,
            Self::Copy => Shape::Copy,
            Self::Move => Shape::Move,
            Self::NoDrop => Shape::NoDrop,
            Self::NotAllowed => Shape::NotAllowed,
            Self::Grab => Shape::Grab,
            Self::Grabbing => Shape::Grabbing,
            Self::EResize => Shape::EResize,
            Self::NResize => Shape::NResize,
            Self::NeResize => Shape::NeResize,
            Self::NwResize => Shape::NwResize,
            Self::SResize => Shape::SResize,
            Self::SeResize => Shape::SeResize,
            Self::SwResize => Shape::SwResize,
            Self::WResize => Shape::WResize,
            Self::EwResize => Shape::EwResize,
            Self::NsResize => Shape::NsResize,
            Self::NeswResize => Shape::NeswResize,
            Self::NwseResize => Shape::NwseResize,
            Self::ColResize => Shape::ColResize,
            Self::RowResize => Shape::RowResize,
            Self::AllScroll => Shape::AllScroll,
            Self::ZoomIn => Shape::ZoomIn,
            Self::ZoomOut => Shape::ZoomOut,
        }
    }
}

/// The manager (bound whenever advertised, destroyed on removal or shutdown) and
/// the shape each live window asked for.
#[derive(Default)]
pub(super) struct Cursors {
    manager: Option<(u32, WpCursorShapeManagerV1)>,
    shapes: BTreeMap<WindowId, CursorShape>,
}

impl Cursors {
    pub(super) fn device(
        &self,
        pointer: &WlPointer,
        qh: &QueueHandle<WindowState>,
    ) -> Option<WpCursorShapeDeviceV1> {
        let (_, manager) = self.manager.as_ref()?;
        Some(manager.get_pointer(pointer, qh, ()))
    }
    /// Window-scoped shapes end with the window; `None` ends them all.
    pub(super) fn forget(&mut self, window: Option<WindowId>) {
        match window {
            Some(window) => {
                self.shapes.remove(&window);
            }
            None => self.shapes.clear(),
        }
    }
    pub(super) fn destroy(&mut self) {
        if let Some((_, manager)) = self.manager.take() {
            manager.destroy();
        }
    }
}

impl Seat {
    /// Apply the stored shape of the window the pointer is in, with its enter serial.
    fn apply_stored(&self, shapes: &BTreeMap<WindowId, CursorShape>) {
        if let (Some(device), Some(window), Some(serial)) =
            (&self.device, self.pointer_focus, self.enter)
            && let Some(shape) = shapes.get(&window)
        {
            device.set_shape(serial, shape.native());
        }
    }
}

impl InputSeats {
    /// Bind the manager when advertised (also after discovery) and equip pointers
    /// that already exist.
    pub(in crate::window) fn bind_cursor_manager(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if interface != "wp_cursor_shape_manager_v1"
            || version == 0
            || self.cursor.manager.is_some()
        {
            return;
        }
        self.cursor.manager = Some((name, registry.bind(name, 1, qh, ())));
        for seat in self.seats.values_mut() {
            if let (Some(pointer), None) = (&seat.pointer, &seat.device) {
                seat.device = self.cursor.device(pointer, qh);
                seat.apply_stored(&self.cursor.shapes);
            }
        }
    }
    /// A removed manager is destroyed; existing devices stay until their pointer goes.
    pub(in crate::window) fn remove_cursor_manager(&mut self, name: u32) {
        if self
            .cursor
            .manager
            .as_ref()
            .is_some_and(|(bound, _)| *bound == name)
        {
            self.cursor.destroy();
        }
    }
    /// The seat's pointer entered a window: apply that window's stored shape.
    pub(super) fn apply_cursor(&self, seat: u32) {
        if self.cursor.manager.is_some() {
            self.seats[&seat].apply_stored(&self.cursor.shapes);
        }
    }
}

impl Host {
    /// Set the cursor shape shown while a pointer is over `window`.
    ///
    /// The shape is retained until the window closes. It is applied at once on
    /// every seat whose pointer is currently in the window, and again on every
    /// later pointer enter. Until a shape is set nothing is requested and the
    /// compositor decides. Without `wp_cursor_shape_manager_v1` (absent, or removed
    /// by the compositor) this returns `Unsupported`; there is no fallback. A
    /// retired or foreign window returns `InvalidInput`. Nothing is sent on error.
    pub fn set_cursor_shape(&mut self, window: WindowId, shape: CursorShape) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_cursor_shape(window, shape))
    }
}

impl WindowState {
    pub(crate) fn set_cursor_shape(&mut self, window: WindowId, shape: CursorShape) -> Result<()> {
        self.ensure_active()?;
        self.record(window)?;
        if self.input.cursor.manager.is_none() {
            return Err(Error::Unsupported("wp_cursor_shape_v1"));
        }
        self.input.cursor.shapes.insert(window, shape);
        for seat in self.input.seats.values() {
            if seat.pointer_focus == Some(window) {
                seat.apply_stored(&self.input.cursor.shapes);
            }
        }
        if let Some(error) = self.connection.backend().last_error() {
            let error = Error::Wayland(error.to_string());
            self.remember(&error);
            return Err(error);
        }
        Ok(())
    }
}

delegate_noop!(WindowState: ignore WpCursorShapeManagerV1);
delegate_noop!(WindowState: ignore WpCursorShapeDeviceV1);
