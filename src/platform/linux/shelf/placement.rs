//! Which output the shelf sits on: the focused monitor from Hyprland, the
//! outputs Wayland knows, and the layout that follows.

use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{KeyboardInteractivity, Layer};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_output;

use crate::record::finalize::FinalizedClip;
use crate::shelf::layout::Layout;

use super::cards::visible_cards;
use super::events::DaemonEvent;
use super::start::{resolve_output_name, transformed_mode_size};
use super::{Daemon, cards};

pub(super) fn parse_focus_snapshot(json: &[u8]) -> Result<Vec<crate::record::Monitor>, String> {
    crate::record::parse_hyprland_monitors(json)
}

pub(super) fn focused_output_from_hyprland_json(json: &[u8]) -> Option<String> {
    parse_focus_snapshot(json)
        .ok()?
        .into_iter()
        .find(|monitor| monitor.focused)
        .map(|monitor| monitor.name)
}

fn query_focus_snapshot() -> Result<Vec<crate::record::Monitor>, String> {
    let reply =
        super::super::hypr::json("monitors").ok_or("Hyprland monitor query is unavailable")?;
    parse_focus_snapshot(&reply)
}

pub(super) fn spawn_focus_query(tx: calloop::channel::Sender<DaemonEvent>) {
    std::thread::spawn(move || {
        let _ = tx.send(DaemonEvent::FocusResolved(query_focus_snapshot()));
    });
}

pub(super) fn monitor_for_geometry<'a>(
    monitors: &'a [crate::record::Monitor],
    geo: &crate::record::Geometry,
) -> Option<&'a crate::record::Monitor> {
    let x = i64::from(geo.x) + i64::from(geo.w / 2);
    let y = i64::from(geo.y) + i64::from(geo.h / 2);
    monitors.iter().find(|monitor| {
        let scale = monitor.scale.max(1.0);
        let width = (f64::from(monitor.width) / scale).round() as i64;
        let height = (f64::from(monitor.height) / scale).round() as i64;
        let left = i64::from(monitor.x);
        let top = i64::from(monitor.y);
        x >= left && x < left + width && y >= top && y < top + height
    })
}

/// Monitor for new video cards: the display a clip was recorded from, the way a
/// screenshot's card follows its capture. Combined and area clips name no single
/// output; they go to the focused monitor instead of wherever the shelf last was.
pub(super) fn card_output(
    clips: &[FinalizedClip],
    focused: impl FnOnce() -> Option<String>,
) -> Option<String> {
    clips
        .iter()
        .rev()
        .find_map(|clip| clip.output.clone())
        .or_else(focused)
}

/// Name of the focused Hyprland monitor. `None` off Hyprland (then the
/// compositor places the shelf on its default output).
pub(crate) fn focused_monitor_name() -> Option<String> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        // No Hyprland: on Plasma ask KWin over D-Bus, elsewhere give up.
        return if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("KDE")) {
            crate::platform::portal::kwin_active_output_name()
        } else {
            None
        };
    }
    focused_output_from_hyprland_json(&super::super::hypr::json("monitors")?)
}

/// Logical layout origin (`x`, `y`) of the focused Hyprland monitor. Used to map a selection rect (overlay-output-local
/// logical px) into compositor-global coords for recording. `None` off Hyprland.
pub(crate) fn focused_monitor_origin() -> Option<(i32, i32)> {
    let v: serde_json::Value =
        serde_json::from_slice(&super::super::hypr::json("monitors")?).ok()?;
    for m in v.as_array()? {
        if m.get("focused").and_then(|f| f.as_bool()) == Some(true) {
            let x = m.get("x").and_then(|n| n.as_i64())? as i32;
            let y = m.get("y").and_then(|n| n.as_i64())? as i32;
            return Some((x, y));
        }
    }
    None
}

