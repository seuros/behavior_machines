use super::*;

use behavior_machines::Node;

use crate::test_support::{leaf, lower, two_leaf_sequence};

fn lowered() -> Lowered {
    lower(&two_leaf_sequence())
}

#[test]
fn a_fresh_state_has_nothing_running() {
    let lowered = lowered();
    let state = State::initial(&lowered);
    assert!(!state.is_tree_running());
    assert_eq!(state.is_running(0), Some(false));
    assert_eq!(state.is_running(99), None);
    assert_eq!(state.cursor(0), Some(0));
}

#[test]
fn the_wrong_number_of_leaf_statuses_is_rejected() {
    let lowered = lowered();
    let mut state = State::initial(&lowered);
    assert_eq!(
        lowered.evaluate(&mut state, &[Status::Success]),
        Err(TickError::LeafCount {
            expected: 2,
            given: 1
        }),
    );
}

#[test]
fn a_state_from_another_tree_is_rejected() {
    let lowered = lowered();
    let other = lower(&leaf());
    let mut state = State::initial(&other);
    assert_eq!(
        lowered.evaluate(&mut state, &[Status::Success, Status::Success]),
        Err(TickError::StateShape {
            expected: 3,
            given: 1
        }),
    );

    let reactive = lower(&Node::reactive_sequence([leaf(), leaf()]));
    for (tree, from, expected, given) in [(&lowered, &reactive, 1, 0), (&reactive, &lowered, 0, 1)]
    {
        let mut state = State::initial(from);
        assert_eq!(
            tree.evaluate(&mut state, &[Status::Failure, Status::Success]),
            Err(TickError::CursorShape { expected, given }),
        );
    }
}

#[test]
fn a_sequence_that_runs_remembers_where_it_stopped() {
    let lowered = lowered();
    let mut state = State::initial(&lowered);

    // The first child runs, so the composite runs and resumes at child 0.
    let schedule = lowered
        .evaluate(&mut state, &[Status::Running, Status::Success])
        .expect("shapes match");
    assert_eq!(schedule.root(), Some(Status::Running));
    assert_eq!(state.cursor(0), Some(0));
    assert!(state.is_tree_running());

    // It succeeds, so the walk advances into the second child in the same
    // tick, which also succeeds and finishes the composite.
    let schedule = lowered
        .evaluate(&mut state, &[Status::Success, Status::Success])
        .expect("shapes match");
    assert_eq!(schedule.root(), Some(Status::Success));
    assert_eq!(state.cursor(0), Some(0), "a finished composite restarts");
    assert!(!state.is_tree_running());
}

#[test]
fn an_unreached_node_has_no_status() {
    let lowered = lower(&two_leaf_sequence());
    let mut state = State::initial(&lowered);
    // The first child fails, so the second is never reached.
    let schedule = lowered
        .evaluate(&mut state, &[Status::Failure, Status::Success])
        .expect("shapes match");

    assert_eq!(schedule.status[1], Some(Status::Failure));
    assert_eq!(
        schedule.status[2], None,
        "the walk stopped before child two"
    );
    assert!(schedule.reached(1));
    assert!(!schedule.reached(2));
}

#[test]
fn the_invariants_hold_over_every_leaf_assignment() {
    let lowered = lowered();
    let plan = lowered.plan();
    for first in [Status::Success, Status::Failure, Status::Running] {
        for second in [Status::Success, Status::Failure, Status::Running] {
            let mut state = State::initial(&lowered);
            for _ in 0..3 {
                lowered
                    .evaluate(&mut state, &[first, second])
                    .expect("shapes match");
            }
            // Re-evaluate the circuit directly to validate the raw values.
            let mut cells = alloc::vec![0.0; plan.cell_count() as usize];
            cells[plan.leaf_status_cell(0, first) as usize] = 1.0;
            cells[plan.leaf_status_cell(1, second) as usize] = 1.0;
            let values = lowered
                .circuit()
                .evaluate(&cells)
                .expect("one cell per input");
            assert_eq!(
                lowered.validate(&values),
                Ok(()),
                "{first:?} then {second:?}"
            );
        }
    }
}
