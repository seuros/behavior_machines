//! Staged circuit execution with non-speculative effects.
//!
//! Unresolved leaves provisionally return `Running`. In the supported node
//! set this stops ordered traversal, even through a status decorator. The
//! circuit therefore exposes the next reachable leaf without executing later
//! leaves. Its result unlocks the next evaluation, always from the state at
//! the beginning of this tick.
//!
//! Tick and halt effects are consumed in post-order, not in separate batches:
//! a reactive subtree must halt its abandoned actions before a later sibling
//! starts. Only the final evaluation commits circuit state. Each reached leaf
//! runs once, so a tick takes at most `leaf_count + 1` circuit evaluations.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use behavior_machines::{Halt, Status, Step};

use crate::{Lowered, Refusal, Schedule, State};

/// A pure check over host state.
///
/// Evaluated only when reached, so it sees changes made by earlier actions
/// within the same tick. It must have no observable effects, including through
/// interior mutability. `Running` is permitted, for example while an externally
/// acquired observation is unavailable.
pub trait Condition<C: ?Sized> {
    /// Returns this check's current status at host-supplied time `now`.
    fn evaluate(&self, ctx: &C, now: Duration) -> Status;
}

/// An effectful operation with an explicit activation lifecycle.
///
/// Definitions are immutable; keep task handles and other per-activation state
/// in `C`. Methods must return promptly: start asynchronous work in `start`,
/// record completion externally, and read it in `poll` on a later host tick.
/// The executor creates no tasks, timers, or threads.
pub trait Action<C: ?Sized> {
    /// Starts an inactive operation. A terminal result finishes it immediately.
    fn start(&self, ctx: &mut C, now: Duration) -> Status;

    /// Advances an operation whose last result was `Running`.
    fn poll(&self, ctx: &mut C, now: Duration) -> Status;

    /// Cancels an abandoned running activation, exactly once.
    ///
    /// Cancellation is a host operation. Hosts should invalidate operation
    /// tickets here as well as request cancellation, so late completions cannot
    /// update a subsequent activation.
    fn halt(&self, ctx: &mut C);
}

/// Explicitly classifies each leaf before the tree is compiled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionLeaf<C, A> {
    /// A read-only condition with no activation effects.
    Condition(C),
    /// An operation that may start, run across ticks, and be halted.
    Action(A),
}

/// A circuit and its staged, effectful execution state.
///
/// Accepts the same node set as [`Lowered`]; unsupported nodes fail before
/// any leaf executes. Ordered children advance within the same tick, running
/// actions resume on a later tick, and a completed root starts fresh on the
/// next tick, just like [`behavior_machines::Tree`].
///
/// Call [`Self::halt`] before discarding a running executor. Dropping it cannot
/// cancel host-owned work because the context is not owned by this type.
#[derive(Clone, Debug, PartialEq)]
pub struct Executor<C, A> {
    lowered: Lowered,
    leaves: Vec<ExecutionLeaf<C, A>>,
    state: State,
    /// Kept separately from provisional circuit state so effects are tracked
    /// immediately, including if evaluation subsequently fails.
    active: Vec<bool>,
    /// The latest honoured tick's instant.
    last: Option<Duration>,
}

impl<C, A> Executor<C, A> {
    /// Compiles a classified definition, given as steps in pre-order (a
    /// `Node` is such a sequence), without evaluating any leaves.
    ///
    /// # Errors
    ///
    /// As [`Lowered::new`].
    pub fn new(
        steps: impl IntoIterator<Item = Step<ExecutionLeaf<C, A>>>,
    ) -> Result<Self, Refusal> {
        let mut leaves = Vec::new();
        // Leaves come off in step order, which is the lowering's leaf order.
        let shape: Vec<Step<()>> = steps
            .into_iter()
            .map(|step| step.map(|leaf| leaves.push(leaf)))
            .collect();
        let lowered = Lowered::new(shape)?;
        let state = State::initial(&lowered);
        let active = vec![false; leaves.len()];
        Ok(Self {
            lowered,
            leaves,
            state,
            active,
            last: None,
        })
    }

    /// The compiled circuit and its immutable plan, for inspecting schedules.
    #[must_use]
    pub const fn lowered(&self) -> &Lowered {
        &self.lowered
    }

