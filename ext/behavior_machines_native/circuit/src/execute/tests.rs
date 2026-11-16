use core::assert_matches;
use core::time::Duration;

use behavior_machines::Node;

use super::*;

#[derive(Clone, Debug, PartialEq)]
struct Guard;

impl Condition<usize> for Guard {
    fn evaluate(&self, _ctx: &usize, _now: Duration) -> Status {
        Status::Success
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Work;

impl Action<usize> for Work {
    fn start(&self, active: &mut usize, _now: Duration) -> Status {
        *active += 1;
        Status::Running
    }

    fn poll(&self, _active: &mut usize, _now: Duration) -> Status {
        Status::Running
    }

    fn halt(&self, active: &mut usize) {
        *active -= 1;
    }
}

#[test]
fn an_internal_evaluation_error_halts_actions_and_resets_state() {
    let Ok(mut executor) = Executor::<Guard, Work>::new(Node::leaf(ExecutionLeaf::Action(Work)))
    else {
        panic!("the tree is supported")
    };
    let mut active = 0;
    assert!(executor.tick(&mut active, Duration::ZERO).is_ok());
    assert_eq!(active, 1);

    // Public callers cannot replace state. Inject a broken internal shape to
    // exercise error recovery after an activation has already started.
    let Ok(other) = Lowered::new(&Node::<()>::sequence([
        Node::AlwaysSuccess,
        Node::AlwaysFailure,
    ])) else {
        panic!("the tree is supported")
    };
    executor.state = State::initial(&other);
    assert_matches!(
        executor.tick(&mut active, Duration::ZERO),
        Err(Refusal::StateShape { .. })
    );
    assert_eq!(active, 0);
    assert!(!executor.is_running());
    assert_eq!(executor.halt(&mut active), Halt::AlreadyIdle);
    assert_eq!(active, 0, "recovery already halted the action");

    assert!(executor.tick(&mut active, Duration::ZERO).is_ok());
    assert_eq!(active, 1, "recovery permits a fresh activation");
    assert_eq!(executor.halt(&mut active), Halt::Halted);
    assert_eq!(active, 0);
}

#[test]
fn a_regressing_tick_is_refused_and_runs_nothing() {
    let Ok(executor) = Executor::<Guard, Work>::new(Node::leaf(ExecutionLeaf::Action(Work))) else {
        panic!("the tree is supported")
    };
    let mut state = (executor, 0);
    assert!(state.0.tick(&mut state.1, Duration::from_nanos(5)).is_ok());

    matryona_machines::testing::refuses(
        &mut state,
        Refusal::TimeRegression,
        |(executor, active)| executor.tick(active, Duration::from_nanos(4)),
    );
    assert!(state.0.tick(&mut state.1, Duration::from_nanos(5)).is_ok());
    assert_eq!(state.0.halt(&mut state.1), Halt::Halted);
}
