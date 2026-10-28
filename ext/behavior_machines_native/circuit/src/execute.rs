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

use behavior_machines::{Node, Status, Time};

use crate::{LowerError, Lowered, Schedule, State, TickError};

/// A pure check over host state.
///
/// Evaluated only when reached, so it sees changes made by earlier actions
/// within the same tick. It must have no observable effects, including through
/// interior mutability. `Running` is permitted, for example while an externally
/// acquired observation is unavailable.
pub trait Condition<C: ?Sized> {
    /// Returns this check's current status at host-supplied time `now`.
    fn evaluate(&self, ctx: &C, now: Time) -> Status;
}

/// An effectful operation with an explicit activation lifecycle.
///
/// Definitions are immutable; keep task handles and other per-activation state
/// in `C`. Methods must return promptly: start asynchronous work in `start`,
/// record completion externally, and read it in `poll` on a later host tick.
/// The executor creates no tasks, timers, or threads.
pub trait Action<C: ?Sized> {
    /// Starts an inactive operation. A terminal result finishes it immediately.
    fn start(&self, ctx: &mut C, now: Time) -> Status;

    /// Advances an operation whose last result was `Running`.
    fn poll(&self, ctx: &mut C, now: Time) -> Status;

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
#[derive(Debug)]
pub struct Executor<C, A> {
    lowered: Lowered,
    leaves: Vec<ExecutionLeaf<C, A>>,
    state: State,
    /// Kept separately from provisional circuit state so effects are tracked
    /// immediately, including if evaluation subsequently fails.
    active: Vec<bool>,
}

impl<C, A> Executor<C, A> {
    /// Compiles a classified definition without evaluating any leaves.
    ///
    /// # Errors
    ///
    /// Returns [`LowerError`] for an unsupported node or empty composite.
    pub fn new(root: Node<ExecutionLeaf<C, A>>) -> Result<Self, LowerError> {
        let lowered = Lowered::new(&root)?;
        let mut leaves = Vec::with_capacity(lowered.plan().leaf_count());
        // Node::map uses the same leaf order as the lowering, without making
        // this executor duplicate the node vocabulary or tree walk.
        root.map(|leaf| leaves.push(leaf));
        let state = State::initial(&lowered);
        let active = vec![false; leaves.len()];
        Ok(Self {
            lowered,
            leaves,
            state,
            active,
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
    /// Returns [`TickError`] if internal circuit evaluation fails. Every active
    /// action is halted and execution state is reset before returning an error.
    /// Like the regular engine, this does not catch panics from host code.
    pub fn tick<H: ?Sized>(&mut self, ctx: &mut H, now: Time) -> Result<Schedule, TickError>
    where
        C: Condition<H>,
        A: Action<H>,
    {
        let result = self.advance(ctx, now);
        if result.is_err() {
            self.halt(ctx);
        }
        result
    }

    /// Halts every active action in engine order and resets the circuit.
    ///
    /// Repeated calls do nothing until another action starts. Conditions have
    /// no halt effects. No clock or task cancellation API is assumed.
    pub fn halt<H: ?Sized>(&mut self, ctx: &mut H)
    where
        A: Action<H>,
    {
        for position in 0..self.lowered.plan().report_order().len() {
            let node = self.lowered.plan().report_order()[position];
            if let Some(slot) = self.lowered.plan().leaf_slot(node) {
                self.halt_leaf(slot, ctx);
            }
        }
        self.state = State::initial(&self.lowered);
    }

    fn advance<H: ?Sized>(&mut self, ctx: &mut H, now: Time) -> Result<Schedule, TickError>
    where
        C: Condition<H>,
        A: Action<H>,
    {
        let mut statuses = vec![Status::Running; self.leaves.len()];
        let mut resolved = vec![false; self.leaves.len()];
        let mut position = 0;
        loop {
            // Never advance memory cursors from a speculative schedule.
            let mut next = self.state.clone();
            let schedule = self.lowered.evaluate(&mut next, &statuses)?;
            while position < self.lowered.plan().report_order().len() {
                let node = self.lowered.plan().report_order()[position];
                if let Some(slot) = self.lowered.plan().leaf_slot(node) {
                    if schedule.halted[node] {
                        self.halt_leaf(slot, ctx);
                    }
                    if schedule.reached(node) && !resolved[slot] {
                        statuses[slot] = match &self.leaves[slot] {
                            ExecutionLeaf::Condition(condition) => condition.evaluate(ctx, now),
                            ExecutionLeaf::Action(action) if self.active[slot] => {
                                action.poll(ctx, now)
                            }
                            ExecutionLeaf::Action(action) => action.start(ctx, now),
                        };
                        self.active[slot] = statuses[slot] == Status::Running;
                        resolved[slot] = true;
                        // Reevaluate before passing this leaf's position:
                        // its actual status can unlock later children/halts.
                        break;
                    }
                }
                position += 1;
            }
            if position == self.lowered.plan().report_order().len() {
                self.state = next;
                return Ok(schedule);
            }
        }
    }

    fn halt_leaf<H: ?Sized>(&mut self, slot: usize, ctx: &mut H)
    where
        A: Action<H>,
    {
        if core::mem::replace(&mut self.active[slot], false)
            && let ExecutionLeaf::Action(action) = &self.leaves[slot]
        {
            action.halt(ctx);
        }
    }
}

#[cfg(test)]
mod tests;
