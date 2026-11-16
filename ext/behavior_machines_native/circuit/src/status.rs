//! The three-valued status, as bits.
//!
//! A node's result is encoded as three bits of which **at most one** is set.
//! All three zero means the node was not ticked this tick, which a tree walk
//! expresses by simply not calling the node and a circuit has to carry
//! explicitly: every node's slot exists in every tick whether it ran or not.
//!
//! Keeping "not ticked" as the all-zero case rather than a fourth bit is what
//! makes [`StatusBits::ticked`] a free sum and lets unticked children fall out
//! of the ordered-composite arithmetic for nothing: a child before the cursor
//! contributes 0 to every count it appears in.

use behavior_machines::Status;

use crate::builder::{Bit, Count};

/// A node's status for one tick: at most one of three bits.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusBits {
    /// The node returned [`Status::Success`].
    pub success: Bit,
    /// The node returned [`Status::Failure`].
    pub failure: Bit,
    /// The node returned [`Status::Running`].
    pub running: Bit,
}

impl StatusBits {
    /// The all-zero encoding: the node was not ticked.
    #[must_use]
    pub const fn unticked() -> Self {
        Self {
            success: Bit::ZERO,
            failure: Bit::ZERO,
            running: Bit::ZERO,
        }
    }

    /// A constant status, as [`Status`] is always one of the three.
    #[must_use]
    pub fn constant(status: Status) -> Self {
        let mut bits = Self::unticked();
        *bits.bit_mut(status) = Bit::ONE;
        bits
    }

    /// The bit for `status`.
    #[must_use]
    pub const fn bit(&self, status: Status) -> &Bit {
        match status {
            Status::Success => &self.success,
            Status::Failure => &self.failure,
            Status::Running => &self.running,
        }
    }

    /// The bit for `status`, mutably.
    pub const fn bit_mut(&mut self, status: Status) -> &mut Bit {
        match status {
            Status::Success => &mut self.success,
            Status::Failure => &mut self.failure,
            Status::Running => &mut self.running,
        }
    }

    /// Whether the node was ticked at all: the sum of the three bits, which is
    /// a bit because at most one is set. Free.
    #[must_use]
    pub fn ticked(&self) -> Bit {
        Bit::or_disjoint([
            self.success.clone(),
            self.failure.clone(),
            self.running.clone(),
        ])
    }

    /// Whether the node finished: success or failure, but not running. Free.
    #[must_use]
    pub fn finished(&self) -> Bit {
        Bit::or_disjoint([self.success.clone(), self.failure.clone()])
    }

    /// The count of set bits, for the zero test.
    #[must_use]
    pub fn count(&self) -> Count {
        Count::of([
            self.success.clone(),
            self.failure.clone(),
            self.running.clone(),
        ])
    }

    /// Swaps success and failure, leaving running alone. Free: the
    /// [`Inverter`](behavior_machines::Step::Inverter) decorator costs no
    /// operations at all, it relabels two wires.
    #[must_use]
    pub fn inverted(self) -> Self {
        Self {
            success: self.failure,
            failure: self.success,
            running: self.running,
        }
    }

    /// Reports any finished child as `forced`, leaving running alone. Free.
    ///
    /// This is [`ForceSuccess`](behavior_machines::Step::ForceSuccess) and
    /// [`ForceFailure`](behavior_machines::Step::ForceFailure): the two
    /// finished bits merge into one, which is addition of disjoint bits.
    #[must_use]
    pub fn forced(self, forced: Status) -> Self {
        let finished = self.finished();
        let mut bits = Self::unticked();
        bits.running = self.running;
        *bits.bit_mut(forced) = finished;
        bits
    }
}

#[cfg(test)]
mod tests;