impl Daemon {
    pub(super) fn cached_monitors(&self) -> Vec<crate::record::Monitor> {
        self.output_state
            .outputs()
            .filter_map(|output| self.output_state.info(&output))
            .filter_map(|info| {
                let name = info.name?;
                let (x, y) = info.logical_position.unwrap_or(info.location);
                let logical_size = info.logical_size.filter(|(w, h)| *w > 0 && *h > 0);
                let mode_size = info
                    .modes
                    .iter()
                    .find(|mode| mode.current)
                    .map(|mode| transformed_mode_size(mode.dimensions, info.transform))
                    .filter(|(w, h)| *w > 0 && *h > 0);
                let fallback_scale = f64::from(info.scale_factor.max(1));
                let scale = match (mode_size, logical_size) {
                    (Some((width, _)), Some((logical_width, _))) => {
                        f64::from(width) / f64::from(logical_width)
                    }
                    _ => fallback_scale,
                };
                let (width, height) = mode_size.or_else(|| {
                    logical_size.map(|(width, height)| {
                        (
                            (f64::from(width) * scale).round() as i32,
                            (f64::from(height) * scale).round() as i32,
                        )
                    })
                })?;
                Some(crate::record::Monitor {
                    focused: self.focused_output.as_deref() == Some(name.as_str()),
                    description: info
                        .description
                        .unwrap_or_else(|| format!("{} {}", info.make, info.model)),
                    name,
                    x,
                    y,
                    width: width as u32,
                    height: height as u32,
                    scale,
                })
            })
            .collect()
    }

    pub(super) fn target_output(
        &self,
        requested: Option<&str>,
    ) -> (Option<String>, Option<wl_output::WlOutput>) {
        let outputs: Vec<_> = self.output_state.outputs().collect();
        let available: Vec<_> = outputs
            .iter()
            .filter_map(|output| self.output_state.info(output).and_then(|info| info.name))
            .collect();
        let requested = resolve_output_name(requested, &available);
        let output = requested
            .and_then(|n| {
                outputs
                    .iter()
                    .find(|o| self.output_state.info(o).and_then(|i| i.name).as_deref() == Some(n))
                    .cloned()
            })
            // Fall back to the first available output when the focused monitor
            // can't be resolved. Hyprland may fail to map a layer surface with a
            // null output, so always pass a concrete one when we have it.
            .or_else(|| outputs.into_iter().next());
        let name = output
            .as_ref()
            .and_then(|output| self.output_state.info(output))
            .and_then(|info| info.name);
        (name, output)
    }

    /// Ensure the shelf surface lives on the currently focused output. Recreates
    /// the layer surface (dropping the old one, which unmaps it) when the focused
    /// monitor changed since last time. Returns true when a fresh surface was
    /// created and must wait for its initial configure before drawing.
    pub(super) fn place_on_output(
        &mut self,
        requested: Option<&str>,
        qh: &QueueHandle<Self>,
    ) -> bool {
        let requested = requested.or(self.output_name.as_deref());
        let (name, output) = self.target_output(requested);
        if self.layer.is_some() && name == self.output_name {
            return false;
        }
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("boltsnap"),
            output.as_ref(),
        );
        let (anchor, [top, right, bottom, left]) = cards::anchor(self.shelf_prefs.corner);
        layer.set_anchor(anchor);
        layer.set_margin(top, right, bottom, left);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);
        layer.set_size(self.layout.width.max(1), self.layout.height.max(1));
        layer.commit();
        self.layer = Some(layer); // old layer dropped here -> unmaps from prior output
        self.output_name = name;
        self.shelf_configured = false;
        self.shelf_pending_draw = false;
        // A callback on the old surface never fires.
        self.frame_requested = None;
        true
    }

    /// Recompute layout from the model and resize the layer surface to match.
    pub(super) fn relayout(&mut self) {
        let sizes: Vec<(u64, u32, u32)> = visible_cards(&self.model, self.shelf_prefs.visible)
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        self.layout = Layout::compute(&sizes, &self.cfg);
        // We are a self-sized layer surface: we dictate our size via set_size and
        // the compositor echoes it back in configure. Track it directly so draw()
        // paints at the new size immediately — relying only on the configure
        // round-trip deadlocked the surface at the startup 1x1 (the post-set_size
        // commit carried a stale-sized buffer, so Hyprland never reconfigured).
        self.width = self.layout.width.max(1);
        self.height = self.layout.height.max(1);
        if let Some(layer) = self.layer.as_ref() {
            layer.set_size(self.layout.width, self.layout.height);
        }
    }
}
