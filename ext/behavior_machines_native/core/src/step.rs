use core::convert::Infallible;
use core::fmt;
use core::time::Duration;

use crate::Refusal;

/// One node of a definition, in the depth-first pre-order a [`Tree`] is
/// built from: each step is followed by its children's steps, in order.
///
/// A step's position in that sequence is its [`NodeId`](crate::NodeId).
/// Composites declare how many children follow; `IfThenElse` and
/// `WhileDoElse` are followed by the condition, the branch it selects on
/// success, then the `otherwise` branch if [`Otherwise::Present`];
/// decorators by exactly one child.
///
/// Decorators that loop (`Repeat`, `RetryUntilSuccessful`,
/// `KeepRunningUntilFailure`) finish at most one run of their child per tick,
/// so a child that finishes instantly cannot spin forever inside one tick.
/// Between runs they are due at once by
/// [`Tree::next_deadline`](crate::Tree::next_deadline).
///
/// [`Tree`]: crate::Tree
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step<L> {
    /// A condition check or action.
    Leaf(L),
    /// Succeeds immediately.
    AlwaysSuccess,
    /// Fails immediately.
    AlwaysFailure,
    /// Runs until the duration has passed since it started, then succeeds.
    /// A zero duration succeeds immediately. The deadline is fixed when the
    /// node starts; if the start plus the duration exceeds `Duration::MAX`,
    /// that deadline never fires and the node stays running (the sum is
    /// neither refused nor clamped).
    Sleep(Duration),
    /// Ticks this many children in order. Fails as soon as one fails and
    /// succeeds once all succeed. A running child is resumed on the next
    /// tick.
    Sequence(usize),
    /// A [`Step::Sequence`] that restarts from its first child on every
    /// tick, so earlier children act as guards: once one stops succeeding,
    /// the running later child is halted.
    ReactiveSequence(usize),
    /// Ticks this many children in order. Succeeds as soon as one succeeds
    /// and fails once all fail. A running child is resumed on the next tick.
    Fallback(usize),
    /// A [`Step::Fallback`] that restarts from its first child on every
    /// tick, so a higher-priority child that starts succeeding or running
    /// preempts the running lower-priority child, which is halted.
    ReactiveFallback(usize),
    /// Ticks every unfinished child, in order, on each tick. Succeeds once
    /// `success` children have succeeded; fails once `failure` children have
    /// failed or success can no longer be reached. Running children are
    /// halted when it decides, and the leftmost child decides ties.
    Parallel {
        /// How many children follow, all running concurrently.
        children: usize,
        /// How many children must succeed.
        success: Threshold,
        /// How many children may fail before the node fails.
        failure: Threshold,
    },
    /// Ticks the condition until it finishes, then runs the `then` branch if
    /// it succeeded or `otherwise` if it failed, and reports that branch's
    /// status. Without `otherwise`, a failed condition fails the node. The
    /// condition is not ticked again while the branch runs.
    IfThenElse(Otherwise),
    /// The reactive [`Step::IfThenElse`]: ticks the condition on every tick
    /// and runs the body while it succeeds or `otherwise` while it fails,
    /// halting the other branch first when the condition flips. While the
    /// condition runs, the branches are paused: not ticked and not halted.
    /// Without `otherwise`, a failed condition fails the node.
    WhileDoElse(Otherwise),
    /// Swaps the child's [`Success`](crate::Status::Success) and
    /// [`Failure`](crate::Status::Failure).
    Inverter,
    /// Reports any finished child as [`Success`](crate::Status::Success).
    ForceSuccess,
    /// Reports any finished child as [`Failure`](crate::Status::Failure).
    ForceFailure,
    /// Fails, halting the child, once the duration has passed since this node
    /// started. The child only ticks strictly before the deadline, so a zero
    /// duration fails without ticking it. To run without a limit, leave the
    /// decorator out. The deadline is fixed when the node starts; if the
    /// start plus the duration exceeds `Duration::MAX`, it never fires and the
    /// child is never failed for elapsed time (the sum is neither refused nor
    /// clamped).
    Timeout(Duration),
    /// Runs the child again each time it succeeds, until it has succeeded
    /// `limit` times, then succeeds. Fails as soon as the child fails.
    Repeat(Limit),
    /// Runs the child again each time it fails, until it has failed `limit`
    /// times, then fails. Succeeds as soon as the child succeeds.
    RetryUntilSuccessful(Limit),
    /// Runs the child again each time it succeeds, forever. Fails as soon as
    /// the child fails.
    KeepRunningUntilFailure,
}

