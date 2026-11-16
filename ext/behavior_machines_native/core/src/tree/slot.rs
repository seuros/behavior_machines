//! The compiled nodes and the walk over them.
//!
//! Nodes sit in pre-order, so a node's subtree is the `size` slots starting
//! at it: the node itself, then each child's subtree in turn. Every walk
//! takes a node's subtree as a slice and splits its children off it, which
//! keeps every borrow disjoint without an index into the wrong place.

use core::time::Duration;

use super::composite;
use super::control::{self, Selected};
use super::decorator::{self, Map};
use crate::observe::{NodeId, Observer};
use crate::{Leaf, Limit, Status, Step, Threshold};

/// What a tick or halt needs besides the nodes themselves.
pub(super) struct Run<'a, L, C: ?Sized, O: ?Sized> {
    pub ctx: &'a mut C,
    pub observer: &'a mut O,
    pub leaves: &'a [Option<L>],
}

/// One compiled node. `running` is set while the node's last tick returned
/// `Running` and it has not been halted since; only running nodes are halted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Slot {
    /// How many slots this node's subtree spans, itself included.
    pub size: usize,
    pub running: bool,
    /// This node's result within its parent `Parallel`'s current activation,
    /// once it finished; `None` everywhere else.
    pub outcome: Option<Status>,
    pub kind: Kind,
}

/// Whether a composite restarts from its first child on every tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Memory,
    Reactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Leaf,
    /// `AlwaysSuccess` and `AlwaysFailure`.
    Constant(Status),
    Sleep {
        duration: Duration,
        /// `None` until started, or when the deadline is past the clock's end.
        deadline: Option<Duration>,
    },
    /// Sequence advances past children that succeed, fallback past children
    /// that fail. A memory node resumes at `cursor`; a reactive one restarts
    /// from the first child every tick and keeps it at zero.
    Ordered {
        advance_on: Status,
        mode: Mode,
        cursor: usize,
    },
    /// Thresholds resolved to counts.
    Parallel {
        success: usize,
        failure: usize,
    },
    /// `IfThenElse` (`Memory`) and `WhileDoElse` (`Reactive`); `selected`
    /// is the branch an `IfThenElse` committed to, until it finishes.
    Conditional {
        mode: Mode,
        selected: Option<Selected>,
    },
    Map(Map),
    Timeout {
        limit: Duration,
        /// `None` until started, or when the deadline is past the clock's end.
        deadline: Option<Duration>,
    },
    /// `Repeat` and `KeepRunningUntilFailure` loop on `Success`,
    /// `RetryUntilSuccessful` on `Failure`; `runs` counts finished runs.
    Loop {
        again_on: Status,
        limit: Option<u64>,
        runs: u64,
        /// The tick that finished a run, while the next one waits for a tick.
        due: Option<Duration>,
    },
}

impl Slot {
    /// A placeholder for a slot the definition does not reach.
    pub const fn idle() -> Self {
        Self::new(Kind::Leaf, 0)
    }

    pub const fn new(kind: Kind, size: usize) -> Self {
        Self {
            size,
            running: false,
            outcome: None,
            kind,
        }
    }
}

