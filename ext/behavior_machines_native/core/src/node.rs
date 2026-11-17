//! The nested definition builder (feature `alloc`).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use crate::{Limit, Otherwise, Step, Threshold};

/// A nested tree definition, for hosts with an allocator. It is only a way to
/// write [`Step`]s: iterating a `Node` yields its steps in pre-order, so it
/// goes straight to [`Tree::new`](crate::Tree::new), which allocates nothing.
///
/// When no capacity is known up front, [`HeapTree::new`](crate::HeapTree::new)
/// takes the same steps.
///
/// ```
/// use core::time::Duration;
/// use behavior_machines::{Node, Tree};
///
/// let tree = Tree::<&str, 8>::new(Node::fallback([
///     Node::sequence([Node::leaf("battery-low"), Node::leaf("charge")]),
///     Node::leaf("patrol").timeout(Duration::from_secs(5)),
/// ]))?;
/// assert!(!tree.is_running());
/// # Ok::<(), behavior_machines::Refusal>(())
/// ```
///
/// The variants mirror [`Step`]; see it for each node's semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node<L> {
    /// [`Step::Leaf`].
    Leaf(L),
    /// [`Step::AlwaysSuccess`].
    AlwaysSuccess,
    /// [`Step::AlwaysFailure`].
    AlwaysFailure,
    /// [`Step::Sleep`].
    Sleep(Duration),
    /// [`Step::Sequence`].
    Sequence(Vec<Self>),
    /// [`Step::ReactiveSequence`].
    ReactiveSequence(Vec<Self>),
    /// [`Step::Fallback`].
    Fallback(Vec<Self>),
    /// [`Step::ReactiveFallback`].
    ReactiveFallback(Vec<Self>),
    /// [`Step::Parallel`].
    Parallel {
        /// The children, all running concurrently.
        children: Vec<Self>,
        /// How many children must succeed.
        success: Threshold,
        /// How many children may fail before the node fails.
        failure: Threshold,
    },
    /// [`Step::IfThenElse`].
    IfThenElse {
        /// Decides the branch, once.
        condition: Box<Self>,
        /// Runs after the condition succeeds.
        then: Box<Self>,
        /// Runs after the condition fails.
        otherwise: Option<Box<Self>>,
    },
    /// [`Step::WhileDoElse`].
    WhileDoElse {
        /// Re-evaluated on every tick.
        condition: Box<Self>,
        /// Runs while the condition succeeds.
        body: Box<Self>,
        /// Runs while the condition fails.
        otherwise: Option<Box<Self>>,
    },
    /// [`Step::Inverter`].
    Inverter(Box<Self>),
    /// [`Step::ForceSuccess`].
    ForceSuccess(Box<Self>),
    /// [`Step::ForceFailure`].
    ForceFailure(Box<Self>),
    /// [`Step::Timeout`].
    Timeout(Duration, Box<Self>),
    /// [`Step::Repeat`].
    Repeat(Limit, Box<Self>),
    /// [`Step::RetryUntilSuccessful`].
    RetryUntilSuccessful(Limit, Box<Self>),
    /// [`Step::KeepRunningUntilFailure`].
    KeepRunningUntilFailure(Box<Self>),
}

impl<L> Node<L> {
    /// Creates a [`Node::Leaf`].
    pub const fn leaf(leaf: L) -> Self {
        Self::Leaf(leaf)
    }

    /// Creates a [`Node::Sleep`].
    #[must_use]
    pub const fn sleep(duration: Duration) -> Self {
        Self::Sleep(duration)
    }

    /// Creates a [`Node::Sequence`].
    pub fn sequence(children: impl IntoIterator<Item = Self>) -> Self {
        Self::Sequence(children.into_iter().collect())
    }

    /// Creates a [`Node::ReactiveSequence`].
    pub fn reactive_sequence(children: impl IntoIterator<Item = Self>) -> Self {
        Self::ReactiveSequence(children.into_iter().collect())
    }

    /// Creates a [`Node::Fallback`].
    pub fn fallback(children: impl IntoIterator<Item = Self>) -> Self {
        Self::Fallback(children.into_iter().collect())
    }

    /// Creates a [`Node::ReactiveFallback`].
    pub fn reactive_fallback(children: impl IntoIterator<Item = Self>) -> Self {
        Self::ReactiveFallback(children.into_iter().collect())
    }

