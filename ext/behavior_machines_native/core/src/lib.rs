#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

mod leaf;
#[cfg(feature = "state-machines")]
pub mod machine;
#[cfg(any(feature = "alloc", test))]
mod node;
mod observe;
mod refusal;
mod status;
mod step;
mod tree;

#[cfg(test)]
mod test_support;

pub use leaf::Leaf;
#[cfg(any(feature = "alloc", test))]
pub use node::{Node, Steps, StepsRef};
pub use observe::{NodeId, Observer};
pub use refusal::Refusal;
pub use status::{Halt, Status};
pub use step::{Composite, Limit, Otherwise, Step, Threshold};
pub use tree::Tree;
