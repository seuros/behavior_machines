//! Driving a lowered tree: state in, schedule out.
//!
//! # Single-pass evaluation requires pure leaves
//!
//! The circuit computes a whole tick in one pass, which means it needs every
//! leaf's status **before** it can say which leaves the tick reaches. So the
//! host evaluates all of them up front and the circuit reports which ones
//! counted.
//!
//! That is sound only when ticking a leaf has no observable effect beyond its
//! returned status. A condition — "is the battery low", "is the machine in
//! this state" — qualifies. An action that starts a motor, sends a request, or
//! acquires a resource does not: the circuit would have it run on every tick,
//! reached or not.
//!
//! This is not a gap in the lowering, it is forced. An ordered composite ticks
//! its next child only if the previous one succeeded, in the same tick, so a
//! single circuit evaluation cannot both order the effects and compute the
//! arbitration. [`crate::Executor`] provides a separate staged interface that
//! classifies leaves as conditions/actions and evaluates again as reached
//! leaves produce their results. This single-pass interface remains pure.
//!
//! [`Schedule::reached`] is still reported per node, so a host that wants the
//! engine's observer events can emit exactly the ones a tree walk would.

use alloc::vec;
use alloc::vec::Vec;

use behavior_machines::Status;

use crate::lower::Lowered;
use crate::plan::Plan;
use crate::refusal::Refusal;

/// The tree state the circuit carries between ticks.
///
/// This mirrors what the engine keeps in its slots: a `running` flag per node
/// and a cursor per memory composite. Build one with [`State::initial`] and
/// hand it to every [`Lowered::evaluate`] call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    running: Vec<bool>,
    cursor: Vec<u32>,
}

impl State {
    /// The state of a tree that has never ticked: nothing running, every
    /// cursor at its first child.
    #[must_use]
    pub fn initial(lowered: &Lowered) -> Self {
        let plan = lowered.plan();
        Self {
            running: vec![false; plan.node_count()],
            cursor: vec![0; plan.cursor_count()],
        }
    }

    /// Whether node `index` was running after the last tick.
    #[must_use]
    pub fn is_running(&self, index: usize) -> Option<bool> {
        self.running.get(index).copied()
    }

    /// Whether the root is running, which is the tree's own running flag.
    #[must_use]
    pub fn is_tree_running(&self) -> bool {
        self.running.first().copied().unwrap_or(false)
    }

    /// The resume cursor of memory composite `cell`.
    #[must_use]
    pub fn cursor(&self, cell: usize) -> Option<u32> {
        self.cursor.get(cell).copied()
    }
}

/// What one tick decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    status: Vec<Option<Status>>,
    halted: Vec<bool>,
}

impl Schedule {
    /// Per node, in pre-order: the status it returned, or `None` if the tick
    /// did not reach it.
    #[must_use]
    pub fn status(&self) -> &[Option<Status>] {
        &self.status
    }

    /// Per node: it was running and this tick halted it, so an effectful leaf
    /// here needs its `halt` called.
    #[must_use]
    pub fn halted(&self) -> &[bool] {
        &self.halted
    }

    /// Whether the tick halted node `index`.
    #[must_use]
    pub fn was_halted(&self, index: usize) -> bool {
        self.halted.get(index).copied().unwrap_or(false)
    }

    /// The root's status, which is the tree's result for the tick.
    ///
    /// The root is always reached, so this is never `None` for a schedule the
    /// circuit produced.
    #[must_use]
    pub fn root(&self) -> Option<Status> {
        self.status.first().copied().flatten()
    }

    /// Whether the tick reached node `index`.
    #[must_use]
    pub fn reached(&self, index: usize) -> bool {
        self.status.get(index).is_some_and(Option::is_some)
    }

    /// The nodes the tick ticked, in the order the engine's
    /// [`Observer`](behavior_machines::Observer) reports them.
    ///
    /// Takes the plan because the order is post-order, which the index order
    /// alone does not give: pre-order is `[parent, subtree(c0), subtree(c1)]`,
    /// so reversing it puts `c1`'s subtree first while the engine ticks `c0`
    /// first.
    pub fn ticked<'a>(&'a self, plan: &'a Plan) -> impl Iterator<Item = (usize, Status)> + 'a {
        plan.report_order().iter().filter_map(|index| {
            self.status
                .get(*index)
                .copied()
                .flatten()
                .map(|status| (*index, status))
        })
    }

    /// The nodes the tick halted, in the engine's report order.
    pub fn halts<'a>(&'a self, plan: &'a Plan) -> impl Iterator<Item = usize> + 'a {
        plan.report_order()
            .iter()
            .copied()
            .filter(|index| self.was_halted(*index))
    }
}

impl Lowered {
    /// Evaluates one tick, advancing `state`.
    ///
    /// `leaf_status` holds one status per leaf, in leaf order, which the host
    /// produced by ticking every leaf. This requires pure leaves; use
    /// [`crate::Executor`] for staged, effectful execution.
    ///
    /// # Errors
    ///
    /// [`Refusal::LeafCount`], [`Refusal::StateShape`], or
    /// [`Refusal::CursorShape`] if the arguments do not match this tree;
    /// `state` is left as it was.
    pub fn evaluate(&self, state: &mut State, leaf_status: &[Status]) -> Result<Schedule, Refusal> {
        let plan = self.plan();
        if leaf_status.len() != plan.leaf_count() {
            return Err(Refusal::LeafCount {
                expected: plan.leaf_count(),
                given: leaf_status.len(),
            });
        }
        if state.running.len() != plan.node_count() {
            return Err(Refusal::StateShape {
                expected: plan.node_count(),
                given: state.running.len(),
            });
        }
        if state.cursor.len() != plan.cursor_count() {
            return Err(Refusal::CursorShape {
                expected: plan.cursor_count(),
                given: state.cursor.len(),
            });
        }

        let cells = plan.cells(&state.running, &state.cursor, leaf_status);
        let values = self.circuit().evaluate(&cells)?;
        debug_assert_eq!(self.validate(&values), Ok(()), "lowering invariant broken");
        let outputs = self.outputs();

        let set = |bit: &crate::builder::Bit| bit.form().value(&values).is_some_and(|v| v != 0.0);

        // Every output vector was built from this plan: one entry per node,
        // or per cursor, the same lengths `state` was just checked against.
        let status = outputs
            .reached
            .iter()
            .zip(&outputs.status)
            .map(|(reached, bits)| {
                set(reached).then(|| {
                    if set(&bits.success) {
                        Status::Success
                    } else if set(&bits.failure) {
                        Status::Failure
                    } else {
                        Status::Running
                    }
                })
            })
            .collect();
        let halted = outputs.halted.iter().map(set).collect();

        for (running, next) in state.running.iter_mut().zip(&outputs.next_running) {
            *running = set(next);
        }
        for (cursor, next) in state.cursor.iter_mut().zip(&outputs.next_cursor) {
            let next = next.value(&values).unwrap_or(0.0);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a cursor is a small integer"
            )]
            #[expect(clippy::cast_sign_loss, reason = "a cursor is never negative")]
            let next = next as u32;
            *cursor = next;
        }

        Ok(Schedule { status, halted })
    }
}

#[cfg(test)]
mod tests;
