#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod builder;
mod execute;
mod lower;
mod plan;
mod status;
mod tick;
mod wire;

#[cfg(test)]
mod test_support;

pub use builder::{Bit, Builder, Count};
pub use execute::{Action, Condition, ExecutionLeaf, Executor};
pub use lower::{InvariantError, LowerError, Lowered, Outputs};
pub use plan::Plan;
pub use status::StatusBits;
pub use tick::{Schedule, State, TickError};
pub use wire::{Affine, Circuit, EvalError, Op, Wire};