    /// Whether the last tick returned `Running` and has not been halted.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.is_tree_running()
    }

    /// Advances one tick, executing only reached leaves in engine order.
    ///
    /// The returned schedule describes the complete tick, not its provisional
    /// evaluations. Leaf methods and halts have already run; do not replay the
    /// schedule as effects.
    ///
    /// # Errors
    ///
    /// [`Refusal::TimeRegression`] when `now` is earlier than the latest
    /// honoured tick: nothing runs and nothing changes. Any other refusal
    /// means internal circuit evaluation failed, a bug in this crate; every
    /// active action is then halted and execution state is reset before
    /// returning it. Like the regular engine, this does not catch panics from
    /// host code.
    pub fn tick<H: ?Sized>(&mut self, ctx: &mut H, now: Duration) -> Result<Schedule, Refusal>
    where
        C: Condition<H>,
        A: Action<H>,
    {
        if self.last.is_some_and(|last| now < last) {
            return Err(Refusal::TimeRegression);
        }
        self.last = Some(now);
        let result = self.advance(ctx, now);
        if result.is_err() {
            self.reset(ctx);
        }
        result
    }

    /// Halts every active action in engine order and resets the circuit.
    ///
    /// An executor with nothing running and no active action answers
    /// [`Halt::AlreadyIdle`] and nothing changes, so repeated calls do
    /// nothing until another action starts. Conditions have no halt effects.
    /// No clock or task cancellation API is assumed.
    pub fn halt<H: ?Sized>(&mut self, ctx: &mut H) -> Halt
    where
        A: Action<H>,
    {
        if !self.is_running() && !self.active.contains(&true) {
            return Halt::AlreadyIdle;
        }
        self.reset(ctx);
        Halt::Halted
    }

    /// Halts every active action in engine order and resets the circuit.
    fn reset<H: ?Sized>(&mut self, ctx: &mut H)
    where
        A: Action<H>,
    {
        let plan = self.lowered.plan();
        for node in plan.report_order() {
            if let Some(slot) = plan.leaf_slot(*node) {
                halt_leaf(&mut self.active, &self.leaves, slot, ctx);
            }
        }
        self.state = State::initial(&self.lowered);
    }

    fn advance<H: ?Sized>(&mut self, ctx: &mut H, now: Duration) -> Result<Schedule, Refusal>
    where
        C: Condition<H>,
        A: Action<H>,
    {
        let Self {
            lowered,
            leaves,
            state,
            active,
            ..
        } = self;
        let plan = lowered.plan();
        let mut statuses = vec![Status::Running; leaves.len()];
        let mut resolved = vec![false; leaves.len()];
        let mut position = 0;
        loop {
            // Never advance memory cursors from a speculative schedule.
            let mut next = state.clone();
            let schedule = lowered.evaluate(&mut next, &statuses)?;
            let mut paused = false;
            while let Some(node) = plan.report_order().get(position).copied() {
                if let Some(slot) = plan.leaf_slot(node) {
                    if schedule.was_halted(node) {
                        halt_leaf(active, leaves, slot, ctx);
                    }
                    if schedule.reached(node) && !resolved.get(slot).copied().unwrap_or(true) {
                        // `evaluate` just accepted `statuses` as one per leaf,
                        // and `leaves`, `resolved` and `active` share that
                        // length, so a leaf slot is in all four.
                        let (Some(status), Some(done), Some(leaf), Some(is_active)) = (
                            statuses.get_mut(slot),
                            resolved.get_mut(slot),
                            leaves.get(slot),
                            active.get_mut(slot),
                        ) else {
                            debug_assert!(false, "every leaf slot has a status, flag and leaf");
                            return Err(Refusal::LeafCount {
                                expected: plan.leaf_count(),
                                given: leaves.len(),
                            });
                        };
                        *status = match leaf {
                            ExecutionLeaf::Condition(condition) => condition.evaluate(ctx, now),
                            ExecutionLeaf::Action(action) if *is_active => action.poll(ctx, now),
                            ExecutionLeaf::Action(action) => action.start(ctx, now),
                        };
                        *is_active = *status == Status::Running;
                        *done = true;
                        // Reevaluate before passing this leaf's position:
                        // its actual status can unlock later children/halts.
                        paused = true;
                        break;
                    }
                }
                // `position` indexes `report_order`, so it stays below its
                // length and the increment cannot overflow.
                position += 1;
            }
            if !paused {
                *state = next;
                return Ok(schedule);
            }
        }
    }
}

/// Halts the action in leaf `slot` if it is active, and marks it inactive.
/// A slot outside the executor names no action, so there is nothing to halt.
fn halt_leaf<C, A, H: ?Sized>(
    active: &mut [bool],
    leaves: &[ExecutionLeaf<C, A>],
    slot: usize,
    ctx: &mut H,
) where
    A: Action<H>,
{
    if active
        .get_mut(slot)
        .is_some_and(|flag| core::mem::replace(flag, false))
        && let Some(ExecutionLeaf::Action(action)) = leaves.get(slot)
    {
        action.halt(ctx);
    }
}

#[cfg(test)]
mod tests;