impl Kind {
    /// The initial state of a validated step.
    pub fn of(step: Step<()>) -> Self {
        let ordered = |advance_on, mode| Self::Ordered {
            advance_on,
            mode,
            cursor: 0,
        };
        let looped = |again_on, limit| Self::Loop {
            again_on,
            limit: match limit {
                Limit::Times(times) => Some(times),
                Limit::Unlimited => None,
            },
            runs: 0,
            due: None,
        };
        let conditional = |mode| Self::Conditional {
            mode,
            selected: None,
        };
        match step {
            Step::Leaf(()) => Self::Leaf,
            Step::AlwaysSuccess => Self::Constant(Status::Success),
            Step::AlwaysFailure => Self::Constant(Status::Failure),
            Step::Sleep(duration) => Self::Sleep {
                duration,
                deadline: None,
            },
            Step::Sequence(_) => ordered(Status::Success, Mode::Memory),
            Step::ReactiveSequence(_) => ordered(Status::Success, Mode::Reactive),
            Step::Fallback(_) => ordered(Status::Failure, Mode::Memory),
            Step::ReactiveFallback(_) => ordered(Status::Failure, Mode::Reactive),
            Step::Parallel {
                children,
                success,
                failure,
            } => Self::Parallel {
                success: resolved(success, children),
                failure: resolved(failure, children),
            },
            Step::IfThenElse(_) => conditional(Mode::Memory),
            Step::WhileDoElse(_) => conditional(Mode::Reactive),
            Step::Inverter => Self::Map(Map::Inverter),
            Step::ForceSuccess => Self::Map(Map::Force(Status::Success)),
            Step::ForceFailure => Self::Map(Map::Force(Status::Failure)),
            Step::Timeout(limit) => Self::Timeout {
                limit,
                deadline: None,
            },
            Step::Repeat(limit) => looped(Status::Success, limit),
            Step::RetryUntilSuccessful(limit) => looped(Status::Failure, limit),
            Step::KeepRunningUntilFailure => looped(Status::Success, Limit::Unlimited),
        }
    }
}

/// A threshold the step already validated against `children`.
fn resolved(threshold: Threshold, children: usize) -> usize {
    threshold.resolve(children).unwrap_or_else(|_| {
        debug_assert!(false, "Tree::new validates every step before compiling it");
        children
    })
}

/// The subtrees of a node's children, in order, each with its node index.
pub(super) struct Children<'a> {
    rest: &'a mut [Slot],
    at: usize,
}

impl<'a> Iterator for Children<'a> {
    type Item = (&'a mut [Slot], usize);

    fn next(&mut self) -> Option<Self::Item> {
        let rest = core::mem::take(&mut self.rest);
        let size = rest.first()?.size;
        if size == 0 {
            debug_assert!(false, "every compiled node spans at least itself");
            return None;
        }
        let (child, tail) = rest.split_at_mut_checked(size)?;
        let at = self.at;
        self.at += size;
        self.rest = tail;
        Some((child, at))
    }
}

/// The children below a node: `below` is its subtree without itself, whose
/// first slot has index `at`.
pub(super) const fn children(below: &mut [Slot], at: usize) -> Children<'_> {
    Children { rest: below, at }
}

/// The children's subtrees, read-only.
fn children_ref(mut rest: &[Slot]) -> impl Iterator<Item = &[Slot]> {
    core::iter::from_fn(move || {
        let size = rest.first()?.size;
        if size == 0 {
            return None;
        }
        let (child, tail) = rest.split_at_checked(size)?;
        rest = tail;
        Some(child)
    })
}

/// Ticks the node at the head of `nodes`, whose index is `at`.
pub(super) fn tick<L, C, O>(
    nodes: &mut [Slot],
    at: usize,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let Some((node, below)) = nodes.split_first_mut() else {
        debug_assert!(false, "a tick always reaches a compiled node");
        return Status::Failure;
    };
    let starting = !node.running;
    let first = at + 1;
    let status = match &mut node.kind {
        Kind::Leaf => {
            if let Some(leaf) = run.leaves.get(at).and_then(Option::as_ref) {
                leaf.tick(run.ctx, now)
            } else {
                debug_assert!(false, "every leaf node holds its leaf");
                Status::Failure
            }
        }
        Kind::Constant(status) => *status,
        Kind::Sleep { duration, deadline } => decorator::sleep(*duration, deadline, starting, now),
        Kind::Ordered {
            advance_on,
            mode: Mode::Memory,
            cursor,
        } => composite::ordered(children(below, first), cursor, *advance_on, run, now),
        Kind::Ordered {
            advance_on,
            mode: Mode::Reactive,
            ..
        } => composite::reactive(children(below, first), *advance_on, run, now),
        Kind::Parallel { success, failure } => {
            composite::parallel(below, first, *success, *failure, run, now)
        }
        Kind::Conditional {
            mode: Mode::Memory,
            selected,
        } => control::if_then_else(children(below, first), selected, run, now),
        Kind::Conditional {
            mode: Mode::Reactive,
            ..
        } => control::while_do_else(children(below, first), run, now),
        Kind::Map(map) => map.apply(tick(below, first, run, now)),
        Kind::Timeout { limit, deadline } => {
            decorator::timeout(below, first, *limit, deadline, starting, run, now)
        }
        Kind::Loop {
            again_on,
            limit,
            runs,
            due,
        } => {
            let looping = decorator::Looping {
                again_on: *again_on,
                limit: *limit,
                runs,
                due,
            };
            decorator::looped(below, first, looping, run, now)
        }
    };
    node.running = status == Status::Running;
    run.observer.ticked(NodeId::new(at), status);
    status
}

