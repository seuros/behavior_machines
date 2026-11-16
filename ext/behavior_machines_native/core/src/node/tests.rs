use core::time::Duration;

use crate::{Limit, Node, Otherwise, Step, Threshold};

/// Every kind of child slot, with leaves in declaration order.
fn shape<L>([check, then, other, left, right, guard, body, retry]: [L; 8]) -> Node<L> {
    Node::sequence([
        Node::if_then_else(Node::leaf(check), Node::leaf(then), Node::leaf(other)),
        Node::parallel(
            [Node::leaf(left), Node::leaf(right).inverter()],
            Threshold::All,
            Threshold::Count(1),
        ),
        Node::while_do(
            Node::leaf(guard),
            Node::leaf(body).timeout(Duration::from_millis(5)),
        ),
        Node::reactive_fallback([
            Node::AlwaysFailure,
            Node::leaf(retry).repeat(Limit::Times(2)),
        ]),
    ])
}

#[test]
fn steps_walk_the_definition_in_pre_order_owned_or_borrowed() {
    let names = ["c", "t", "e", "p1", "p2", "w", "b", "r"];
    let ms5 = Duration::from_millis(5);
    let expected = [
        Step::Sequence(4),
        Step::IfThenElse(Otherwise::Present),
        Step::Leaf("c"),
        Step::Leaf("t"),
        Step::Leaf("e"),
        Step::Parallel {
            children: 2,
            success: Threshold::All,
            failure: Threshold::Count(1),
        },
        Step::Leaf("p1"),
        Step::Inverter,
        Step::Leaf("p2"),
        Step::WhileDoElse(Otherwise::Absent),
        Step::Leaf("w"),
        Step::Timeout(ms5),
        Step::Leaf("b"),
        Step::ReactiveFallback(2),
        Step::AlwaysFailure,
        Step::Repeat(Limit::Times(2)),
        Step::Leaf("r"),
    ];
    let node = shape(names);

    let borrowed: Vec<Step<&str>> = (&node)
        .into_iter()
        .map(|step| step.map(|name| *name))
        .collect();
    assert_eq!(borrowed, expected);
    assert_eq!(node.into_iter().collect::<Vec<_>>(), expected);
}

#[test]
fn try_map_runs_only_for_a_leaf() {
    assert_eq!(
        Step::Leaf(2).try_map(|n| Ok::<_, ()>(n * 3)),
        Ok(Step::Leaf(6))
    );
    assert_eq!(
        Step::Leaf(2).try_map(|_| Err::<u8, _>("refused")),
        Err("refused")
    );
    assert_eq!(
        Step::<u8>::Sequence(3).try_map(|_| Err::<u8, _>("never asked")),
        Ok(Step::Sequence(3))
    );
}
