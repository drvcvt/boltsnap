//! Bounded output scales and owned optional fractional/viewport dependencies.
mod identity;
mod protocol;
#[cfg(test)]
mod tests;

use super::{
    state::WindowState,
    types::{Configuration, WindowEvent, WindowId},
};
use crate::{Error, Result};
use identity::Identity;
pub use identity::{OutputId, OutputInfo};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{wl_output::WlOutput, wl_registry::WlRegistry, wl_surface::WlSurface},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::WpFractionalScaleV1,
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

const MAX_OUTPUTS: usize = 32;

struct Binding<P> {
    name: u32,
    proxy: P,
    available: AtomicBool,
}

impl<P> Binding<P> {
    fn new(name: u32, proxy: P) -> Self {
        Self {
            name,
            proxy,
            available: AtomicBool::new(true),
        }
    }
    fn remove(&self, name: u32) -> bool {
        name == self.name && self.available.swap(false, Ordering::AcqRel)
    }
}

/// Bound managers remain owned after global removal, through all native leases.
#[derive(Default)]
pub(super) struct ScaleGlobals {
    fractional: Option<Binding<WpFractionalScaleManagerV1>>,
    viewporter: Option<Binding<WpViewporter>>,
}

impl ScaleGlobals {
    pub(super) fn bind(
        &mut self,
        registry: &WlRegistry,
        global: (u32, &str, u32),
        qh: &QueueHandle<WindowState>,
    ) {
        let (name, interface, version) = global;
        if version == 0 {
            return;
        }
        match interface {
            "wp_fractional_scale_manager_v1" if self.fractional.is_none() => {
                self.fractional = Some(Binding::new(name, registry.bind(name, 1, qh, ())));
            }
            "wp_viewporter" if self.viewporter.is_none() => {
                self.viewporter = Some(Binding::new(name, registry.bind(name, 1, qh, ())));
            }
            _ => {}
        }
    }

    pub(super) fn remove(&self, name: u32) -> bool {
        let fractional = self
            .fractional
            .as_ref()
            .is_some_and(|value| value.remove(name));
        let viewport = self
            .viewporter
            .as_ref()
            .is_some_and(|value| value.remove(name));
        fractional || viewport
    }

    pub(super) fn fractional_available(&self) -> bool {
        self.fractional
            .as_ref()
            .is_some_and(|value| value.available.load(Ordering::Acquire))
            && self
                .viewporter
                .as_ref()
                .is_some_and(|value| value.available.load(Ordering::Acquire))
    }

    pub(super) fn create_surface(
        &self,
        surface: &WlSurface,
        qh: &QueueHandle<WindowState>,
        id: WindowId,
    ) -> Result<SurfaceScaleObjects> {
        if !self.fractional_available() {
            return Ok(SurfaceScaleObjects::default());
        }
        let (Some(fractional), Some(viewporter)) = (&self.fractional, &self.viewporter) else {
            return Err(Error::Unsupported(
                "fractional scaling requires both managers",
            ));
        };
        let objects = SurfaceScaleObjects {
            viewport: Some(viewporter.proxy.get_viewport(surface, qh, id)),
            fractional: Some(fractional.proxy.get_fractional_scale(surface, qh, id)),
        };
        if !objects.alive() {
            return Err(Error::Wayland("scale extension creation failed".into()));
        }
        Ok(objects)
    }

    /// Parent global teardown calls this before its final nonblocking flush.
    pub(super) fn destroy(&mut self) {
        if let Some(binding) = self.fractional.take() {
            binding.proxy.destroy();
        }
        if let Some(binding) = self.viewporter.take() {
            binding.proxy.destroy();
        }
    }
}

impl Drop for ScaleGlobals {
    fn drop(&mut self) {
        self.destroy();
    }
}

/// Root destruction calls destroy before wl_surface; Drop also covers partial setup.
#[derive(Default)]
pub(super) struct SurfaceScaleObjects {
    viewport: Option<WpViewport>,
    fractional: Option<WpFractionalScaleV1>,
}

impl SurfaceScaleObjects {
    pub(super) fn fractional(&self) -> bool {
        self.fractional.is_some()
    }
    pub(super) fn alive(&self) -> bool {
        self.viewport.as_ref().is_none_or(Proxy::is_alive)
            && self.fractional.as_ref().is_none_or(Proxy::is_alive)
    }

    /// Stage matching scale state only; the presenter owns the surface commit.
    pub(super) fn stage(&self, surface: &WlSurface, configuration: Configuration) -> Result<()> {
        if configuration.suspended {
            return Err(Error::NotReady);
        }
        let width = i32::try_from(configuration.width).map_err(|_| Error::InvalidDimensions)?;
        let height = i32::try_from(configuration.height).map_err(|_| Error::InvalidDimensions)?;
        if width <= 0 || height <= 0 {
            return Err(Error::InvalidDimensions);
        }
        let factor = if self.fractional() {
            1
        } else {
            if configuration.scale_120 == 0 || !configuration.scale_120.is_multiple_of(120) {
                return Err(Error::InvalidInput("fractional scale without viewport"));
            }
            i32::try_from(configuration.scale_120 / 120).map_err(|_| Error::InvalidDimensions)?
        };
        if surface.version() < 3 && factor != 1 {
            return Err(Error::Unsupported(
                "surface buffer scaling requires version 3",
            ));
        }
        if surface.version() >= 3 {
            surface.set_buffer_scale(factor);
        }
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(width, height);
        }
        Ok(())
    }

    pub(super) fn destroy(&mut self) {
        if let Some(fractional) = self.fractional.take() {
            fractional.destroy();
        }
        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }
    }
}

