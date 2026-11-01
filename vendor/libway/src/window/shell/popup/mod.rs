//! `xdg_popup` windows (R6): placement types, validation and the positioner.
//!
//! A popup is an ordinary [`WindowId`] whose parent is a toplevel, a layer or
//! another popup of the same host. Creation, the grab rule and the close
//! cascade live in `tree`; the popup's own events in `protocol`.
mod protocol;
mod tree;

use super::{region::LogicalRect, state::WindowState, types::WindowId};
use crate::window::InputSeatId;
use crate::{Error, Result};
use wayland_client::QueueHandle;
use wayland_protocols::xdg::shell::client::{
    xdg_positioner::{self, XdgPositioner},
    xdg_wm_base::XdgWmBase,
};

/// Point on the anchor rectangle the popup is placed against (wire `anchor`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupAnchor {
    /// Wire `none`: the center of the anchor rectangle.
    #[default]
    Center,
    /// Middle of the top edge.
    Top,
    /// Middle of the bottom edge.
    Bottom,
    /// Middle of the left edge.
    Left,
    /// Middle of the right edge.
    Right,
    /// Top left corner.
    TopLeft,
    /// Bottom left corner.
    BottomLeft,
    /// Top right corner.
    TopRight,
    /// Bottom right corner.
    BottomRight,
}

/// Direction the popup extends from the anchor point (wire `gravity`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PopupGravity {
    /// Wire `none`: centered over the anchor point.
    #[default]
    Center,
    /// Extends upward.
    Top,
    /// Extends downward.
    Bottom,
    /// Extends to the left.
    Left,
    /// Extends to the right.
    Right,
    /// Extends up and to the left.
    TopLeft,
    /// Extends down and to the left.
    BottomLeft,
    /// Extends up and to the right.
    TopRight,
    /// Extends down and to the right.
    BottomRight,
}

/// What the compositor may do when the popup would be cut off. Combine with
/// `|`. The protocol applies them in the order flip, slide, resize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConstraintAdjustment(u8);

impl ConstraintAdjustment {
    /// No adjustment.
    pub const NONE: Self = Self(0);
    /// Slide along the x axis.
    pub const SLIDE_X: Self = Self(1);
    /// Slide along the y axis.
    pub const SLIDE_Y: Self = Self(2);
    /// Flip anchor and gravity on the x axis.
    pub const FLIP_X: Self = Self(4);
    /// Flip anchor and gravity on the y axis.
    pub const FLIP_Y: Self = Self(8);
    /// Shrink along the x axis.
    pub const RESIZE_X: Self = Self(16);
    /// Shrink along the y axis.
    pub const RESIZE_Y: Self = Self(32);

    /// Whether every bit of `other` is set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for ConstraintAdjustment {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Everything a popup needs at creation. The placement fields become one
/// `xdg_positioner`, which the compositor copies; libway destroys it right
/// after use. Build it as `PopupOptions { size, anchor_rect, ..PopupOptions::new(parent) }`
/// so fields added later stay non-breaking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupOptions {
    /// A live toplevel, layer or popup of this host that has shown a frame.
    pub parent: WindowId,
    /// Logical popup size (width, height), each 1..=`i32::MAX`.
    pub size: (u32, u32),
    /// Rectangle in the parent's surface-local logical coordinates, at least
    /// 1x1 and inside the parent's current size.
    pub anchor_rect: LogicalRect,
    /// Point on `anchor_rect` the popup is placed against.
    pub anchor: PopupAnchor,
    /// Direction the popup extends from the anchor point.
    pub gravity: PopupGravity,
    /// Allowed adjustments when the popup would be cut off.
    pub constraint_adjustment: ConstraintAdjustment,
    /// Shift (x, y) after anchoring, also used for constraint testing. Hyprland
    /// does not flip it.
    pub offset: (i32, i32),
    /// Take an explicit grab with this seat's latest press in the parent's
    /// popup tree. `None` for tooltips and other passive popups. Fixed at
    /// creation.
    pub grab: Option<InputSeatId>,
    /// Ask to be placed again when the parent moves (needs `xdg_wm_base` v3).
    /// Hyprland ignores it.
    pub reactive: bool,
}

impl PopupOptions {
    /// The protocol defaults plus the smallest complete size and anchor
    /// rectangle: 1x1 at (0, 0), anchor and gravity `Center`, no adjustment,
    /// offset 0, no grab, not reactive.
    pub fn new(parent: WindowId) -> Self {
        Self {
            parent,
            size: (1, 1),
            anchor_rect: LogicalRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            anchor: PopupAnchor::Center,
            gravity: PopupGravity::Center,
            constraint_adjustment: ConstraintAdjustment::NONE,
            offset: (0, 0),
            grab: None,
            reactive: false,
        }
    }
}

