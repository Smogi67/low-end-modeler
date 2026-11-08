//! Undo and redo. See the [crate documentation](crate) for the design.

use std::collections::VecDeque;
use std::mem::size_of;

use crate::error::MeshError;
use crate::id::{FaceId, VertexId};
use crate::mesh::{Mesh, MeshDelta};

/// Default history memory cap: 64 MiB. The target laptop has 4 GB of RAM shared
/// with Windows, and a low-poly edit normally costs far less than this.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Default history step cap.
pub const DEFAULT_MAX_STEPS: usize = 256;

/// Limits on undo history. The oldest steps are dropped first when either is exceeded.
///
/// The most recent step is always kept, even if it alone exceeds `max_bytes` or if
/// `max_steps` is 0, so the last action can always be undone. Only in that case can
/// [`Document::history_bytes`] exceed `max_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    /// Most memory that undo and redo steps may use together.
    pub max_bytes: usize,
    /// Most steps that may be kept for undo.
    pub max_steps: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_steps: DEFAULT_MAX_STEPS,
        }
    }
}

#[derive(Debug)]
struct Step {
    /// Unique among this document's steps, so a saved state can be recognized.
    id: u64,
    label: &'static str,
    delta: MeshDelta,
    bytes: usize,
}

/// A mesh with its undo history.
///
/// The mesh can only be changed through [`edit`](Self::edit) or between
/// [`begin_edit`](Self::begin_edit) and [`commit_edit`](Self::commit_edit), so no
/// change can slip past the history.
#[derive(Debug)]
pub struct Document {
    mesh: Mesh,
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    limits: HistoryLimits,
    history_bytes: usize,
    active_edit: Option<&'static str>,
    /// The ID the next step gets.
    next_id: u64,
    /// The ID of the newest step dropped from history, which the mesh still includes
    /// once everything kept has been undone.
    dropped_id: u64,
    /// Where joined groups were when the open edit began. See
    /// [`Mesh::restore_groups`].
    group_marks: Vec<(FaceId, VertexId, u16)>,
}

impl Document {
    /// Wraps a mesh, with the default history limits.
    pub fn new(mesh: Mesh) -> Self {
        Self::with_limits(mesh, HistoryLimits::default())
    }

    /// Wraps a mesh, with the given history limits.
    pub fn with_limits(mesh: Mesh, limits: HistoryLimits) -> Self {
        Self {
            mesh,
            undo: VecDeque::new(),
            redo: Vec::new(),
            limits,
            history_bytes: 0,
            active_edit: None,
            next_id: 1,
            dropped_id: 0,
            group_marks: Vec::new(),
        }
    }

