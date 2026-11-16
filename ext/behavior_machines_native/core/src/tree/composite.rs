//! Composites over a list of children.

use core::time::Duration;

use super::slot::{self, Children, Run, Slot, children, halt_all};
use crate::observe::Observer;
use crate::{Leaf, Status};

/// Sequence and fallback with memory. The cursor survives only while a child
/// runs, so a finished composite restarts from its first child.
pub(super) fn ordered<L, C, O>(
    children: Children<'_>,
    cursor: &mut usize,
    advance_on: Status,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    for (index, (child, at)) in children.enumerate().skip(*cursor) {
        match slot::tick(child, at, run, now) {
            Status::Running => return Status::Running,
            status if status == advance_on => *cursor = index + 1,
            decisive => {
                *cursor = 0;
                return decisive;
            }
        }
    }
    *cursor = 0;
    advance_on
}

/// Reactive sequence and fallback: re-walk from the first child. The child
/// that stops the walk keeps running or decides the result, and every later
/// child that was still running is halted.
pub(super) fn reactive<L, C, O>(
    mut children: Children<'_>,
    advance_on: Status,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    while let Some((child, at)) = children.next() {
        let status = slot::tick(child, at, run, now);
        if status != advance_on {
            halt_all(children, run);
            return status;
        }
    }
    advance_on
}

/// Decides once `success` children succeeded, or once `failure` failed or
/// success became unreachable. Finished children keep their result in their
/// own `outcome` and are not ticked again within an activation; children
/// decide in order, so the leftmost wins ties.
pub(super) fn parallel<L, C, O>(
    below: &mut [Slot],
    at: usize,
    success: usize,
    failure: usize,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let (mut total, mut succeeded, mut failed) = (0, 0, 0);
    for (child, _) in children(below, at) {
        total += 1;
        match child.first().and_then(|head| head.outcome) {
            Some(Status::Success) => succeeded += 1,
            Some(Status::Failure) => failed += 1,
            Some(Status::Running) | None => {}
        }
    }

    let mut decided = None;
    for (child, child_at) in children(below, at) {
        if child.first().is_some_and(|head| head.outcome.is_some()) {
            continue;
        }
        let status = slot::tick(child, child_at, run, now);
        match status {
            Status::Running => continue,
            Status::Success => succeeded += 1,
            Status::Failure => failed += 1,
        }
        if let Some(head) = child.first_mut() {
            head.outcome = Some(status);
        }
        // Each child is counted once, either as already finished above or
        // as it finishes here, so `failed <= total` and the subtraction
        // below cannot wrap.
        if succeeded >= success {
            decided = Some(Status::Success);
        } else if failed >= failure || total - failed < success {
            decided = Some(Status::Failure);
        }
        if decided.is_some() {
            break;
        }
    }

    let Some(status) = decided else {
        return Status::Running;
    };
    reset(below, at, run);
    status
}

/// Halts every running child and forgets every finished child's result.
pub(super) fn reset<L, C, O>(below: &mut [Slot], at: usize, run: &mut Run<'_, L, C, O>)
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    for (child, child_at) in children(below, at) {
        slot::halt(child, child_at, run);
        if let Some(head) = child.first_mut() {
            head.outcome = None;
        }
    }
}

#[cfg(test)]
mod tests;