impl Drop for SurfaceScaleObjects {
    fn drop(&mut self) {
        self.destroy();
    }
}

pub(super) struct WindowScale {
    fractional_enabled: bool,
    fractional: Option<u32>,
    surface_version: u32,
    preferred: u32,
    entered: BTreeSet<u32>,
}

impl WindowScale {
    pub(super) fn new(surface_version: u32, fractional: bool) -> Self {
        Self {
            fractional_enabled: fractional,
            fractional: None,
            surface_version,
            preferred: 1,
            entered: BTreeSet::new(),
        }
    }
    pub(super) fn selected(&self, outputs: &OutputScales) -> Result<u32> {
        if let Some(scale) = self.fractional.filter(|_| self.fractional_enabled) {
            return Ok(scale);
        }
        let factor = if self.surface_version >= 6 {
            self.preferred
        } else if self.surface_version < 3 {
            1
        } else {
            self.entered
                .iter()
                .filter_map(|name| outputs.records.get(name))
                .map(|output| output.current)
                .max()
                .unwrap_or(1)
        };
        factor.checked_mul(120).ok_or(Error::InvalidDimensions)
    }
}

struct OutputRecord {
    proxy: WlOutput,
    staged: u32,
    current: u32,
    identity: Identity,
}

impl Drop for OutputRecord {
    fn drop(&mut self) {
        // Versions 1/2 have no wire destructor; their objects live with the connection.
        if self.proxy.version() >= 3 {
            self.proxy.release();
        }
    }
}

#[derive(Default)]
pub(super) struct OutputScales {
    records: BTreeMap<u32, OutputRecord>,
    // Old protocol versions cannot release bound objects. Their connection-owned
    // resource credits remain charged after global removal, preventing hotplug
    // churn from silently exceeding the same native output admission bound.
    retired_legacy: usize,
    // Last issued output identity sequence; identities are never reused.
    issued: u64,
}

impl OutputScales {
    /// Queue modern output releases before the stopped host's final flush.
    /// Legacy bindings cannot be released; their credits remain charged for the
    /// connection lifetime even when shutdown discards their local records.
    pub(super) fn clear(&mut self) {
        self.retired_legacy += self
            .records
            .values()
            .filter(|output| output.proxy.version() < 3)
            .count();
        self.records.clear();
    }

    fn bind(
        &mut self,
        registry: &WlRegistry,
        (name, version, connection): (u32, u32, u64),
        qh: &QueueHandle<WindowState>,
    ) -> Result<()> {
        if version == 0 || self.records.contains_key(&name) {
            return Ok(());
        }
        if self.records.len() + self.retired_legacy >= MAX_OUTPUTS {
            return Err(Error::LimitExceeded);
        }
        let sequence = self.issued.checked_add(1).ok_or(Error::LimitExceeded)?;
        self.issued = sequence;
        self.records.insert(
            name,
            OutputRecord {
                proxy: registry.bind(name, version.min(4), qh, name),
                staged: 1,
                current: 1,
                identity: Identity::new(OutputId {
                    connection,
                    sequence,
                }),
            },
        );
        Ok(())
    }
    fn name(&self, proxy: &WlOutput) -> Option<u32> {
        self.records
            .iter()
            .find_map(|(name, output)| (&output.proxy == proxy).then_some(*name))
    }
}

/// Positive logical extent and scale; locked fractional XML requires nearest,
/// ties away from zero. u64 checked arithmetic avoids floating-point disagreement.
pub(super) fn pixel_extent(logical: u32, scale_120: u32) -> Result<u32> {
    if logical == 0 || logical > i32::MAX as u32 || scale_120 == 0 {
        return Err(Error::InvalidDimensions);
    }
    let scaled = u64::from(logical)
        .checked_mul(u64::from(scale_120))
        .and_then(|value| value.checked_add(60))
        .ok_or(Error::InvalidDimensions)?
        / 120;
    if scaled == 0 || scaled > i32::MAX as u64 {
        return Err(Error::InvalidDimensions);
    }
    u32::try_from(scaled).map_err(|_| Error::InvalidDimensions)
}

pub(super) fn bind_output(
    state: &mut WindowState,
    registry: &WlRegistry,
    global: (u32, &str, u32),
    qh: &QueueHandle<WindowState>,
) {
    if global.1 == "wl_output"
        && let Err(error) =
            state
                .outputs
                .bind(registry, (global.0, global.2, state.connection_id), qh)
    {
        state.remember(&error);
    }
}

pub(super) fn remove_output(state: &mut WindowState, name: u32) {
    let Some(output) = state.outputs.records.remove(&name) else {
        return;
    };
    if output.proxy.version() < 3 {
        state.outputs.retired_legacy += 1;
    }
    let announced = output.identity.announced.then_some(output.identity.id);
    drop(output);
    if let Some(id) = announced {
        state.push(WindowEvent::OutputRemoved(id));
    }
    let affected: Vec<_> = state
        .windows
        .iter_mut()
        .filter_map(|(id, record)| record.scale.entered.remove(&name).then_some(*id))
        .collect();
    for id in affected {
        super::protocol::refresh_scale(state, id);
    }
}