/// Halts the node at the head of `nodes` if it is running, and everything
/// running under it.
pub(super) fn halt<L, C, O>(nodes: &mut [Slot], at: usize, run: &mut Run<'_, L, C, O>)
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let Some((node, below)) = nodes.split_first_mut() else {
        return;
    };
    if !node.running {
        return;
    }
    node.running = false;
    match &mut node.kind {
        Kind::Leaf => {
            if let Some(leaf) = run.leaves.get(at).and_then(Option::as_ref) {
                leaf.halt(run.ctx);
            }
        }
        Kind::Constant(_) | Kind::Sleep { .. } => {}
        Kind::Ordered { cursor, .. } => {
            halt_all(children(below, at + 1), run);
            *cursor = 0;
        }
        Kind::Parallel { .. } => composite::reset(below, at + 1, run),
        Kind::Conditional { selected, .. } => {
            halt_all(children(below, at + 1), run);
            *selected = None;
        }
        Kind::Map(_) | Kind::Timeout { .. } => halt(below, at + 1, run),
        Kind::Loop { runs, .. } => {
            halt(below, at + 1, run);
            *runs = 0;
        }
    }
    run.observer.halted(NodeId::new(at));
}

/// Halts every running child. Finished and never-started children are not
/// running, so they are skipped.
pub(super) fn halt_all<L, C, O>(children: Children<'_>, run: &mut Run<'_, L, C, O>)
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    for (child, at) in children {
        halt(child, at, run);
    }
}

/// The earliest deadline of a running timed or looping node in this subtree.
pub(super) fn next_deadline(nodes: &[Slot]) -> Option<Duration> {
    let (node, below) = nodes.split_first()?;
    if !node.running {
        return None;
    }
    let below_deadline = || earliest(children_ref(below).map(next_deadline));
    match node.kind {
        Kind::Leaf | Kind::Constant(_) => None,
        Kind::Sleep { deadline, .. } => deadline,
        Kind::Timeout { deadline, .. } => earliest([deadline, below_deadline()]),
        Kind::Loop { due, .. } => earliest([due, below_deadline()]),
        Kind::Map(_)
        | Kind::Ordered { .. }
        | Kind::Parallel { .. }
        | Kind::Conditional {
            mode: Mode::Memory, ..
        } => below_deadline(),
        Kind::Conditional {
            mode: Mode::Reactive,
            ..
        } => {
            // A running `WhileDoElse` condition pauses the branches, so their
            // timers cannot act until it finishes.
            let condition = children_ref(below).next()?;
            if condition.first().is_some_and(|slot| slot.running) {
                next_deadline(condition)
            } else {
                below_deadline()
            }
        }
    }
}

fn earliest(deadlines: impl IntoIterator<Item = Option<Duration>>) -> Option<Duration> {
    deadlines.into_iter().flatten().min()
}
