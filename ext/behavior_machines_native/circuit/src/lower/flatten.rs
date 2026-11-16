//! Pass 1 of the lowering: reading the steps into flat nodes.

use alloc::vec::Vec;

use behavior_machines::{Composite, Status, Step};

use super::{Flat, MapKind};
use crate::refusal::{Refusal, Unsupported};

/// A node still waiting for children while the steps are read.
struct Open {
    node: usize,
    remaining: usize,
}

/// The flat nodes, the node index of each leaf slot, and of each cursor cell.
pub(super) type Flattened = (Vec<Flat>, Vec<usize>, Vec<usize>);

/// Pass 1: the steps are already the engine's pre-order, so a step's position
/// is its [`NodeId`](behavior_machines::NodeId). Each step is a child of the
/// innermost node still waiting for children. More than `limit` steps is
/// [`Refusal::Full`], refused before the step past it is stored.
pub(super) fn flatten<L>(
    steps: impl IntoIterator<Item = Step<L>>,
    limit: usize,
) -> Result<Flattened, Refusal> {
    let mut flat = Vec::new();
    let mut leaves = Vec::new();
    let mut cursors = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        if index > 0 && open.is_empty() {
            return Err(Refusal::Trailing);
        }
        if index >= limit {
            return Err(Refusal::Full);
        }
        if let Some(parent) = open.last_mut() {
            // An open node awaits at least one child: it is popped below the
            // moment its count reaches zero, so this never wraps.
            parent.remaining -= 1;
            if let Some(node) = flat.get_mut(parent.node) {
                adopt(node, index);
            }
        }
        let (kind, children) = flat_of(&step, index, &mut leaves, &mut cursors)?;
        flat.push(kind);
        if children > 0 {
            open.push(Open {
                node: index,
                remaining: children,
            });
        }
        while open.pop_if(|top| top.remaining == 0).is_some() {}
    }
    if flat.is_empty() || !open.is_empty() {
        return Err(Refusal::Incomplete);
    }
    Ok((flat, leaves, cursors))
}

/// Records `child` as the next child of `parent`.
fn adopt(parent: &mut Flat, child: usize) {
    match parent {
        Flat::Ordered { children, .. } => children.push(child),
        Flat::Map { child: slot, .. } => *slot = child,
        Flat::Leaf { .. } | Flat::Constant(_) => {
            debug_assert!(false, "only composites and decorators wait for children");
        }
    }
}

/// One step as a flat node, and how many children follow it. A memory
/// composite claims its cursor cell here, before its children, so cells
/// follow the same pre-order as node ids.
fn flat_of<L>(
    step: &Step<L>,
    index: usize,
    leaves: &mut Vec<usize>,
    cursors: &mut Vec<usize>,
) -> Result<(Flat, usize), Refusal> {
    let mut ordered = |children: usize, kind, advance_on, reactive: bool| {
        if children == 0 {
            return Err(Refusal::EmptyComposite(kind));
        }
        let cursor = (!reactive).then(|| {
            let cell = cursors.len();
            cursors.push(index);
            cell
        });
        // The declared count is not trusted to size anything: it may be far
        // more than the steps that follow, which `flatten` then refuses.
        let node = Flat::Ordered {
            children: Vec::new(),
            advance_on,
            cursor,
        };
        Ok((node, children))
    };
    let map = |map| Ok((Flat::Map { child: 0, map }, 1));
    let unsupported = |node| Err(Refusal::Unsupported(node));
    match step {
        Step::Leaf(_) => {
            let slot = leaves.len();
            leaves.push(index);
            Ok((Flat::Leaf { slot }, 0))
        }
        Step::AlwaysSuccess => Ok((Flat::Constant(Status::Success), 0)),
        Step::AlwaysFailure => Ok((Flat::Constant(Status::Failure), 0)),
        Step::Sequence(n) => ordered(*n, Composite::Sequence, Status::Success, false),
        Step::ReactiveSequence(n) => {
            ordered(*n, Composite::ReactiveSequence, Status::Success, true)
        }
        Step::Fallback(n) => ordered(*n, Composite::Fallback, Status::Failure, false),
        Step::ReactiveFallback(n) => {
            ordered(*n, Composite::ReactiveFallback, Status::Failure, true)
        }
        Step::Inverter => map(MapKind::Invert),
        Step::ForceSuccess => map(MapKind::Force(Status::Success)),
        Step::ForceFailure => map(MapKind::Force(Status::Failure)),
        Step::Sleep(_) => unsupported(Unsupported::Sleep),
        Step::Parallel { .. } => unsupported(Unsupported::Parallel),
        Step::IfThenElse(_) => unsupported(Unsupported::IfThenElse),
        Step::WhileDoElse(_) => unsupported(Unsupported::WhileDoElse),
        Step::Timeout(_) => unsupported(Unsupported::Timeout),
        Step::Repeat(_) => unsupported(Unsupported::Repeat),
        Step::RetryUntilSuccessful(_) => unsupported(Unsupported::RetryUntilSuccessful),
        Step::KeepRunningUntilFailure => unsupported(Unsupported::KeepRunningUntilFailure),
    }
}
