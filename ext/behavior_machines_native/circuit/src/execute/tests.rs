use super::*;

struct Guard;

impl Condition<usize> for Guard {
    fn evaluate(&self, _ctx: &usize, _now: Time) -> Status {
        Status::Success
    }
}

struct Work;

impl Action<usize> for Work {
    fn start(&self, active: &mut usize, _now: Time) -> Status {
        *active += 1;
        Status::Running
    }

    fn poll(&self, _active: &mut usize, _now: Time) -> Status {
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
    assert!(executor.tick(&mut active, Time::ZERO).is_ok());
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
    assert!(matches!(
        executor.tick(&mut active, Time::ZERO),
        Err(TickError::StateShape { .. })
    ));
    assert_eq!(active, 0);
    assert!(!executor.is_running());
    executor.halt(&mut active);
    assert_eq!(active, 0, "recovery already halted the action");

    assert!(executor.tick(&mut active, Time::ZERO).is_ok());
    assert_eq!(active, 1, "recovery permits a fresh activation");
    executor.halt(&mut active);
}