/// Whether an `IfThenElse` or `WhileDoElse` has an `otherwise` branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Otherwise {
    /// Two children follow: the condition and one branch.
    Absent,
    /// Three children follow: the condition and both branches.
    Present,
}

/// How many children of a [`Step::Parallel`] a decision needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Threshold {
    /// Every child.
    All,
    /// This many children, between 1 and the number of children. Anything
    /// else is refused when the tree is built.
    Count(usize),
}

/// How many runs a looping decorator allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Limit {
    /// This many runs. Zero finishes immediately without ticking the child.
    Times(u64),
    /// No limit.
    Unlimited,
}

/// The steps that need at least one child, as reported by
/// [`Refusal::EmptyComposite`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Composite {
    /// [`Step::Sequence`].
    Sequence,
    /// [`Step::ReactiveSequence`].
    ReactiveSequence,
    /// [`Step::Fallback`].
    Fallback,
    /// [`Step::ReactiveFallback`].
    ReactiveFallback,
    /// [`Step::Parallel`].
    Parallel,
}

impl fmt::Display for Composite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Sequence => "Sequence",
            Self::ReactiveSequence => "ReactiveSequence",
            Self::Fallback => "Fallback",
            Self::ReactiveFallback => "ReactiveFallback",
            Self::Parallel => "Parallel",
        })
    }
}

impl<L> Step<L> {
    /// How many children follow this step.
    #[must_use]
    pub const fn children(&self) -> usize {
        match self {
            Self::Leaf(_) | Self::AlwaysSuccess | Self::AlwaysFailure | Self::Sleep(_) => 0,
            Self::Sequence(children)
            | Self::ReactiveSequence(children)
            | Self::Fallback(children)
            | Self::ReactiveFallback(children)
            | Self::Parallel { children, .. } => *children,
            Self::IfThenElse(Otherwise::Absent) | Self::WhileDoElse(Otherwise::Absent) => 2,
            Self::IfThenElse(Otherwise::Present) | Self::WhileDoElse(Otherwise::Present) => 3,
            Self::Inverter
            | Self::ForceSuccess
            | Self::ForceFailure
            | Self::Timeout(_)
            | Self::Repeat(_)
            | Self::RetryUntilSuccessful(_)
            | Self::KeepRunningUntilFailure => 1,
        }
    }

    /// The check [`Tree::new`](crate::Tree::new) makes of every step on its
    /// own, before placing it: a composite has children, and a parallel's
    /// thresholds fit its child count.
    ///
    /// # Errors
    ///
    /// [`Refusal::EmptyComposite`] or [`Refusal::ParallelThreshold`].
    pub const fn validate(&self) -> Result<(), Refusal> {
        let (kind, children) = match self {
            Self::Sequence(children) => (Composite::Sequence, *children),
            Self::ReactiveSequence(children) => (Composite::ReactiveSequence, *children),
            Self::Fallback(children) => (Composite::Fallback, *children),
            Self::ReactiveFallback(children) => (Composite::ReactiveFallback, *children),
            Self::Parallel {
                children,
                success,
                failure,
            } => {
                if *children == 0 {
                    return Err(Refusal::EmptyComposite(Composite::Parallel));
                }
                if let Err(refusal) = success.resolve(*children) {
                    return Err(refusal);
                }
                return match failure.resolve(*children) {
                    Ok(_) => Ok(()),
                    Err(refusal) => Err(refusal),
                };
            }
            Self::Leaf(_)
            | Self::AlwaysSuccess
            | Self::AlwaysFailure
            | Self::Sleep(_)
            | Self::IfThenElse(_)
            | Self::WhileDoElse(_)
            | Self::Inverter
            | Self::ForceSuccess
            | Self::ForceFailure
            | Self::Timeout(_)
            | Self::Repeat(_)
            | Self::RetryUntilSuccessful(_)
            | Self::KeepRunningUntilFailure => return Ok(()),
        };
        if children == 0 {
            Err(Refusal::EmptyComposite(kind))
        } else {
            Ok(())
        }
    }

