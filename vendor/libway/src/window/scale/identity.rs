//! Public output identity from `wl_output` v4 `name`/`description`, announced
//! after the output's first `done` and retired on global removal.
use super::{super::Host, OutputScales};
use crate::{Error, Result};
use wayland_client::protocol::wl_output::WlOutput;

/// Longest accepted name or description; longer values are reported as absent.
const MAX_TEXT_BYTES: usize = 256;

/// An output identity with connection provenance, never reused during that connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputId {
    pub(crate) connection: u64,
    pub(crate) sequence: u64,
}

/// What the compositor said about one output. Geometry, modes and transform
/// are not reported.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutputInfo {
    /// The identity used to target this output, for example with a layer surface.
    pub id: OutputId,
    /// Connector name such as `DP-3` (`wl_output` v4), if announced and at most 256 bytes.
    pub name: Option<String>,
    /// Human-readable description (`wl_output` v4), if announced and at most 256 bytes.
    pub description: Option<String>,
}

/// Per-record identity state. Name and description stay current after the
/// announcement; changes after it publish no event.
pub(super) struct Identity {
    pub(super) id: OutputId,
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) announced: bool,
}

impl Identity {
    pub(super) fn new(id: OutputId) -> Self {
        Self {
            id,
            name: None,
            description: None,
            announced: false,
        }
    }

    pub(super) fn info(&self) -> OutputInfo {
        OutputInfo {
            id: self.id,
            name: self.name.clone(),
            description: self.description.clone(),
        }
    }
}

/// Bounded text without an interior NUL, otherwise absent.
pub(super) fn text(value: String) -> Option<String> {
    (value.len() <= MAX_TEXT_BYTES && !value.contains('\0')).then_some(value)
}

impl OutputScales {
    /// Announced outputs in bind order.
    pub(in crate::window) fn infos(&self) -> Vec<OutputInfo> {
        let mut infos: Vec<_> = self
            .records
            .values()
            .filter(|record| record.identity.announced)
            .map(|record| record.identity.info())
            .collect();
        infos.sort_by_key(|info| info.id);
        infos
    }

    /// The live proxy of an announced output owned by this connection.
    pub(in crate::window) fn announced(&self, connection: u64, id: OutputId) -> Result<&WlOutput> {
        if id.connection != connection {
            return Err(Error::InvalidInput("output belongs to another connection"));
        }
        self.records
            .values()
            .find(|record| record.identity.announced && record.identity.id == id)
            .map(|record| &record.proxy)
            .ok_or(Error::InvalidInput("unknown or removed output"))
    }
}

impl Host {
    /// Outputs announced so far (after their first `done`), in bind order.
    /// Outputs may be announced after [`super::super::WindowEvent::Ready`];
    /// follow [`super::super::WindowEvent::OutputAdded`] and `OutputRemoved`.
    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.coordinator.state.outputs.infos()
    }
}