    /// Creates a [`Node::Parallel`].
    pub fn parallel(
        children: impl IntoIterator<Item = Self>,
        success: Threshold,
        failure: Threshold,
    ) -> Self {
        Self::Parallel {
            children: children.into_iter().collect(),
            success,
            failure,
        }
    }

    /// Creates a [`Node::IfThenElse`].
    pub fn if_then_else(condition: Self, then: Self, otherwise: Self) -> Self {
        Self::IfThenElse {
            condition: Box::new(condition),
            then: Box::new(then),
            otherwise: Some(Box::new(otherwise)),
        }
    }

    /// Creates a [`Node::IfThenElse`] without an `otherwise` branch.
    pub fn if_then(condition: Self, then: Self) -> Self {
        Self::IfThenElse {
            condition: Box::new(condition),
            then: Box::new(then),
            otherwise: None,
        }
    }

    /// Creates a [`Node::WhileDoElse`].
    pub fn while_do_else(condition: Self, body: Self, otherwise: Self) -> Self {
        Self::WhileDoElse {
            condition: Box::new(condition),
            body: Box::new(body),
            otherwise: Some(Box::new(otherwise)),
        }
    }

    /// Creates a [`Node::WhileDoElse`] without an `otherwise` branch.
    pub fn while_do(condition: Self, body: Self) -> Self {
        Self::WhileDoElse {
            condition: Box::new(condition),
            body: Box::new(body),
            otherwise: None,
        }
    }

    /// Wraps this node in a [`Node::Inverter`].
    #[must_use]
    pub fn inverter(self) -> Self {
        Self::Inverter(Box::new(self))
    }

    /// Wraps this node in a [`Node::ForceSuccess`].
    #[must_use]
    pub fn force_success(self) -> Self {
        Self::ForceSuccess(Box::new(self))
    }

    /// Wraps this node in a [`Node::ForceFailure`].
    #[must_use]
    pub fn force_failure(self) -> Self {
        Self::ForceFailure(Box::new(self))
    }

    /// Wraps this node in a [`Node::Timeout`] of `limit`.
    #[must_use]
    pub fn timeout(self, limit: Duration) -> Self {
        Self::Timeout(limit, Box::new(self))
    }

    /// Wraps this node in a [`Node::Repeat`].
    #[must_use]
    pub fn repeat(self, limit: Limit) -> Self {
        Self::Repeat(limit, Box::new(self))
    }

    /// Wraps this node in a [`Node::RetryUntilSuccessful`].
    #[must_use]
    pub fn retry_until_successful(self, limit: Limit) -> Self {
        Self::RetryUntilSuccessful(limit, Box::new(self))
    }

    /// Wraps this node in a [`Node::KeepRunningUntilFailure`].
    #[must_use]
    pub fn keep_running_until_failure(self) -> Self {
        Self::KeepRunningUntilFailure(Box::new(self))
    }

    /// The same definition with every leaf passed through `f`, in the
    /// pre-order of [`NodeId`](crate::NodeId)s, so a host can number or count
    /// its leaves.
    pub fn map<M>(self, mut f: impl FnMut(L) -> M) -> Node<M> {
        self.map_with(&mut f)
    }

