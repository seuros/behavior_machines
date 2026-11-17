use core::time::Duration;

use crate::observe::Observer;
use crate::{Halt, Leaf, Refusal, Status, Step};

mod composite;
mod control;
mod decorator;
mod slot;

use slot::{Kind, Run, Slot};
#[cfg(any(feature = "alloc", test))]
use storage::Heap;
use storage::Inline;
use storage::Storage;
use storage::sealed::Stack as _;

pub mod storage;

/// A compiled behavior tree of at most `N` nodes, held inline: building and
/// ticking allocate nothing.
pub type Tree<L, const N: usize> = Behavior<Inline<L, N>>;

/// A compiled behavior tree on the heap, sized by its definition (feature
/// `alloc`). It answers exactly as a [`Tree`] with room for the definition;
/// only building allocates.
///
/// ```
/// use behavior_machines::{HeapTree, Node};
///
/// let tree = HeapTree::new(Node::sequence((0..1_000).map(Node::leaf)))?;
/// assert_eq!(tree.len(), 1_001);
/// # Ok::<(), behavior_machines::Refusal>(())
/// ```
#[cfg(any(feature = "alloc", test))]
pub type HeapTree<L> = Behavior<Heap<L>>;

/// A compiled behavior tree together with its execution state, its nodes kept
/// in `S`: [`Tree`] holds them inline, `HeapTree` (feature `alloc`) on the
/// heap. The storage changes nothing below.
///
/// Each [`Behavior::tick`] advances as far as the leaves allow: a sequence
/// that sees a child succeed starts the next child in the same tick. The one
/// exception is a looping decorator, which finishes at most one run of its
/// child per tick.
///
/// When the tree stops ticking a running node (a parallel decided, a reactive
/// guard or `WhileDoElse` condition flipped, a timeout expired, or
/// [`Behavior::halt`] was called), every active leaf under it is halted
/// exactly once before the call returns. See [`Leaf`] for the full contract.
///
/// Time comes only from the `now` passed to each tick, a monotonic
/// [`Duration`] since an epoch the host picks. A host that sleeps between
/// ticks should wake by [`Behavior::next_deadline`] so `Sleep`, `Timeout`,
/// and looping nodes act on time.
///
/// After the root finishes, the next tick starts a fresh run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Behavior<S> {
    /// The nodes in pre-order, and the leaf of each leaf node at its index.
    store: S,
    /// The latest honoured tick's instant.
    last: Option<Duration>,
}

impl<S: Storage> Behavior<S> {
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
    /// [`Refusal::Full`] past an inline tree's `N` nodes, or when a heap tree
    /// cannot grow, [`Refusal::EmptyComposite`] or
    /// [`Refusal::ParallelThreshold`] for a step [`Step::validate`] refuses,
    /// [`Refusal::Incomplete`] if the steps end before every child is given,
    /// and [`Refusal::Trailing`] for a step after the complete root.
    pub fn new(steps: impl IntoIterator<Item = Step<S::Leaf>>) -> Result<Self, Refusal> {
        let mut store = S::empty();
        // The indices of nodes still waiting for children; while a node is
        // open its `size` counts the children it still waits for.
        let mut open = S::Open::empty();
        for step in steps {
            if !store.slots().is_empty() && open.none_open() {
                return Err(Refusal::Trailing);
            }
            step.validate()?;
            let children = step.children();
            let mut leaf = None;
            let shape = step.map(|own| leaf = Some(own));
            let at = store.push(Slot::new(Kind::of(shape), children.max(1)), leaf)?;
            if children > 0 {
                open.enter(at)?;
                continue;
            }
            // Close every ancestor this step completed.
            while let Some(parent) = open.innermost() {
                let Some(node) = store.slot_mut(parent) else {
                    break;
                };
                // An open node's `size` is its count of children still
                // awaited, at least one while it sits on `open`; it is popped
                // the moment it reaches zero, so this never wraps. `parent`
                // precedes `at`, which every compiled step follows.
                node.size -= 1;
                if node.size > 0 {
                    break;
                }
                node.size = at + 1 - parent;
                open.leave();
            }
        }
        if store.slots().is_empty() || !open.none_open() {
            return Err(Refusal::Incomplete);
        }
        Ok(Self { store, last: None })
    }

    /// Ticks the tree once at `now` and returns the root's status.
    ///
    /// # Errors
    ///
    /// [`Refusal::TimeRegression`] when `now` is earlier than the latest
    /// honoured tick; no leaf runs.
    pub fn tick<C: ?Sized>(&mut self, ctx: &mut C, now: Duration) -> Result<Status, Refusal>
    where
        S::Leaf: Leaf<C>,
    {
        self.tick_observed(ctx, now, &mut ())
    }

    /// [`Behavior::tick`], reporting every node ticked or halted to `observer`.
    ///
    /// # Errors
    ///
    /// As [`Behavior::tick`].
    pub fn tick_observed<C, O>(
        &mut self,
        ctx: &mut C,
        now: Duration,
        observer: &mut O,
    ) -> Result<Status, Refusal>
    where
        S::Leaf: Leaf<C>,
        C: ?Sized,
        O: Observer + ?Sized,
    {
        if self.last.is_some_and(|last| now < last) {
            return Err(Refusal::TimeRegression);
        }
        self.last = Some(now);
        let (slots, leaves) = self.store.parts();
        let run = &mut Run {
            ctx,
            observer,
            leaves,
        };
        Ok(slot::tick(slots, 0, run, now))
    }

    /// Halts every active leaf and resets the tree. An idle tree answers
    /// [`Halt::AlreadyIdle`] and nothing changes.
    pub fn halt<C: ?Sized>(&mut self, ctx: &mut C) -> Halt
    where
        S::Leaf: Leaf<C>,
    {
        self.halt_observed(ctx, &mut ())
    }

    /// [`Behavior::halt`], reporting every node halted to `observer`.
    pub fn halt_observed<C, O>(&mut self, ctx: &mut C, observer: &mut O) -> Halt
    where
        S::Leaf: Leaf<C>,
        C: ?Sized,
        O: Observer + ?Sized,
    {
        if !self.is_running() {
            return Halt::AlreadyIdle;
        }
        let (slots, leaves) = self.store.parts();
        let run = &mut Run {
            ctx,
            observer,
            leaves,
        };
        slot::halt(slots, 0, run);
        Halt::Halted
    }

    /// Whether the last tick returned [`Status::Running`] and the tree has not
    /// been halted since.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.store.slots().first().is_some_and(|root| root.running)
    }

    /// How many nodes the tree has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.slots().len()
    }

    /// Always `false`: a tree has at least its root.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.slots().is_empty()
    }

    /// The earliest instant a running `Sleep` or `Timeout` node will fire, or
    /// a looping decorator wants its next run (the instant of the tick that
    /// finished the last one, so already due), if any. Ticking at or after it
    /// lets that node act; leaves wake the host through their own means.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Duration> {
        slot::next_deadline(self.store.slots())
    }
}

#[cfg(test)]
mod tests;
