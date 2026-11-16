//! Ruby bindings for `behavior-machines`, the gem's required native engine.
//!
//! Node definitions are `BehaviorMachines::Node` `Data` values, validated
//! through the core when they are built (`nodes::validate`, the private
//! `native_node` hook). `BehaviorMachines::Tree.new(root)` walks them again,
//! reading every node by its exact class and its fields from the struct, so a
//! forged value raises before it reaches the core; the walk yields the core's
//! pre-order steps. Ticks call back into Ruby leaves and observers. Once a
//! leaf or observer raises, no further Ruby runs for that tick, every leaf
//! still active is halted without observer reports, and the error propagates.
mod classes;
mod engine;
mod init;
mod leaf;
mod names;
mod nodes;
mod tree;
