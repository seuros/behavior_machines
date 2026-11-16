//! Single-child decorators and timed nodes.

use core::time::Duration;

use super::slot::{self, Run, Slot};
use crate::observe::Observer;
use crate::{Leaf, Status};

/// A status rewrite applied to the child's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Map {
    Inverter,
    /// `ForceSuccess` and `ForceFailure`.
    Force(Status),
}

impl Map {
    pub(super) const fn apply(self, status: Status) -> Status {
        match (self, status) {
            (Self::Inverter | Self::Force(_), Status::Running) => Status::Running,
            (Self::Inverter, Status::Success) => Status::Failure,
            (Self::Inverter, Status::Failure) => Status::Success,
            (Self::Force(forced), Status::Success | Status::Failure) => forced,
        }
    }
}

/// The deadline is fixed when the node starts. The child only ticks strictly
/// before it; at or after it the child is halted and the timeout fails.
pub(super) fn timeout<L, C, O>(
    child: &mut [Slot],
    at: usize,
    limit: Duration,
    deadline: &mut Option<Duration>,
    starting: bool,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    if starting {
        *deadline = now.checked_add(limit);
    }
    if reached(*deadline, now) {
        slot::halt(child, at, run);
        return Status::Failure;
    }
    slot::tick(child, at, run, now)
}

pub(super) fn sleep(
    duration: Duration,
    deadline: &mut Option<Duration>,
    starting: bool,
    now: Duration,
) -> Status {
    if starting {
        *deadline = now.checked_add(duration);
    }
    if reached(*deadline, now) {
        Status::Success
    } else {
        Status::Running
    }
}

/// `None` is a deadline past the end of the clock: it never fires.
fn reached(deadline: Option<Duration>, now: Duration) -> bool {
    deadline.is_some_and(|deadline| now >= deadline)
}

/// A looping decorator's settings and the state it carries between ticks.
pub(super) struct Looping<'a> {
    pub again_on: Status,
    pub limit: Option<u64>,
    pub runs: &'a mut u64,
    pub due: &'a mut Option<Duration>,
}

/// `Repeat`, `RetryUntilSuccessful`, and `KeepRunningUntilFailure`. A child
/// run that ends with `again_on` counts toward `limit`; reaching it finishes
/// with that status, otherwise the next run starts on the next tick, which
/// `due` marks as already due. The other terminal status finishes immediately.
pub(super) fn looped<L, C, O>(
    child: &mut [Slot],
    at: usize,
    looping: Looping<'_>,
    run: &mut Run<'_, L, C, O>,
    now: Duration,
) -> Status
where
    L: Leaf<C>,
    C: ?Sized,
    O: Observer + ?Sized,
{
    let Looping {
        again_on,
        limit,
        runs,
        due,
    } = looping;
    *due = None;
    if limit == Some(0) {
        return again_on;
    }
    match slot::tick(child, at, run, now) {
        Status::Running => Status::Running,
        // Unlimited loops never finish this way, so they do not count runs.
        status if status == again_on => match limit {
            Some(limit) if *runs + 1 >= limit => {
                *runs = 0;
                status
            }
            Some(_) => {
                *runs += 1;
                again(due, now)
            }
            None => again(due, now),
        },
        decisive => {
            *runs = 0;
            decisive
        }
    }
}

/// A run finished and the next waits for a tick: due at once.
const fn again(due: &mut Option<Duration>, now: Duration) -> Status {
    *due = Some(now);
    Status::Running
}

#[cfg(test)]
mod tests;
