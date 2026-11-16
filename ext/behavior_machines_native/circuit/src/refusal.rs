use core::fmt;

use behavior_machines::Composite;

/// Every reason the crate refuses a call. A refusal changes nothing: no
/// circuit is built, no state advances, and no leaf runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The definition contains a step this version does not lower.
    Unsupported(Unsupported),
    /// A composite step declared no children, which the engine also refuses.
    EmptyComposite(Composite),
    /// The steps ended before every declared child was given, or there were
    /// none at all.
    Incomplete,
    /// A step came after the root was complete.
    Trailing,
    /// The leaf statuses did not match the tree's leaf count.
    LeafCount {
        /// How many leaves the tree has.
        expected: usize,
        /// How many statuses were given.
        given: usize,
    },
    /// The state's node flags did not come from this lowered tree.
    StateShape {
        /// How many nodes the tree has.
        expected: usize,
        /// How many the state holds.
        given: usize,
    },
    /// The state's cursors did not come from this lowered tree.
    CursorShape {
        /// How many memory composites the tree has.
        expected: usize,
        /// How many cursors the state holds.
        given: usize,
    },
    /// The input slice did not match the circuit's input width.
    InputCount {
        /// The circuit's input width.
        expected: usize,
        /// How many were given.
        given: usize,
    },
    /// An output that should be 0 or 1 was neither. A bug in this crate.
    NotABit {
        /// Which output.
        what: &'static str,
        /// The node it belongs to.
        node: usize,
    },
    /// A status triple had more than one bit set. A bug in this crate.
    NotOneHot {
        /// The node.
        node: usize,
    },
    /// A cursor was negative or fractional. A bug in this crate.
    BadCursor {
        /// The cursor cell.
        cell: usize,
    },
    /// An [`Executor`](crate::Executor) tick's `now` is earlier than the
    /// latest honoured tick's. The answer is about the instant supplied: the
    /// same call at a later instant may be honoured.
    TimeRegression,
}

/// The steps the circuit does not lower yet, named in
/// [`Refusal::Unsupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// `Step::Sleep`.
    Sleep,
    /// `Step::Parallel`.
    Parallel,
    /// `Step::IfThenElse`.
    IfThenElse,
    /// `Step::WhileDoElse`.
    WhileDoElse,
    /// `Step::Timeout`.
    Timeout,
    /// `Step::Repeat`.
    Repeat,
    /// `Step::RetryUntilSuccessful`.
    RetryUntilSuccessful,
    /// `Step::KeepRunningUntilFailure`.
    KeepRunningUntilFailure,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Sleep => "Sleep",
            Self::Parallel => "Parallel",
            Self::IfThenElse => "IfThenElse",
            Self::WhileDoElse => "WhileDoElse",
            Self::Timeout => "Timeout",
            Self::Repeat => "Repeat",
            Self::RetryUntilSuccessful => "RetryUntilSuccessful",
            Self::KeepRunningUntilFailure => "KeepRunningUntilFailure",
        })
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(node) => write!(formatter, "{node} is not lowered yet"),
            Self::EmptyComposite(kind) => write!(formatter, "{kind} needs at least one child"),
            Self::Incomplete => formatter.write_str("the steps end before every child is given"),
            Self::Trailing => formatter.write_str("a step follows the complete root"),
            Self::LeafCount { expected, given } => {
                write!(
                    formatter,
                    "tree has {expected} leaves, got {given} statuses"
                )
            }
            Self::StateShape { expected, given } => write!(
                formatter,
                "state has {given} node flags, tree has {expected} nodes"
            ),
            Self::CursorShape { expected, given } => {
                write!(formatter, "state has {given} cursors, tree has {expected}")
            }
            Self::InputCount { expected, given } => {
                write!(
                    formatter,
                    "circuit takes {expected} input cells, got {given}"
                )
            }
            Self::NotABit { what, node } => write!(formatter, "node {node}: {what} is not a bit"),
            Self::NotOneHot { node } => {
                write!(formatter, "node {node}: status has more than one bit set")
            }
            Self::BadCursor { cell } => {
                write!(
                    formatter,
                    "cursor cell {cell} is not a non-negative integer"
                )
            }
            Self::TimeRegression => {
                formatter.write_str("now is earlier than the latest honoured tick")
            }
        }
    }
}

impl core::error::Error for Refusal {}
