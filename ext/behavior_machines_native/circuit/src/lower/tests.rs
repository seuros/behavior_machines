use super::*;

use alloc::string::ToString;
use alloc::vec::Vec;
use core::time::Duration;

use behavior_machines::{Composite, Limit, Node, Otherwise, Threshold};

use crate::{Op, Unsupported};

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
        // Past 64 the block sums are one more layer, up to 64 × 64.
        for width in [65, 2048] {
            assert_eq!(
                composite(width, reactive).circuit().depth(),
                saturated + 1,
                "width {width}, reactive {reactive}",
            );
        }
    }

    assert_eq!(composite(4, true).circuit().depth(), 3);
    assert_eq!(composite(4, false).circuit().depth(), 6);
}

#[test]
fn a_wide_composite_stores_terms_near_linear_in_width() {
    // Inline prefix sums stored about 4 terms per earlier child per child
    // with memory: 8.4M at width 2048.
    for reactive in [true, false] {
        let terms: usize = composite(2048, reactive)
            .circuit()
            .ops()
            .iter()
            .map(|op| match op {
                Op::Affine(form) => form.terms().len(),
                Op::Reglu { left, gate } => left.terms().len() + gate.terms().len(),
            })
            .sum();
        assert!(
            terms <= 2048 * 256,
            "width 2048, reactive {reactive}: {terms} terms",
        );
    }
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
fn an_empty_composite_is_refused() {
    let empty: Node<()> = Node::sequence([]);
    assert_eq!(
        Lowered::new(&empty).unwrap_err(),
        Refusal::EmptyComposite(Composite::Sequence)
    );
}

#[test]
fn steps_that_are_not_exactly_one_tree_are_refused() {
    let cases: [(&[Step<()>], Refusal); 4] = [
        (&[], Refusal::Incomplete),
        (&[Step::Fallback(2), Step::Leaf(())], Refusal::Incomplete),
        (&[Step::Leaf(()), Step::Leaf(())], Refusal::Trailing),
        (
            &[Step::Inverter, Step::Leaf(()), Step::AlwaysSuccess],
            Refusal::Trailing,
        ),
    ];
    for (steps, refusal) in cases {
        assert_eq!(
            Lowered::new(steps.iter().copied()).unwrap_err(),
            refusal,
            "{steps:?}"
        );
    }
}

#[test]
fn unsupported_nodes_are_named() {
    type Case = (Unsupported, fn() -> Node<()>);
    const CASES: &[Case] = &[
        (Unsupported::Sleep, || Node::sleep(Duration::from_secs(1))),
        (Unsupported::Parallel, || {
            Node::parallel([leaf()], Threshold::All, Threshold::All)
        }),
        (Unsupported::IfThenElse, || Node::if_then(leaf(), leaf())),
        (Unsupported::WhileDoElse, || Node::while_do(leaf(), leaf())),
        (Unsupported::Timeout, || {
            leaf().timeout(Duration::from_secs(1))
        }),
        (Unsupported::Repeat, || leaf().repeat(Limit::Times(2))),
        (Unsupported::RetryUntilSuccessful, || {
            leaf().retry_until_successful(Limit::Unlimited)
        }),
        (Unsupported::KeepRunningUntilFailure, || {
            leaf().keep_running_until_failure()
        }),
    ];
    for (node, build) in CASES {
        assert_eq!(
            Lowered::new(&build()).unwrap_err(),
            Refusal::Unsupported(*node),
            "lowering {node}",
        );
        // The message names the node, so a caller can act on it.
        let name = node.to_string();
        assert!(
            Refusal::Unsupported(*node).to_string().contains(&name),
            "the message must name {name}",
        );
    }
    let otherwise = [
        Step::IfThenElse(Otherwise::Present),
        Step::Leaf(()),
        Step::Leaf(()),
        Step::Leaf(()),
    ];
    assert_eq!(
        Lowered::new(otherwise).unwrap_err(),
        Refusal::Unsupported(Unsupported::IfThenElse)
    );
}

#[test]
fn an_unsupported_node_is_found_below_a_supported_one() {
    let tree = Node::sequence([leaf(), Node::sleep(Duration::from_secs(1))]);
    assert_eq!(
        Lowered::new(&tree).unwrap_err(),
        Refusal::Unsupported(Unsupported::Sleep)
    );
}

// Bug: `Lowered::new` sized a `Vec` from the declared child count and
// panicked with `capacity overflow` instead of refusing.
#[test]
fn a_declared_child_count_past_memory_is_incomplete_not_a_panic() {
    for step in [
        Step::<()>::Sequence(usize::MAX),
        Step::Fallback(usize::MAX),
        Step::ReactiveSequence(usize::MAX),
        Step::ReactiveFallback(usize::MAX),
    ] {
        assert_eq!(Lowered::new([step]), Err(Refusal::Incomplete));
    }
}

#[test]
fn a_definition_past_the_node_limit_is_full() {
    let steps = || {
        [
            Step::<()>::Sequence(2),
            Step::AlwaysSuccess,
            Step::AlwaysFailure,
        ]
    };
    assert!(flatten(steps(), 3).is_ok());
    assert_eq!(flatten(steps(), 2).err(), Some(Refusal::Full));
}

#[test]
fn a_deep_decorator_chain_lowers_without_recursion() {
    // Bug: `post_order` recursed once per nesting level, so a chain well under
    // `MAX_NODES` overflowed the stack in `Lowered::new`.
    const DEPTH: usize = 200_000;
    let decorator = |level: usize| match level % 5 {
        2 => Step::<()>::ForceSuccess,
        4 => Step::ForceFailure,
        _ => Step::Inverter,
    };
    // Steps rather than a `Node`, whose recursive `Drop` would overflow first.
    let steps = (0..DEPTH).map(decorator).chain([Step::Leaf(())]);
    let lowered = match Lowered::new(steps) {
        Ok(lowered) => lowered,
        Err(error) => panic!("a decorator chain is supported: {error}"),
    };

    let order: Vec<usize> = (0..=DEPTH).rev().collect();
    assert_eq!(lowered.plan().report_order(), order.as_slice());

    // The tree walk, done as a loop: the leaf's status rewritten by each
    // decorator from the innermost out.
    for leaf_status in [Status::Success, Status::Failure, Status::Running] {
        let expected = (0..DEPTH).rev().fold(leaf_status, |status, level| {
            match (decorator(level), status) {
                (_, Status::Running) => Status::Running,
                (Step::Inverter, Status::Success) => Status::Failure,
                (Step::Inverter | Step::ForceSuccess, _) => Status::Success,
                _ => Status::Failure,
            }
        });
        let mut state = crate::State::initial(&lowered);
        let schedule = match lowered.evaluate(&mut state, &[leaf_status]) {
            Ok(schedule) => schedule,
            Err(error) => panic!("the state came from this tree: {error}"),
        };
        assert_eq!(schedule.root(), Some(expected), "leaf {leaf_status:?}");
        assert!((0..=DEPTH).all(|node| schedule.reached(node)));
    }
}
