/// The result of ticking a node.
#[must_use = "a status says whether the node is still running"]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The node finished and achieved its goal.
    Success,
    /// The node finished without achieving its goal.
    Failure,
    /// The node needs more ticks before it can finish.
    Running,
}

/// What [`Tree::halt`](crate::Tree::halt) did. Both are honoured: halting an
/// idle tree is a repeat whose effect already holds, and it changes nothing.
#[must_use = "a halt says whether any leaf was halted"]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Halt {
    /// The tree was running: every active leaf has been halted once.
    Halted,
    /// The tree was not running, so there was nothing to halt.
    AlreadyIdle,
}