    fn map_with<M, F: FnMut(L) -> M>(self, f: &mut F) -> Node<M> {
        let mut boxed = |node: Box<Self>| Box::new(node.map_with(f));
        match self {
            Self::Leaf(leaf) => Node::Leaf(f(leaf)),
            Self::AlwaysSuccess => Node::AlwaysSuccess,
            Self::AlwaysFailure => Node::AlwaysFailure,
            Self::Sleep(duration) => Node::Sleep(duration),
            Self::Sequence(children) => Node::Sequence(map_all(children, f)),
            Self::ReactiveSequence(children) => Node::ReactiveSequence(map_all(children, f)),
            Self::Fallback(children) => Node::Fallback(map_all(children, f)),
            Self::ReactiveFallback(children) => Node::ReactiveFallback(map_all(children, f)),
            Self::Parallel {
                children,
                success,
                failure,
            } => Node::Parallel {
                children: map_all(children, f),
                success,
                failure,
            },
            Self::IfThenElse {
                condition,
                then,
                otherwise,
            } => {
                let condition = boxed(condition);
                let then = boxed(then);
                Node::IfThenElse {
                    condition,
                    then,
                    otherwise: otherwise.map(boxed),
                }
            }
            Self::WhileDoElse {
                condition,
                body,
                otherwise,
            } => {
                let condition = boxed(condition);
                let body = boxed(body);
                Node::WhileDoElse {
                    condition,
                    body,
                    otherwise: otherwise.map(boxed),
                }
            }
            Self::Inverter(child) => Node::Inverter(boxed(child)),
            Self::ForceSuccess(child) => Node::ForceSuccess(boxed(child)),
            Self::ForceFailure(child) => Node::ForceFailure(boxed(child)),
            Self::Timeout(limit, child) => Node::Timeout(limit, boxed(child)),
            Self::Repeat(limit, child) => Node::Repeat(limit, boxed(child)),
            Self::RetryUntilSuccessful(limit, child) => {
                Node::RetryUntilSuccessful(limit, boxed(child))
            }
            Self::KeepRunningUntilFailure(child) => Node::KeepRunningUntilFailure(boxed(child)),
        }
    }

    /// This node's steps with every leaf borrowed, in pre-order.
    #[must_use]
    pub fn iter(&self) -> StepsRef<'_, L> {
        StepsRef {
            pending: alloc::vec![self],
        }
    }

    /// This node's own step, and its children in order.
    fn unfold(self) -> (Step<L>, Vec<Self>) {
        let otherwise = |branch: &Option<Box<Self>>| match branch {
            Some(_) => Otherwise::Present,
            None => Otherwise::Absent,
        };
        let three = |first: Box<Self>, second: Box<Self>, third: Option<Box<Self>>| {
            let mut children = alloc::vec![*first, *second];
            children.extend(third.map(|node| *node));
            children
        };
        match self {
            Self::Leaf(leaf) => (Step::Leaf(leaf), Vec::new()),
            Self::AlwaysSuccess => (Step::AlwaysSuccess, Vec::new()),
            Self::AlwaysFailure => (Step::AlwaysFailure, Vec::new()),
            Self::Sleep(duration) => (Step::Sleep(duration), Vec::new()),
            Self::Sequence(children) => (Step::Sequence(children.len()), children),
            Self::ReactiveSequence(children) => (Step::ReactiveSequence(children.len()), children),
            Self::Fallback(children) => (Step::Fallback(children.len()), children),
            Self::ReactiveFallback(children) => (Step::ReactiveFallback(children.len()), children),
            Self::Parallel {
                children,
                success,
                failure,
            } => (
                Step::Parallel {
                    children: children.len(),
                    success,
                    failure,
                },
                children,
            ),
            Self::IfThenElse {
                condition,
                then,
                otherwise: other,
            } => (
                Step::IfThenElse(otherwise(&other)),
                three(condition, then, other),
            ),
            Self::WhileDoElse {
                condition,
                body,
                otherwise: other,
            } => (
                Step::WhileDoElse(otherwise(&other)),
                three(condition, body, other),
            ),
            Self::Inverter(child) => (Step::Inverter, alloc::vec![*child]),
            Self::ForceSuccess(child) => (Step::ForceSuccess, alloc::vec![*child]),
            Self::ForceFailure(child) => (Step::ForceFailure, alloc::vec![*child]),
            Self::Timeout(limit, child) => (Step::Timeout(limit), alloc::vec![*child]),
            Self::Repeat(limit, child) => (Step::Repeat(limit), alloc::vec![*child]),
            Self::RetryUntilSuccessful(limit, child) => {
                (Step::RetryUntilSuccessful(limit), alloc::vec![*child])
            }
            Self::KeepRunningUntilFailure(child) => {
                (Step::KeepRunningUntilFailure, alloc::vec![*child])
            }
        }
    }

    /// This node's own step with its leaf borrowed, and its children.
    fn unfold_ref(&self) -> (Step<&L>, Vec<&Self>) {
        let present = |branch: Option<&Self>| match branch {
            Some(_) => Otherwise::Present,
            None => Otherwise::Absent,
        };
        match self {
            Self::Leaf(leaf) => (Step::Leaf(leaf), Vec::new()),
            Self::AlwaysSuccess => (Step::AlwaysSuccess, Vec::new()),
            Self::AlwaysFailure => (Step::AlwaysFailure, Vec::new()),
            Self::Sleep(duration) => (Step::Sleep(*duration), Vec::new()),
            Self::Sequence(children) => (Step::Sequence(children.len()), children.iter().collect()),
            Self::ReactiveSequence(children) => (
                Step::ReactiveSequence(children.len()),
                children.iter().collect(),
            ),
            Self::Fallback(children) => (Step::Fallback(children.len()), children.iter().collect()),
            Self::ReactiveFallback(children) => (
                Step::ReactiveFallback(children.len()),
                children.iter().collect(),
            ),
            Self::Parallel {
                children,
                success,
                failure,
            } => (
                Step::Parallel {
                    children: children.len(),
                    success: *success,
                    failure: *failure,
                },
                children.iter().collect(),
            ),
            Self::IfThenElse {
                condition,
                then,
                otherwise,
            } => {
                let otherwise = otherwise.as_deref();
                (
                    Step::IfThenElse(present(otherwise)),
                    branches(condition, then, otherwise),
                )
            }
            Self::WhileDoElse {
                condition,
                body,
                otherwise,
            } => {
                let otherwise = otherwise.as_deref();
                (
                    Step::WhileDoElse(present(otherwise)),
                    branches(condition, body, otherwise),
                )
            }
            Self::Inverter(child) => (Step::Inverter, alloc::vec![&**child]),
            Self::ForceSuccess(child) => (Step::ForceSuccess, alloc::vec![&**child]),
            Self::ForceFailure(child) => (Step::ForceFailure, alloc::vec![&**child]),
            Self::Timeout(limit, child) => (Step::Timeout(*limit), alloc::vec![&**child]),
            Self::Repeat(limit, child) => (Step::Repeat(*limit), alloc::vec![&**child]),
            Self::RetryUntilSuccessful(limit, child) => {
                (Step::RetryUntilSuccessful(*limit), alloc::vec![&**child])
            }
            Self::KeepRunningUntilFailure(child) => {
                (Step::KeepRunningUntilFailure, alloc::vec![&**child])
            }
        }
    }
}

