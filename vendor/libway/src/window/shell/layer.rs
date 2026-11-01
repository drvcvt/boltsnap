//! Layer-shell surfaces (`zwlr_layer_shell_v1`): overlays, HUDs, panels and
//! selectors. A layer is an ordinary [`WindowId`]: configurations, presenters,
//! input, presentation feedback and close work as for toplevels.
use super::{Host, OutputId, state::WindowState, types::WindowId};
use crate::{Error, Result};
use wayland_client::Proxy;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Longest accepted namespace in bytes.
const MAX_NAMESPACE_BYTES: usize = 256;

/// Stacking layer, bottom-most first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Layer {
    /// Below everything, for wallpapers.
    Background,
    /// Below ordinary windows.
    Bottom,
    /// Above ordinary windows, for panels and HUDs.
    Top,
    /// Above everything including fullscreen windows, for overlays and selectors.
    Overlay,
}

/// Output edges a layer surface is anchored to. Combine with `|`. Two orthogonal
/// edges anchor to that corner, one edge centers on it, none centers on the output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Anchor(u8);

impl Anchor {
    /// Not anchored: centered on the output.
    pub const NONE: Self = Self(0);
    /// The top edge.
    pub const TOP: Self = Self(1);
    /// The bottom edge.
    pub const BOTTOM: Self = Self(2);
    /// The left edge.
    pub const LEFT: Self = Self(4);
    /// The right edge.
    pub const RIGHT: Self = Self(8);
    /// All four edges, for full-output surfaces.
    pub const ALL: Self = Self(15);

    /// Whether every edge of `other` is part of this anchor.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Anchor {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// One output edge, for [`LayerState::exclusive_edge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Edge {
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
}

impl Edge {
    fn anchor(self) -> Anchor {
        match self {
            Self::Top => Anchor::TOP,
            Self::Bottom => Anchor::BOTTOM,
            Self::Left => Anchor::LEFT,
            Self::Right => Anchor::RIGHT,
        }
    }
}

/// How the surface interacts with other surfaces' exclusive zones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExclusiveZone {
    /// Wire 0: moved to avoid other surfaces' exclusive zones (notifications).
    Respect,
    /// Wire -1: not moved, extends to the anchored edges (wallpapers, overlays).
    Ignore,
    /// Positive wire value: reserve this many logical pixels from the anchored
    /// edge, margin included (panels). At most `i32::MAX`. The compositor
    /// treats it as zero unless anchored to one edge, or one edge and both
    /// perpendicular ones, or a corner with an [`Edge`] chosen.
    Reserve(u32),
}

/// Whether and how the surface takes keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyboardInteractivity {
    /// Never receives keyboard focus (protocol default). For HUDs, OSDs,
    /// notifications and panels.
    None,
    /// Takes all keyboard input while on the top or overlay layer; the user
    /// cannot type elsewhere while it exists. Hyprland also refuses focus to
    /// every window and routes the pointer to the surface, on any layer and any
    /// monitor. Only for short full-screen selectors, ideally one overlay per
    /// output, with an Escape path: with no output the compositor picks the
    /// monitor and may lock the keyboard from one the user is not looking at.
    Exclusive,
    /// Ordinary focus semantics, like a window: Hyprland focuses it when it
    /// maps and a click on another window takes focus away (watch for
    /// `KeyboardObservation::Leave`). For prompts and dialogs. Needs a bound
    /// layer shell of version 4.
    OnDemand,
}

/// The part of a layer surface that may change after creation, see
/// [`Host::set_layer_state`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerState {
    /// Stacking layer. Changing it later needs a bound layer shell of version 2.
    pub layer: Layer,
    /// Anchored output edges.
    pub anchor: Anchor,
    /// Logical width; 0 stretches between the left and right anchors, which
    /// must then both be set.
    pub width: u32,
    /// Logical height; 0 stretches between the top and bottom anchors, which
    /// must then both be set.
    pub height: u32,
    /// Margins from the anchored edges in wire order: top, right, bottom,
    /// left. Margins on edges that are not anchored have no effect.
    pub margin: [i32; 4],
    /// Interaction with exclusive zones.
    pub exclusive_zone: ExclusiveZone,
    /// The edge an exclusive zone applies to, only needed to disambiguate a
    /// corner anchor. Must be anchored. Needs a bound layer shell of version 5
    /// and cannot be cleared again once sent.
    pub exclusive_edge: Option<Edge>,
    /// Keyboard focus behavior.
    pub keyboard: KeyboardInteractivity,
}

