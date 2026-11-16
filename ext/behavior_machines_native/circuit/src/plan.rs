//! Where each input cell lives and what it means.
//!
//! The circuit reads one flat `[f32]`. The plan is the only thing that knows
//! how to address it, so the layout can change without touching the lowering
//! or the host.
//!
//! ```text
//! [ running: one per node ][ cursor: one per memory composite ][ leaf status: three per leaf ]
//! ```
//!
//! `running` and `cursor` are the tree's state carried between ticks, the same
//! fields the engine keeps in each slot. The leaf statuses are this tick's
//! input: the host evaluates its leaves and writes them here.

use alloc::vec::Vec;

use behavior_machines::Status;

use crate::builder::Bit;
use crate::status::StatusBits;
use crate::wire::Wire;

/// The input-cell layout of a lowered tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    nodes: usize,
    /// Node index of each leaf slot, in leaf order.
    leaves: Vec<usize>,
    /// Node index of each cursor cell, in cell order.
    cursors: Vec<usize>,
    /// Every node index in post-order.
    report_order: Vec<usize>,
}

impl Plan {
    pub(crate) const fn new(
        nodes: usize,
        leaves: Vec<usize>,
        cursors: Vec<usize>,
        report_order: Vec<usize>,
    ) -> Self {
        Self {
            nodes,
            leaves,
            cursors,
            report_order,
        }
    }

    /// Every node index in the order the engine's
    /// [`Observer`](behavior_machines::Observer) reports them: depth-first
    /// post-order, children before parents.
    ///
    /// Reversing the pre-order indices is *not* this order. Pre-order is
    /// `[parent, subtree(c0), subtree(c1)]`, so reversing it yields `c1`'s
    /// subtree before `c0`'s, while the engine ticks `c0` first.
    #[must_use]
    pub fn report_order(&self) -> &[usize] {
        &self.report_order
    }

    /// How many nodes the tree has. Node indices are the engine's
    /// [`NodeId`](behavior_machines::NodeId) values: both number nodes by a
    /// depth-first pre-order walk of the definition.
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.nodes
    }

    /// How many leaves the tree has.
    #[must_use]
    pub const fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// How many memory composites the tree has, each owning one cursor cell.
    #[must_use]
    pub const fn cursor_count(&self) -> usize {
        self.cursors.len()
    }

    /// The node index of leaf slot `slot`.
    #[must_use]
    pub fn leaf_node(&self, slot: usize) -> Option<usize> {
        self.leaves.get(slot).copied()
    }

    /// The leaf slot of node `index`, if it is a leaf.
    #[must_use]
    pub fn leaf_slot(&self, index: usize) -> Option<usize> {
        self.leaves.iter().position(|node| *node == index)
    }

    /// The node index owning cursor cell `cell`.
    #[must_use]
    pub fn cursor_node(&self, cell: usize) -> Option<usize> {
        self.cursors.get(cell).copied()
    }

    /// How many input cells a tick needs.
    #[must_use]
    pub const fn cell_count(&self) -> u32 {
        let cells = self.nodes + self.cursors.len() + self.leaves.len() * 3;
        // A tree large enough to overflow this would not fit in memory.
        #[expect(clippy::cast_possible_truncation, reason = "bounded by tree size")]
        let cells = cells as u32;
        cells
    }

    /// The cell holding node `index`'s `running` flag, or `None` if this
    /// plan has no such node.
    #[must_use]
    pub fn running_cell(&self, index: usize) -> Option<u32> {
        // The running flags open the layout, so a node's cell is its index.
        (index < self.nodes)
            .then_some(index)
            .and_then(|offset| u32::try_from(offset).ok())
    }

    /// The cell holding cursor `cell`, or `None` if this plan has no such
    /// cursor.
    #[must_use]
    pub fn cursor_cell(&self, cell: usize) -> Option<u32> {
        (cell < self.cursors.len())
            .then(|| self.cursor_offset(cell))
            .and_then(|offset| u32::try_from(offset).ok())
    }

    /// The first of the three cells holding leaf `slot`'s status, in
    /// success, failure, running order, or `None` if this plan has no such
    /// leaf.
    #[must_use]
    pub fn leaf_cell(&self, slot: usize) -> Option<u32> {
        self.leaf_status_cell(slot, Status::Success)
    }

    /// The cell holding leaf `slot`'s `status` bit, or `None` if this plan
    /// has no such leaf.
    #[must_use]
    pub fn leaf_status_cell(&self, slot: usize, status: Status) -> Option<u32> {
        (slot < self.leaves.len())
            .then(|| self.leaf_offset(slot, status))
            .and_then(|offset| u32::try_from(offset).ok())
    }

    // The offsets below take an index already known to belong to this plan
    // (checked above, or produced by the lowering that built it). The result
    // is then below the cell count, a sum of `Vec` lengths that fits in
    // memory, so the arithmetic cannot overflow.

    const fn cursor_offset(&self, cell: usize) -> usize {
        self.nodes + cell
    }

    const fn leaf_offset(&self, slot: usize, status: Status) -> usize {
        self.nodes + self.cursors.len() + slot * 3 + status_offset(status)
    }

    /// The input cells for one tick, in layout order: the state's `running`
    /// flags and cursors, then a one-hot triple per leaf status. The caller
    /// has checked every length against this plan.
    pub(crate) fn cells(
        &self,
        running: &[bool],
        cursor: &[u32],
        leaf_status: &[Status],
    ) -> Vec<f32> {
        let mut cells = Vec::with_capacity(self.cell_count() as usize);
        cells.extend(running.iter().map(|running| f32::from(u8::from(*running))));
        cells.extend(
            cursor
                .iter()
                .map(|cursor| f32::from(u16::try_from(*cursor).unwrap_or(u16::MAX))),
        );
        for status in leaf_status {
            cells.extend(
                [Status::Success, Status::Failure, Status::Running]
                    .map(|bit| f32::from(u8::from(bit == *status))),
            );
        }
        cells
    }

    /// The running flags open the layout, so a node's cell is its index.
    pub(crate) const fn running_wire(index: usize) -> Wire {
        Wire::cell(wire_cell(index))
    }

    pub(crate) const fn cursor_wire(&self, cell: usize) -> Wire {
        Wire::cell(wire_cell(self.cursor_offset(cell)))
    }

    /// Leaf `slot`'s status, read from its three cells.
    ///
    /// The host promises the three hold a one-hot triple; a leaf always
    /// returns exactly one of the three statuses, so that is what writing the
    /// result down produces.
    pub(crate) fn leaf_status(&self, slot: usize) -> StatusBits {
        let bit = |status| Bit::assume(Wire::cell(wire_cell(self.leaf_offset(slot, status))));
        StatusBits {
            success: bit(Status::Success),
            failure: bit(Status::Failure),
            running: bit(Status::Running),
        }
    }
}

/// A cell offset of a lowered tree as a wire cell. The lowering only builds
/// trees whose cell count fits a `u32` (see [`Plan::cell_count`]).
const fn wire_cell(offset: usize) -> u32 {
    #[expect(clippy::cast_possible_truncation, reason = "bounded by tree size")]
    let cell = offset as u32;
    cell
}

/// The offset of a status within a leaf's three cells.
const fn status_offset(status: Status) -> usize {
    match status {
        Status::Success => 0,
        Status::Failure => 1,
        Status::Running => 2,
    }
}

#[cfg(test)]
mod tests;
