use super::*;

use alloc::string::ToString;
use alloc::vec::Vec;
use core::time::Duration;

use behavior_machines::{Limit, Threshold};

use crate::test_support::{leaf, lower, two_leaf_sequence};

#[test]
fn node_indices_follow_the_engines_pre_order() {
    // Fallback(0) [ Sequence(1) [ leaf(2), leaf(3) ], leaf(4) ]
    let tree = Node::fallback([Node::sequence([leaf(), leaf()]), leaf()]);
    let lowered = lower(&tree);
    let plan = lowered.plan();

    assert_eq!(plan.node_count(), 5);
    assert_eq!(plan.leaf_slot(2), Some(0));
    assert_eq!(plan.leaf_slot(3), Some(1));
    assert_eq!(plan.leaf_slot(4), Some(2));
    assert_eq!(
        plan.cursor_node(0),
        Some(0),
        "the outer composite takes the first cell"
    );
    assert_eq!(plan.cursor_node(1), Some(1));
}

#[test]
fn the_report_order_is_post_order_not_reversed_pre_order() {
    let tree = Node::sequence([Node::sequence([leaf(), leaf()]), leaf()]);
    let lowered = lower(&tree);
    // Pre-order is [0, 1, 2, 3, 4]; post-order visits the first subtree first.
    assert_eq!(lowered.plan().report_order(), &[2, 3, 1, 4, 0]);
}

#[test]
fn a_reactive_composite_needs_no_cursor() {
    let memory = lower(&two_leaf_sequence());
    let reactive = lower(&Node::reactive_sequence([leaf(), leaf()]));

    assert_eq!(memory.plan().cursor_count(), 1);
    assert_eq!(reactive.plan().cursor_count(), 0);
    assert!(
        reactive.circuit().ops().len() < memory.circuit().ops().len(),
        "pinning the cursor to zero must fold operations away: reactive {}, memory {}",
        reactive.circuit().ops().len(),
        memory.circuit().ops().len(),
    );
}

#[test]
fn decorators_cost_no_operations() {
    let bare = lower(&leaf());
    for wrapped in [
        leaf().inverter(),
        leaf().force_success(),
        leaf().force_failure(),
        leaf().inverter().inverter(),
    ] {
        let lowered = lower(&wrapped);
        assert_eq!(
            lowered.circuit().ops().len(),
            bare.circuit().ops().len(),
            "a status rewrite is wire relabeling, not arithmetic",
        );
    }
}

fn composite(width: usize, reactive: bool) -> Lowered {
    let children: Vec<Node<()>> = (0..width).map(|_| leaf()).collect();
    let tree = if reactive {
        Node::reactive_sequence(children)
    } else {
        Node::sequence(children)
    };
    lower(&tree)
}

#[test]
fn depth_does_not_grow_with_width() {
    // A prefix test reads a sum of bits, and a sum costs no operations, so
    // adding children widens a layer instead of adding one.
    for reactive in [true, false] {
        let saturated = composite(4, reactive).circuit().depth();
        for width in [8, 16, 32, 64] {
            assert_eq!(
                composite(width, reactive).circuit().depth(),
                saturated,
                "width {width}, reactive {reactive}",
            );
        }
    }

    assert_eq!(composite(4, true).circuit().depth(), 4);
    assert_eq!(composite(4, false).circuit().depth(), 6);
}

#[test]
fn operations_grow_linearly_in_width() {
    // The marginal cost of a child, measured between two widths so the
    // composite's fixed overhead cancels.
    for (reactive, expected) in [(true, 7), (false, 10)] {
        let wide = composite(32, reactive).circuit().ops().len();
        let narrow = composite(16, reactive).circuit().ops().len();
        assert_eq!(
            (wide - narrow) / 16,
            expected,
            "operations per child, reactive {reactive}",
        );
    }
}

#[test]
fn depth_grows_with_nesting() {
    let mut tree = leaf();
    let mut last = lower(&tree).circuit().depth();
    for level in 1..=4 {
        tree = Node::reactive_sequence([leaf(), tree]);
        let depth = lower(&tree).circuit().depth();
        assert!(
            depth > last,
            "nesting level {level} must add layers: {last} then {depth}"
        );
        last = depth;
    }
}

#[test]
fn an_empty_composite_is_rejected() {
    let empty: Node<()> = Node::sequence([]);
    assert_eq!(
        Lowered::new(&empty).unwrap_err(),
        LowerError::EmptyComposite
    );
}

#[test]
fn unsupported_nodes_are_named() {
    type Case = (&'static str, fn() -> Node<()>);
    const CASES: &[Case] = &[
        ("Sleep", || Node::sleep(Duration::from_secs(1))),
        ("Parallel", || {
            Node::parallel([leaf()], Threshold::All, Threshold::All)
        }),
        ("IfThenElse", || Node::if_then(leaf(), leaf())),
        ("WhileDoElse", || Node::while_do(leaf(), leaf())),
        ("Timeout", || leaf().timeout(Duration::from_secs(1))),
        ("Repeat", || leaf().repeat(Limit::Times(2))),
        ("RetryUntilSuccessful", || {
            leaf().retry_until_successful(Limit::Unlimited)
        }),
        ("KeepRunningUntilFailure", || {
            leaf().keep_running_until_failure()
        }),
    ];
    for (name, build) in CASES {
        assert_eq!(
            Lowered::new(&build()).unwrap_err(),
            LowerError::Unsupported(name),
            "lowering {name}",
        );
        // The message names the node, so a caller can act on it.
        assert!(
            LowerError::Unsupported(name).to_string().contains(name),
            "the error message must name {name}",
        );
    }
}

#[test]
fn an_unsupported_node_is_found_below_a_supported_one() {
    let tree = Node::sequence([leaf(), Node::sleep(Duration::from_secs(1))]);
    assert_eq!(
        Lowered::new(&tree).unwrap_err(),
        LowerError::Unsupported("Sleep")
    );
}