/// A full-output overlay: overlay layer, anchored to all edges, 0x0 size, no
/// margins, [`ExclusiveZone::Ignore`], no exclusive edge, no keyboard focus.
impl Default for LayerState {
    fn default() -> Self {
        Self {
            layer: Layer::Overlay,
            anchor: Anchor::ALL,
            width: 0,
            height: 0,
            margin: [0; 4],
            exclusive_zone: ExclusiveZone::Ignore,
            exclusive_edge: None,
            keyboard: KeyboardInteractivity::None,
        }
    }
}

/// Everything [`Host::create_layer`] needs.
#[derive(Clone, Debug)]
pub struct LayerOptions {
    /// Purpose of the surface, used by compositor rules. At most 256 UTF-8
    /// bytes without an interior NUL. Fixed for the surface lifetime.
    pub namespace: String,
    /// `None` lets the compositor choose (usually the most recently used
    /// output). Fixed for the surface lifetime.
    pub output: Option<OutputId>,
    /// Initial state.
    pub state: LayerState,
}

impl Default for LayerOptions {
    fn default() -> Self {
        Self {
            namespace: "libway".into(),
            output: None,
            state: LayerState::default(),
        }
    }
}

impl Host {
    /// Create a layer surface after [`super::WindowEvent::Ready`]. Everything is
    /// validated before any request; then the role setup and the bufferless
    /// first commit are sent. The returned window then works like any other:
    /// wait for [`super::WindowEvent::Configured`] before presenting.
    ///
    /// Errors: `NotReady` before `Ready`; `Unsupported` without (or after
    /// removal of) `zwlr_layer_shell_v1`, or for a field the bound version lacks
    /// ([`KeyboardInteractivity::OnDemand`] below v4, an exclusive edge below
    /// v5); `InvalidInput` for a 0 size without both opposite anchors, an
    /// exclusive edge that is not anchored, a reserve above `i32::MAX`, a bad
    /// namespace, or an output that is unknown, removed or foreign;
    /// `InvalidDimensions` for a size above `i32::MAX`; `LimitExceeded` beyond 16
    /// windows. Nothing is sent on error.
    pub fn create_layer(&mut self, options: LayerOptions) -> Result<WindowId> {
        self.coordinator.mutate(|state| state.create_layer(options))
    }

    /// Replace a layer's state. Sends only the changed requests, then one
    /// bufferless commit so the change applies without a new frame; an
    /// unchanged state sends nothing. Validation and errors as for
    /// [`Host::create_layer`], plus `Unsupported` for a layer change below v2
    /// and `InvalidInput` for a toplevel or for clearing an exclusive edge.
    /// A size change arrives as a new configuration.
    pub fn set_layer_state(&mut self, window: WindowId, state: LayerState) -> Result<()> {
        self.coordinator
            .mutate(|owner| owner.set_layer_state(window, state))
    }

    /// The last state set on this layer. This is what libway requested, not a
    /// compositor confirmation; the configured size is in the configuration.
    /// A toplevel returns `InvalidInput`.
    pub fn layer_state(&self, window: WindowId) -> Result<LayerState> {
        if self.coordinator.failure().is_some() {
            return Err(Error::SessionStopped);
        }
        let state = &self.coordinator.state;
        state.ensure_live()?;
        state
            .record(window)?
            .layer
            .ok_or(Error::InvalidInput("window is not a layer"))
    }
}

/// Version-independent combination rules.
pub(super) fn validate_shape(state: &LayerState) -> Result<()> {
    if state.width > i32::MAX as u32 || state.height > i32::MAX as u32 {
        return Err(Error::InvalidDimensions);
    }
    if state.width == 0 && !state.anchor.contains(Anchor::LEFT | Anchor::RIGHT) {
        return Err(Error::InvalidInput(
            "zero layer width needs left and right anchors",
        ));
    }
    if state.height == 0 && !state.anchor.contains(Anchor::TOP | Anchor::BOTTOM) {
        return Err(Error::InvalidInput(
            "zero layer height needs top and bottom anchors",
        ));
    }
    if matches!(state.exclusive_zone, ExclusiveZone::Reserve(zone) if zone > i32::MAX as u32) {
        return Err(Error::InvalidInput("exclusive zone above i32::MAX"));
    }
    if let Some(edge) = state.exclusive_edge
        && !state.anchor.contains(edge.anchor())
    {
        return Err(Error::InvalidInput("exclusive edge is not anchored"));
    }
    Ok(())
}