/// The placed rectangle of a popup, relative to its parent's surface-local
/// logical origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PopupPlacement {
    /// Left edge relative to the parent.
    pub x: i32,
    /// Top edge relative to the parent.
    pub y: i32,
    /// Placed logical width; equals the configuration's `width`.
    pub width: u32,
    /// Placed logical height; equals the configuration's `height`.
    pub height: u32,
    /// The [`super::Host::reposition_popup`] token this configuration
    /// completes, if any.
    pub repositioned: Option<u32>,
}

/// Per-popup state kept on its window record.
#[derive(Clone, Copy, Debug)]
pub(in crate::window) struct PopupRecord {
    /// Created with an explicit grab.
    pub(in crate::window) grab: bool,
    /// Next reposition token; never 0.
    pub(in crate::window) next_token: u32,
    /// The last `xdg_popup.configure` (x, y, width, height). A later bare
    /// `xdg_surface.configure` reuses it.
    pub(in crate::window) placed: Option<(i32, i32, u32, u32)>,
    /// A `repositioned` token waiting for the next completed sequence.
    pub(in crate::window) token: Option<u32>,
}

impl PopupRecord {
    pub(in crate::window) fn new(grab: bool) -> Self {
        Self {
            grab,
            next_token: 1,
            placed: None,
            token: None,
        }
    }
}

/// Placement rules checked before any request. `parent_size` is the parent's
/// current logical size.
pub(in crate::window) fn validate(
    options: &PopupOptions,
    parent_size: (u32, u32),
    shell_version: u32,
) -> Result<()> {
    let (width, height) = options.size;
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(Error::InvalidDimensions);
    }
    let rect = options.anchor_rect;
    let inside = rect.width >= 1
        && rect.height >= 1
        && rect.x >= 0
        && rect.y >= 0
        && i64::from(rect.x) + i64::from(rect.width) <= i64::from(parent_size.0)
        && i64::from(rect.y) + i64::from(rect.height) <= i64::from(parent_size.1);
    if !inside {
        return Err(Error::InvalidInput(
            "popup anchor rectangle must lie inside the parent",
        ));
    }
    if options.reactive && shell_version < 3 {
        return Err(Error::Unsupported("reactive popups need xdg_wm_base v3"));
    }
    Ok(())
}

/// Send one positioner for `options`; the caller destroys it after use.
pub(in crate::window) fn positioner(
    shell: &XdgWmBase,
    qh: &QueueHandle<WindowState>,
    options: &PopupOptions,
) -> XdgPositioner {
    let positioner = shell.create_positioner(qh, ());
    positioner.set_size(options.size.0 as i32, options.size.1 as i32);
    let rect = options.anchor_rect;
    positioner.set_anchor_rect(rect.x, rect.y, rect.width as i32, rect.height as i32);
    positioner.set_anchor(native_anchor(options.anchor));
    positioner.set_gravity(native_gravity(options.gravity));
    positioner.set_constraint_adjustment(xdg_positioner::ConstraintAdjustment::from_bits_truncate(
        u32::from(options.constraint_adjustment.0),
    ));
    positioner.set_offset(options.offset.0, options.offset.1);
    if options.reactive {
        positioner.set_reactive();
    }
    positioner
}

fn native_anchor(anchor: PopupAnchor) -> xdg_positioner::Anchor {
    use xdg_positioner::Anchor;
    match anchor {
        PopupAnchor::Center => Anchor::None,
        PopupAnchor::Top => Anchor::Top,
        PopupAnchor::Bottom => Anchor::Bottom,
        PopupAnchor::Left => Anchor::Left,
        PopupAnchor::Right => Anchor::Right,
        PopupAnchor::TopLeft => Anchor::TopLeft,
        PopupAnchor::BottomLeft => Anchor::BottomLeft,
        PopupAnchor::TopRight => Anchor::TopRight,
        PopupAnchor::BottomRight => Anchor::BottomRight,
    }
}

fn native_gravity(gravity: PopupGravity) -> xdg_positioner::Gravity {
    use xdg_positioner::Gravity;
    match gravity {
        PopupGravity::Center => Gravity::None,
        PopupGravity::Top => Gravity::Top,
        PopupGravity::Bottom => Gravity::Bottom,
        PopupGravity::Left => Gravity::Left,
        PopupGravity::Right => Gravity::Right,
        PopupGravity::TopLeft => Gravity::TopLeft,
        PopupGravity::BottomLeft => Gravity::BottomLeft,
        PopupGravity::TopRight => Gravity::TopRight,
        PopupGravity::BottomRight => Gravity::BottomRight,
    }
}
