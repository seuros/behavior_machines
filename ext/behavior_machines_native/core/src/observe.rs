use crate::Status;

/// Identifies a node within one compiled tree: its position in a depth-first,
#[cfg_attr(
    feature = "alloc",
    doc = " pre-order walk of the [`Node`](crate::Node) definition, with the root at 0."
)]
#[cfg_attr(
    not(feature = "alloc"),
    doc = " pre-order walk of the `Node` definition, with the root at 0."
)]
///
/// Children are numbered in declaration order: for `IfThenElse` and
/// `WhileDoElse` that is condition, then the branch it selects on success,
/// then the `otherwise` branch if present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(usize);

impl NodeId {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The node's pre-order index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Watches a tree run, for tracing, metrics, or visualization. Pass one to
/// [`Tree::tick_observed`](crate::Tree::tick_observed) or
/// [`Tree::halt_observed`](crate::Tree::halt_observed).
///
/// Children report before their parent. Both methods default to doing nothing,
/// and `()` is the observer [`Tree::tick`](crate::Tree::tick) uses.
pub trait Observer {
    /// `node` was ticked and returned `status`.
    fn ticked(&mut self, _node: NodeId, _status: Status) {}

    /// `node` was running and has been halted.
    fn halted(&mut self, _node: NodeId) {}
}

impl Observer for () {}

impl<O: Observer + ?Sized> Observer for &mut O {
    fn ticked(&mut self, node: NodeId, status: Status) {
        (**self).ticked(node, status);
    }

    fn halted(&mut self, node: NodeId) {
        (**self).halted(node);
    }
}

#[cfg(test)]
mod tests;
