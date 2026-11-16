//! Leaves that drive a [state-machines] dynamic machine. Requires the
//! `state-machines` feature.
//!
//! Each adapter reaches its machine through an [`Access`] function on the
//! tree context, so one context can hold many machines. A leaf enum mixes
//! adapters with its own leaves by delegating to them.
//!
//! [state-machines]: https://docs.rs/state-machines

use core::fmt;
use core::time::Duration;

use state_machines::DynamicMachine;

use crate::{Leaf, Status};

/// Borrows the machine an adapter drives out of the tree context.
pub type Access<C, M> = fn(&mut C) -> &mut M;

/// Builds an event to dispatch. Generated event enums are not `Clone`, so
/// adapters keep a constructor instead of a value.
pub type MakeEvent<M> = fn() -> <M as DynamicMachine>::Event;

/// A condition leaf: [`Status::Success`] while the machine's state satisfies
/// `accepts`, [`Status::Failure`] otherwise.
pub struct InState<C: ?Sized, M: DynamicMachine> {
    machine: Access<C, M>,
    accepts: fn(M::State) -> bool,
}

impl<C: ?Sized, M: DynamicMachine> InState<C, M> {
    /// Succeeds while `accepts` holds for the machine's state.
    pub const fn new(machine: Access<C, M>, accepts: fn(M::State) -> bool) -> Self {
        Self { machine, accepts }
    }
}

impl<C: ?Sized, M: DynamicMachine> Leaf<C> for InState<C, M> {
    fn tick(&self, ctx: &mut C, _now: Duration) -> Status {
        if (self.accepts)((self.machine)(ctx).state()) {
            Status::Success
        } else {
            Status::Failure
        }
    }
}

/// An action leaf: dispatches one event per tick.
///
/// [`Status::Success`] if the machine accepts it, [`Status::Failure`] if a
/// transition, guard, or callback rejects it. The machine's error is not kept:
/// a leaf answers with a status.
pub struct Fire<C: ?Sized, M: DynamicMachine> {
    machine: Access<C, M>,
    event: MakeEvent<M>,
}

impl<C: ?Sized, M: DynamicMachine> Fire<C, M> {
    /// Dispatches `event()` on every tick.
    pub const fn new(machine: Access<C, M>, event: MakeEvent<M>) -> Self {
        Self { machine, event }
    }
}

impl<C: ?Sized, M: DynamicMachine> Leaf<C> for Fire<C, M> {
    fn tick(&self, ctx: &mut C, _now: Duration) -> Status {
        match (self.machine)(ctx).handle((self.event)()) {
            Ok(()) => Status::Success,
            Err(_) => Status::Failure,
        }
    }
}

/// A wait leaf for a machine something else drives, such as a task or a
/// callback: each tick maps the machine's state to a status with `classify`.
///
/// If the tree abandons the leaf while it runs, the [`WaitFor::on_halt`]
/// event, if any, is dispatched.
pub struct WaitFor<C: ?Sized, M: DynamicMachine> {
    machine: Access<C, M>,
    classify: fn(M::State) -> Status,
    on_halt: Option<MakeEvent<M>>,
}

impl<C: ?Sized, M: DynamicMachine> WaitFor<C, M> {
    /// Runs while `classify` reports [`Status::Running`] for the machine's
    /// state.
    pub const fn new(machine: Access<C, M>, classify: fn(M::State) -> Status) -> Self {
        Self {
            machine,
            classify,
            on_halt: None,
        }
    }

    /// Dispatches `event()` when the tree halts this leaf, typically a
    /// `cancel` event.
    #[must_use]
    pub const fn on_halt(mut self, event: MakeEvent<M>) -> Self {
        self.on_halt = Some(event);
        self
    }
}

impl<C: ?Sized, M: DynamicMachine> Leaf<C> for WaitFor<C, M> {
    fn tick(&self, ctx: &mut C, _now: Duration) -> Status {
        (self.classify)((self.machine)(ctx).state())
    }

    fn halt(&self, ctx: &mut C) {
        if let Some(event) = self.on_halt {
            // Ignored on purpose: a rejection means the machine already left
            // the states that accept the event, such as a job that completed
            // after the last tick.
            (self.machine)(ctx).handle(event()).ok();
        }
    }
}

/// `Clone`, `Copy`, and `Debug` without the `C: Clone`/`M: Debug` bounds a
/// derive would add: the adapters only hold function pointers.
macro_rules! adapter_traits {
    ($($adapter:ident),*) => {$(
        impl<C: ?Sized, M: DynamicMachine> Clone for $adapter<C, M> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<C: ?Sized, M: DynamicMachine> Copy for $adapter<C, M> {}

        impl<C: ?Sized, M: DynamicMachine> fmt::Debug for $adapter<C, M> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.debug_struct(stringify!($adapter)).finish_non_exhaustive()
            }
        }
    )*};
}

adapter_traits!(InState, Fire, WaitFor);

#[cfg(test)]
mod tests;
