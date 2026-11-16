//! Conditional control nodes.

use core::time::Duration;

use super::slot::{self, Children, Run};
use crate::observe::Observer;
use crate::{Leaf, Status};

/// The branch an `IfThenElse` committed to when its condition finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Selected {
    Then,
    Otherwise,
}

pub(super) fn if_then_else<L, C, O>(
    mut children: Children<'_>,
    selected: &mut Option<Selected>,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let (Some((condition, condition_at)), Some((then, then_at))) =
        (children.next(), children.next())
    else {
        debug_assert!(false, "a conditional has a condition and a branch");
        return Status::Failure;
    };
    let branch = match *selected {
        Some(branch) => branch,
        None => match slot::tick(condition, condition_at, run, now) {
            Status::Running => return Status::Running,
            Status::Success => Selected::Then,
            Status::Failure => Selected::Otherwise,
        },
    };
    let status = match (branch, children.next()) {
        (Selected::Then, _) => slot::tick(then, then_at, run, now),
        (Selected::Otherwise, Some((otherwise, at))) => slot::tick(otherwise, at, run, now),
        (Selected::Otherwise, None) => Status::Failure,
    };
    *selected = match status {
        Status::Running => Some(branch),
        Status::Success | Status::Failure => None,
    };
    status
}

/// The condition is ticked every tick. While it runs, the branches are
/// paused: neither ticks nor is halted. Otherwise the branch it does not
/// select is halted before the selected one ticks.
pub(super) fn while_do_else<L, C, O>(
    mut children: Children<'_>,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let (Some((condition, condition_at)), Some((body, body_at))) =
        (children.next(), children.next())
    else {
        debug_assert!(false, "a conditional has a condition and a body");
        return Status::Failure;
    };
    match (
        slot::tick(condition, condition_at, run, now),
        children.next(),
    ) {
        (Status::Running, _) => Status::Running,
        (Status::Success, otherwise) => {
            if let Some((otherwise, at)) = otherwise {
                slot::halt(otherwise, at, run);
            }
            slot::tick(body, body_at, run, now)
        }
        (Status::Failure, Some((otherwise, at))) => {
            slot::halt(body, body_at, run);
            slot::tick(otherwise, at, run, now)
        }
        (Status::Failure, None) => {
            slot::halt(body, body_at, run);
            Status::Failure
        }
    }
}

#[cfg(test)]
mod tests;