    /// The same step with its leaf borrowed.
    #[must_use]
    pub const fn as_ref(&self) -> Step<&L> {
        match self {
            Self::Leaf(leaf) => Step::Leaf(leaf),
            Self::AlwaysSuccess => Step::AlwaysSuccess,
            Self::AlwaysFailure => Step::AlwaysFailure,
            Self::Sleep(duration) => Step::Sleep(*duration),
            Self::Sequence(children) => Step::Sequence(*children),
            Self::ReactiveSequence(children) => Step::ReactiveSequence(*children),
            Self::Fallback(children) => Step::Fallback(*children),
            Self::ReactiveFallback(children) => Step::ReactiveFallback(*children),
            Self::Parallel {
                children,
                success,
                failure,
            } => Step::Parallel {
                children: *children,
                success: *success,
                failure: *failure,
            },
            Self::IfThenElse(otherwise) => Step::IfThenElse(*otherwise),
            Self::WhileDoElse(otherwise) => Step::WhileDoElse(*otherwise),
            Self::Inverter => Step::Inverter,
            Self::ForceSuccess => Step::ForceSuccess,
            Self::ForceFailure => Step::ForceFailure,
            Self::Timeout(limit) => Step::Timeout(*limit),
            Self::Repeat(limit) => Step::Repeat(*limit),
            Self::RetryUntilSuccessful(limit) => Step::RetryUntilSuccessful(*limit),
            Self::KeepRunningUntilFailure => Step::KeepRunningUntilFailure,
        }
    }

    /// The same step with its leaf passed through `f`.
    pub fn map<M>(self, f: impl FnOnce(L) -> M) -> Step<M> {
        match self.try_map(|leaf| Ok::<_, Infallible>(f(leaf))) {
            Ok(step) => step,
            Err(never) => match never {},
        }
    }

    /// The same step with its leaf passed through a fallible `f`: `f`'s
    /// error, or the mapped step. `f` runs only for a leaf.
    ///
    /// # Errors
    ///
    /// Whatever `f` returns for the leaf.
    pub fn try_map<M, E>(self, f: impl FnOnce(L) -> Result<M, E>) -> Result<Step<M>, E> {
        Ok(match self {
            Self::Leaf(leaf) => Step::Leaf(f(leaf)?),
            Self::AlwaysSuccess => Step::AlwaysSuccess,
            Self::AlwaysFailure => Step::AlwaysFailure,
            Self::Sleep(duration) => Step::Sleep(duration),
            Self::Sequence(children) => Step::Sequence(children),
            Self::ReactiveSequence(children) => Step::ReactiveSequence(children),
            Self::Fallback(children) => Step::Fallback(children),
            Self::ReactiveFallback(children) => Step::ReactiveFallback(children),
            Self::Parallel {
                children,
                success,
                failure,
            } => Step::Parallel {
                children,
                success,
                failure,
            },
            Self::IfThenElse(otherwise) => Step::IfThenElse(otherwise),
            Self::WhileDoElse(otherwise) => Step::WhileDoElse(otherwise),
            Self::Inverter => Step::Inverter,
            Self::ForceSuccess => Step::ForceSuccess,
            Self::ForceFailure => Step::ForceFailure,
            Self::Timeout(limit) => Step::Timeout(limit),
            Self::Repeat(limit) => Step::Repeat(limit),
            Self::RetryUntilSuccessful(limit) => Step::RetryUntilSuccessful(limit),
            Self::KeepRunningUntilFailure => Step::KeepRunningUntilFailure,
        })
    }
}

impl Threshold {
    /// The child count this threshold stands for among `children`.
    pub(crate) const fn resolve(self, children: usize) -> Result<usize, Refusal> {
        match self {
            Self::All => Ok(children),
            Self::Count(threshold) if threshold >= 1 && threshold <= children => Ok(threshold),
            Self::Count(threshold) => Err(Refusal::ParallelThreshold {
                threshold,
                children,
            }),
        }
    }
}
