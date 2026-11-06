//! Fixtures shared by the sibling `tests.rs` modules.
//!
//! The integration tests in `tests/` are separate crates and cannot reach
//! these; they share `tests/support/fixtures.rs` instead.

use behavior_machines::Node;

use crate::lower::Lowered;

/// A leaf with no payload, since the lowering never reads one.
pub const fn leaf() -> Node<()> {
    Node::leaf(())
}

/// The smallest tree with state to carry: one cursor and two leaves.
pub fn two_leaf_sequence() -> Node<()> {
    Node::sequence([leaf(), leaf()])
}

/// Lowers a tree that is known to be supported.
pub fn lower(tree: &Node<()>) -> Lowered {
    match Lowered::new(tree) {
        Ok(lowered) => lowered,
        Err(error) => panic!("every node in this fixture is supported: {error}"),
    }
}
