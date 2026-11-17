use core::fmt;

use crate::Composite;

/// Every reason the crate refuses a call. A refusal changes nothing: a
/// refused [`Tree::new`](crate::Tree::new) builds no tree, and a refused
/// [`Tree::tick`](crate::Tree::tick) runs no leaf and records no clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The definition has more nodes than an inline tree's capacity `N`, or
    /// a heap tree could not allocate room for the next one.
    Full,
    /// A composite step declared no children, so its result would be
    /// arbitrary.
    EmptyComposite(Composite),
    /// A [`Step::Parallel`](crate::Step::Parallel) threshold is zero or
    /// larger than its child count.
    ParallelThreshold {
        /// The requested threshold.
        threshold: usize,
        /// How many children the node has.
        children: usize,
    },
    /// The steps ended before every declared child was given, or there were
    /// none at all.
    Incomplete,
    /// A step came after the root was complete.
    Trailing,
    /// The tick's `now` is earlier than the latest honoured tick's. The
    /// answer is about the instant supplied: the same call at a later
    /// instant may be honoured.
    TimeRegression,
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("the definition exceeds the tree's capacity"),
            Self::EmptyComposite(kind) => write!(formatter, "{kind} needs at least one child"),
            Self::ParallelThreshold {
                threshold,
                children,
            } => write!(
                formatter,
                "Parallel threshold {threshold} must be between 1 and {children}"
            ),
            Self::Incomplete => formatter.write_str("the steps end before every child is given"),
            Self::Trailing => formatter.write_str("a step follows the complete root"),
            Self::TimeRegression => {
                formatter.write_str("now is earlier than the latest honoured tick")
            }
        }
    }
}

impl core::error::Error for Refusal {}
