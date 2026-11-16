use core::time::Duration;

use crate::observe::Observer;
use crate::{Halt, Leaf, Refusal, Status, Step};

mod composite;
mod control;
mod decorator;
mod slot;

use slot::{Kind, Run, Slot};

/// A compiled behavior tree of at most `N` nodes, together with its execution
/// state. It holds everything inline: building and ticking allocate nothing.
///
/// Each [`Tree::tick`] advances as far as the leaves allow: a sequence that
/// sees a child succeed starts the next child in the same tick. The one
/// exception is a looping decorator, which finishes at most one run of its
/// child per tick.
///
/// When the tree stops ticking a running node (a parallel decided, a reactive
/// guard or `WhileDoElse` condition flipped, a timeout expired, or
/// [`Tree::halt`] was called), every active leaf under it is halted exactly
/// once before the call returns. See [`Leaf`] for the full contract.
///
/// Time comes only from the `now` passed to each tick, a monotonic
/// [`Duration`] since an epoch the host picks. A host that sleeps between
/// ticks should wake by [`Tree::next_deadline`] so `Sleep`, `Timeout`, and
/// looping nodes act on time.
///
/// After the root finishes, the next tick starts a fresh run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree<L, const N: usize> {
    /// The nodes in pre-order; only the first `len` are part of the tree.
    slots: [Slot; N],
    /// The leaf of each leaf node, at its node's index.
    leaves: [Option<L>; N],
    len: usize,
    /// The latest honoured tick's instant.
    last: Option<Duration>,
}

impl<L, const N: usize> Tree<L, N> {
    /// Compiles a definition given as [`Step`]s in pre-order (a
    #[cfg_attr(
        feature = "alloc",
        doc = " [`Node`](crate::Node) with feature `alloc` is such a sequence)."
    )]
    #[cfg_attr(
        not(feature = "alloc"),
        doc = " `Node` with feature `alloc` is such a sequence)."
    )]
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] past `N` nodes, [`Refusal::EmptyComposite`] or
    /// [`Refusal::ParallelThreshold`] for a step [`Step::validate`] refuses,
    /// [`Refusal::Incomplete`] if the steps end before every child is given,
    /// and [`Refusal::Trailing`] for a step after the complete root.
    pub fn new(steps: impl IntoIterator<Item = Step<L>>) -> Result<Self, Refusal> {
        const { assert!(N > 0, "a tree needs room for at least its root") };
        let mut slots = [Slot::idle(); N];
        let mut leaves: [Option<L>; N] = core::array::from_fn(|_| None);
        // The indices of nodes still waiting for children; while a node is
        // open its `size` counts the children it still waits for.
        let mut open = [0; N];
        let mut depth = 0;
        let mut len = 0;
        for step in steps {
            if len > 0 && depth == 0 {
                return Err(Refusal::Trailing);
            }
            step.validate()?;
            let (Some(slot), Some(leaf_cell)) = (slots.get_mut(len), leaves.get_mut(len)) else {
                return Err(Refusal::Full);
            };
            let children = step.children();
            let shape = step.map(|leaf| *leaf_cell = Some(leaf));
            *slot = Slot::new(Kind::of(shape), children);
            len += 1;
            if children > 0 {
                if let Some(cell) = open.get_mut(depth) {
                    *cell = len - 1;
                }
                depth += 1;
                continue;
            }
            slot.size = 1;
            // Close every ancestor this step completed.
            while let Some(parent) = depth.checked_sub(1).and_then(|top| open.get(top)) {
                let Some(node) = slots.get_mut(*parent) else {
                    break;
                };
                // An open node's `size` is its count of children still
                // awaited, at least one while it sits on `open`; it is popped
                // the moment it reaches zero, so this never wraps. `len`,
                // `depth` and `len - parent` stay within `N`: every open node
                // and every compiled step owns a distinct slot.
                node.size -= 1;
                if node.size > 0 {
                    break;
                }
                node.size = len - parent;
                depth -= 1;
            }
        }
        if len == 0 || depth > 0 {
            return Err(Refusal::Incomplete);
        }
        Ok(Self {
            slots,
            leaves,
            len,
            last: None,
        })
    }

    /// Ticks the tree once at `now` and returns the root's status.
    ///
    /// # Errors
    ///
    /// [`Refusal::TimeRegression`] when `now` is earlier than the latest
    /// honoured tick; no leaf runs.
    pub fn tick<C: ?Sized>(&mut self, ctx: &mut C, now: Duration) -> Result<Status, Refusal>
    where
        L: Leaf<C>,
    {
        self.tick_observed(ctx, now, &mut ())
    }

    /// [`Tree::tick`], reporting every node ticked or halted to `observer`.
    ///
    /// # Errors
    ///
    /// As [`Tree::tick`].
    pub fn tick_observed<C, O>(
        &mut self,
        ctx: &mut C,
        now: Duration,
        observer: &mut O,
    ) -> Result<Status, Refusal>
    where
        L: Leaf<C>,
        C: ?Sized,
        O: Observer + ?Sized,
    {
        if self.last.is_some_and(|last| now < last) {
            return Err(Refusal::TimeRegression);
        }
        self.last = Some(now);
        let Self {
            slots, leaves, len, ..
        } = self;
        let run = &mut Run {
            ctx,
            observer,
            leaves,
        };
        Ok(slot::tick(
            slots.get_mut(..*len).unwrap_or_default(),
            0,
            run,
            now,
        ))
    }

    /// Halts every active leaf and resets the tree. An idle tree answers
    /// [`Halt::AlreadyIdle`] and nothing changes.
    pub fn halt<C: ?Sized>(&mut self, ctx: &mut C) -> Halt
    where
        L: Leaf<C>,
    {
        self.halt_observed(ctx, &mut ())
    }

    /// [`Tree::halt`], reporting every node halted to `observer`.
    pub fn halt_observed<C, O>(&mut self, ctx: &mut C, observer: &mut O) -> Halt
    where
        L: Leaf<C>,
        C: ?Sized,
        O: Observer + ?Sized,
    {
        if !self.is_running() {
            return Halt::AlreadyIdle;
        }
        let Self {
            slots, leaves, len, ..
        } = self;
        let run = &mut Run {
            ctx,
            observer,
            leaves,
        };
        slot::halt(slots.get_mut(..*len).unwrap_or_default(), 0, run);
        Halt::Halted
    }

    /// Whether the last tick returned [`Status::Running`] and the tree has not
    /// been halted since.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.slots.first().is_some_and(|root| root.running)
    }

    /// How many nodes the tree has.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Always `false`: a tree has at least its root.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The earliest instant a running `Sleep` or `Timeout` node will fire, or
    /// a looping decorator wants its next run (the instant of the tick that
    /// finished the last one, so already due), if any. Ticking at or after it
    /// lets that node act; leaves wake the host through their own means.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Duration> {
        slot::next_deadline(self.slots.get(..self.len).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;