/// Fields that need a newer bound layer-shell version.
pub(super) fn validate_version(state: &LayerState, version: u32) -> Result<()> {
    if state.keyboard == KeyboardInteractivity::OnDemand && version < 4 {
        return Err(Error::Unsupported(
            "on-demand keyboard interactivity needs layer shell version 4",
        ));
    }
    if state.exclusive_edge.is_some() && version < 5 {
        return Err(Error::Unsupported(
            "an exclusive edge needs layer shell version 5",
        ));
    }
    Ok(())
}

pub(super) fn validate_namespace(namespace: &str) -> Result<()> {
    if namespace.as_bytes().contains(&0) || namespace.len() > MAX_NAMESPACE_BYTES {
        return Err(Error::InvalidInput("invalid layer namespace"));
    }
    Ok(())
}

pub(super) fn native_layer(layer: Layer) -> zwlr_layer_shell_v1::Layer {
    match layer {
        Layer::Background => zwlr_layer_shell_v1::Layer::Background,
        Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        Layer::Top => zwlr_layer_shell_v1::Layer::Top,
        Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

fn native_anchor(anchor: Anchor) -> zwlr_layer_surface_v1::Anchor {
    zwlr_layer_surface_v1::Anchor::from_bits_truncate(u32::from(anchor.0))
}

fn native_zone(zone: ExclusiveZone) -> i32 {
    match zone {
        ExclusiveZone::Respect => 0,
        ExclusiveZone::Ignore => -1,
        // validate_shape bounds the reserve to i32::MAX.
        ExclusiveZone::Reserve(zone) => i32::try_from(zone).unwrap_or(i32::MAX),
    }
}

fn native_keyboard(
    keyboard: KeyboardInteractivity,
) -> zwlr_layer_surface_v1::KeyboardInteractivity {
    match keyboard {
        KeyboardInteractivity::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
        KeyboardInteractivity::Exclusive => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
        KeyboardInteractivity::OnDemand => zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
    }
}

/// Send every request whose field differs from `old`; `None` sends all of them
/// in creation order. Returns whether anything was sent.
pub(super) fn send_state(
    surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    old: Option<&LayerState>,
    new: &LayerState,
) -> bool {
    let changed = |field: &dyn Fn(&LayerState) -> bool| old.is_none_or(field);
    let mut sent = false;
    if changed(&|old: &LayerState| (old.width, old.height) != (new.width, new.height)) {
        surface.set_size(new.width, new.height);
        sent = true;
    }
    if changed(&|old: &LayerState| old.anchor != new.anchor) {
        surface.set_anchor(native_anchor(new.anchor));
        sent = true;
    }
    if changed(&|old: &LayerState| old.exclusive_zone != new.exclusive_zone) {
        surface.set_exclusive_zone(native_zone(new.exclusive_zone));
        sent = true;
    }
    if changed(&|old: &LayerState| old.margin != new.margin) {
        let [top, right, bottom, left] = new.margin;
        surface.set_margin(top, right, bottom, left);
        sent = true;
    }
    if changed(&|old: &LayerState| old.keyboard != new.keyboard) {
        surface.set_keyboard_interactivity(native_keyboard(new.keyboard));
        sent = true;
    }
    if let Some(edge) = new.exclusive_edge
        && changed(&|old: &LayerState| old.exclusive_edge != new.exclusive_edge)
    {
        surface.set_exclusive_edge(native_anchor(edge.anchor()));
        sent = true;
    }
    // The initial layer is a get_layer_surface argument.
    if old.is_some_and(|old| old.layer != new.layer) {
        surface.set_layer(native_layer(new.layer));
        sent = true;
    }
    sent
}

impl WindowState {
    pub(crate) fn set_layer_state(&mut self, window: WindowId, new: LayerState) -> Result<()> {
        self.ensure_active()?;
        let record = self.record(window)?;
        let old = record
            .layer
            .ok_or(Error::InvalidInput("window is not a layer"))?;
        let surface = record.native.layer()?;
        validate_shape(&new)?;
        validate_version(&new, surface.version())?;
        if old.layer != new.layer && surface.version() < 2 {
            return Err(Error::Unsupported(
                "changing the layer needs layer shell version 2",
            ));
        }
        if old.exclusive_edge.is_some() && new.exclusive_edge.is_none() {
            return Err(Error::InvalidInput("an exclusive edge cannot be cleared"));
        }
        if send_state(surface, Some(&old), &new) {
            record.native.surface().commit();
        }
        if let Some(record) = self.windows.get_mut(&window) {
            record.layer = Some(new);
        }
        self.requested()
    }
}
