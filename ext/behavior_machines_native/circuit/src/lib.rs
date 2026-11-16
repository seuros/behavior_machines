#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

extern crate alloc;

mod builder;
mod execute;
mod lower;
mod plan;
mod refusal;
mod status;
mod tick;
mod wire;

#[cfg(test)]
mod test_support;

pub use behavior_machines::{Composite, Halt, Status, Step};
pub use builder::{Bit, Builder, Count};
pub use execute::{Action, Condition, ExecutionLeaf, Executor};
pub use lower::{Lowered, Outputs};
pub use plan::Plan;
pub use refusal::{Refusal, Unsupported};
pub use status::StatusBits;
pub use tick::{Schedule, State};
pub use wire::{Affine, Circuit, Op, Wire};
