//! Explicit owner mutations; no legacy reader or session reference escapes.
use super::CoordinatedDnd;
use crate::{
    Error, Result, SurfaceHandle,
    dnd::{
        DragData, DragId, DragRequest, LocalRect, Outcome, OutgoingLimits, TargetId, TargetSpec,
        TransferId,
    },
};

impl CoordinatedDnd {
    /// Register a target with the legacy session's validation and capacity rules.
    /// Unregister it before destroying its surface.
    pub fn register_target(
        &mut self,
        surface: &SurfaceHandle,
        spec: TargetSpec,
    ) -> Result<TargetId> {
        self.ensure_active()?;
        let result = self.dnd.register_target(surface, spec);
        self.finish_request(result)
    }

    /// Update geometry or enabled state; stale and foreign targets are rejected.
    pub fn update_target(
        &mut self,
        id: TargetId,
        rect: Option<LocalRect>,
        enabled: Option<bool>,
    ) -> Result<()> {
        self.ensure_active()?;
        let result = self.dnd.update_target(id, rect, enabled);
        self.finish_request(result)
    }

    /// Remove a target and abort its reading drops. Already delivered drops still
    /// require completion; unknown targets leave the participant usable.
    pub fn unregister_target(&mut self, id: TargetId) -> Result<()> {
        self.ensure_active()?;
        let result = self.dnd.unregister_target(id);
        self.finish_request(result)
    }

    /// Answer a delivered drop. Invalid outcomes leave it available for correction.
    pub fn complete(&mut self, transfer: TransferId, outcome: Outcome) -> Result<()> {
        self.ensure_active()?;
        let result = self.dnd.complete(transfer, outcome);
        self.finish_request(result)
    }

    /// Start an eager outgoing drag with explicit payload bounds. Lazy providers
    /// are rejected before preparation or replacement of an existing drag.
    /// Preparation is synchronous; I/O failures terminalize this participant.
    pub fn start_drag(&mut self, request: DragRequest, limits: OutgoingLimits) -> Result<DragId> {
        self.ensure_active()?;
        if matches!(&request.data, DragData::Lazy { .. }) {
            return Err(Error::Unsupported("lazy providers in coordinated DnD"));
        }
        let result = self.dnd.start_drag_bounded(request, limits);
        self.finish_request(result)
    }

    /// Cancel a drag. Already started sends retain their owned payload and may
    /// finish; an unknown drag leaves the participant usable.
    pub fn cancel_drag(&mut self, id: DragId) -> Result<()> {
        self.ensure_active()?;
        let result = self.dnd.cancel_drag(id);
        self.finish_request(result)
    }
}