fn map_all<L, M, F: FnMut(L) -> M>(children: Vec<Node<L>>, f: &mut F) -> Vec<Node<M>> {
    children
        .into_iter()
        .map(|child| child.map_with(f))
        .collect()
}

/// A conditional's children, borrowed, in order.
fn branches<'a, L>(
    first: &'a Node<L>,
    second: &'a Node<L>,
    third: Option<&'a Node<L>>,
) -> Vec<&'a Node<L>> {
    let mut children = alloc::vec![first, second];
    children.extend(third);
    children
}

/// The steps of a [`Node`], in pre-order, consuming it.
#[derive(Debug)]
pub struct Steps<L> {
    pending: Vec<Node<L>>,
}

/// The steps of a borrowed [`Node`], in pre-order, with every leaf borrowed.
#[derive(Debug)]
pub struct StepsRef<'a, L> {
    pending: Vec<&'a Node<L>>,
}

impl<L> IntoIterator for Node<L> {
    type Item = Step<L>;
    type IntoIter = Steps<L>;

    fn into_iter(self) -> Steps<L> {
        Steps {
            pending: alloc::vec![self],
        }
    }
}

impl<'a, L> IntoIterator for &'a Node<L> {
    type Item = Step<&'a L>;
    type IntoIter = StepsRef<'a, L>;

    fn into_iter(self) -> StepsRef<'a, L> {
        self.iter()
    }
}

impl<L> Iterator for Steps<L> {
    type Item = Step<L>;

    fn next(&mut self) -> Option<Step<L>> {
        let (step, children) = self.pending.pop()?.unfold();
        self.pending.extend(children.into_iter().rev());
        Some(step)
    }
}

impl<'a, L> Iterator for StepsRef<'a, L> {
    type Item = Step<&'a L>;

    fn next(&mut self) -> Option<Step<&'a L>> {
        let (step, children) = self.pending.pop()?.unfold_ref();
        self.pending.extend(children.into_iter().rev());
        Some(step)
    }
}

#[cfg(test)]
mod tests;