    /// The mesh, for reading.
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Runs one edit as a single undo step. If `edit` returns an error, every
    /// change it made is reverted and no step is recorded.
    ///
    /// If the edit changes nothing, no step is recorded and redo stays available.
    pub fn edit<T>(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut Mesh) -> Result<T, MeshError>,
    ) -> Result<T, MeshError> {
        self.begin_edit(label)?;
        match edit(&mut self.mesh) {
            Ok(value) => {
                self.commit_edit()?;
                Ok(value)
            }
            Err(error) => {
                self.cancel_edit()?;
                Err(error)
            }
        }
    }

    /// Starts an edit that stays open across frames, such as dragging a vertex.
    /// Get the mesh with [`active_mesh`](Self::active_mesh), then finish with
    /// [`commit_edit`](Self::commit_edit) or [`cancel_edit`](Self::cancel_edit).
    pub fn begin_edit(&mut self, label: &'static str) -> Result<(), MeshError> {
        if self.active_edit.is_some() {
            return Err(MeshError::EditInProgress);
        }
        self.mesh.begin_journal();
        self.active_edit = Some(label);
        self.group_marks = self.mesh.group_marks();
        Ok(())
    }

    /// The mesh for changing, while an edit is open.
    pub fn active_mesh(&mut self) -> Option<&mut Mesh> {
        self.active_edit.map(|_| &mut self.mesh)
    }

    /// True while an edit is open.
    pub fn edit_in_progress(&self) -> bool {
        self.active_edit.is_some()
    }

    /// Finishes the open edit. Returns `true` if it recorded a step, or `false` if
    /// it changed nothing.
    ///
    /// In debug builds this also validates the mesh, so a tool that damages the
    /// mesh fails straight away in tests.
    pub fn commit_edit(&mut self) -> Result<bool, MeshError> {
        let label = self.active_edit.take().ok_or(MeshError::NoEditInProgress)?;
        // A tool that rebuilt every face of a joined part gave the new faces no group.
        let marks = std::mem::take(&mut self.group_marks);
        self.mesh.restore_groups(&marks);
        let delta = self.mesh.finish_journal();
        debug_assert_eq!(
            self.mesh.validate(),
            Ok(()),
            "edit {label:?} damaged the mesh"
        );
        if delta.is_empty() {
            return Ok(false);
        }

        let bytes = size_of::<Step>() + delta.memory_bytes();
        self.clear_redo();
        let id = self.next_id;
        self.next_id += 1;
        self.undo.push_back(Step {
            id,
            label,
            delta,
            bytes,
        });
        self.history_bytes += bytes;
        self.enforce_limits();
        Ok(true)
    }

    /// Abandons the open edit and restores the mesh to how it was when the edit
    /// began. History is untouched, so redo stays available. IDs created during
    /// the edit stay stale permanently.
    pub fn cancel_edit(&mut self) -> Result<(), MeshError> {
        self.active_edit.take().ok_or(MeshError::NoEditInProgress)?;
        self.mesh.rollback_journal();
        Ok(())
    }

    /// Undoes the latest step and returns its label. Returns `None` if there is
    /// nothing to undo or an edit is open.
    pub fn undo(&mut self) -> Option<&'static str> {
        if self.active_edit.is_some() {
            return None;
        }
        let mut step = self.undo.pop_back()?;
        self.mesh.swap_delta(&mut step.delta);
        let label = step.label;
        self.redo.push(step);
        Some(label)
    }

    /// Redoes the latest undone step and returns its label. Returns `None` if there
    /// is nothing to redo or an edit is open.
    pub fn redo(&mut self) -> Option<&'static str> {
        if self.active_edit.is_some() {
            return None;
        }
        let mut step = self.redo.pop()?;
        self.mesh.swap_delta(&mut step.delta);
        let label = step.label;
        self.undo.push_back(step);
        Some(label)
    }

    /// A number for the mesh's current state in history. It changes with every edit,
    /// undo and redo, and comes back when undo or redo returns to the same state, so
    /// comparing it with the number at the last save tells whether there are unsaved
    /// changes.
    pub fn version(&self) -> u64 {
        self.undo.back().map_or(self.dropped_id, |step| step.id)
    }

    /// Label of the step [`undo`](Self::undo) would undo, for the UI.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.back().map(|step| step.label)
    }

    /// Label of the step [`redo`](Self::redo) would redo, for the UI.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|step| step.label)
    }

    /// How many steps can be undone.
    pub fn undo_count(&self) -> usize {
        self.undo.len()
    }

    /// How many steps can be redone.
    pub fn redo_count(&self) -> usize {
        self.redo.len()
    }

    /// Memory used by undo and redo steps together, in bytes.
    pub fn history_bytes(&self) -> usize {
        self.history_bytes
    }

    /// The current limits.
    pub fn limits(&self) -> HistoryLimits {
        self.limits
    }

    /// Changes the limits, dropping the oldest undo steps if needed.
    pub fn set_limits(&mut self, limits: HistoryLimits) {
        self.limits = limits;
        self.enforce_limits();
    }

    /// Clears all history. Fails if an edit is open.
    pub fn clear_history(&mut self) -> Result<(), MeshError> {
        if self.active_edit.is_some() {
            return Err(MeshError::EditInProgress);
        }
        self.undo.clear();
        self.redo.clear();
        self.history_bytes = 0;
        Ok(())
    }

    /// Ends the document and returns its mesh. An open edit is cancelled first.
    pub fn into_mesh(mut self) -> Mesh {
        let _ = self.cancel_edit();
        self.mesh
    }

    fn clear_redo(&mut self) {
        for step in self.redo.drain(..) {
            self.history_bytes -= step.bytes;
        }
    }

    /// Drops the oldest undo steps, then the farthest redo steps, until within the
    /// limits. The step that the next undo or redo would use is kept for last.
    fn enforce_limits(&mut self) {
        while self.undo.len() > self.limits.max_steps.max(1) {
            self.drop_oldest_undo();
        }
        while self.history_bytes > self.limits.max_bytes && self.undo.len() > 1 {
            self.drop_oldest_undo();
        }
        // Reached only by set_limits, when lowering the limits with redo steps held.
        while self.history_bytes > self.limits.max_bytes
            && self.undo.len() + self.redo.len() > 1
            && !self.redo.is_empty()
        {
            let step = self.redo.remove(0);
            self.history_bytes -= step.bytes;
        }
    }

    fn drop_oldest_undo(&mut self) {
        if let Some(step) = self.undo.pop_front() {
            self.history_bytes -= step.bytes;
            self.dropped_id = step.id;
        }
    }
}
