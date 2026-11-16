use state_machines::state_machine;

use core::time::Duration;

use super::*;
use crate::test_support::{Drive, TestTree};
use crate::{Halt, Node};

use Status::{Failure, Running, Success};

// REAL: a generated dynamic machine, shaped like a task lifecycle.
state_machine! {
    name: Job,
    dynamic: true,
    initial: Pending,
    states: [Pending, Started, Completed, Failed, Cancelled],
    events {
        start { transition: { from: Pending, to: Started } }
        complete { transition: { from: Started, to: Completed } }
        fail { transition: { from: [Pending, Started], to: Failed } }
        cancel { transition: { from: [Pending, Started], to: Cancelled } }
    }
}

type Machine = DynamicJob<()>;
/// Events applied to a fresh job before a case runs.
type Events = &'static [fn() -> JobEvent];

struct World {
    job: Machine,
    deadline_passed: bool,
}

impl World {
    fn new() -> Self {
        Self {
            job: Job::new(()).into_dynamic(),
            deadline_passed: false,
        }
    }

    fn drive(&mut self, events: &[fn() -> JobEvent]) {
        for event in events {
            self.job.handle(event()).unwrap();
        }
    }
}

fn job(world: &mut World) -> &mut Machine {
    &mut world.job
}

const fn finished(state: JobState) -> Status {
    match state {
        JobState::Pending | JobState::Started => Running,
        JobState::Completed => Success,
        JobState::Failed | JobState::Cancelled => Failure,
    }
}

/// A mixed leaf type: adapters next to an application leaf, by delegation.
#[derive(Debug)]
enum Step {
    Start(Fire<World, Machine>),
    Await(WaitFor<World, Machine>),
    // MOCKED: stands in for a timer or competing branch.
    Deadline,
}

impl Leaf<World> for Step {
    fn tick(&self, world: &mut World, now: Duration) -> Status {
        match self {
            Self::Start(leaf) => leaf.tick(world, now),
            Self::Await(leaf) => leaf.tick(world, now),
            Self::Deadline if world.deadline_passed => Success,
            Self::Deadline => Running,
        }
    }

    fn halt(&self, world: &mut World) {
        match self {
            Self::Start(leaf) => leaf.halt(world),
            Self::Await(leaf) => leaf.halt(world),
            Self::Deadline => {}
        }
    }
}

/// Starts the job, then waits for it; abandoning the wait cancels the job.
fn run_job() -> Node<Step> {
    Node::sequence([
        Node::leaf(Step::Start(Fire::new(job, || JobEvent::Start))),
        Node::leaf(Step::Await(
            WaitFor::new(job, finished).on_halt(|| JobEvent::Cancel),
        )),
    ])
}

/// Ticks `leaf` once against a fresh job that went through each case's events.
fn assert_cases(leaf: &impl Leaf<World>, cases: &[(Events, Status)]) {
    for (events, expected) in cases {
        let mut world = World::new();
        world.drive(events);
        assert_eq!(
            leaf.tick(&mut world, Duration::ZERO),
            *expected,
            "after {} events",
            events.len()
        );
    }
}

#[test]
fn in_state_reports_whether_the_state_is_accepted() {
    let leaf = InState::new(job, |state| state == JobState::Started);
    let cases: [(Events, Status); 3] = [
        (&[], Failure),
        (&[|| JobEvent::Start], Success),
        (&[|| JobEvent::Start, || JobEvent::Complete], Failure),
    ];
    assert_cases(&leaf, &cases);
}

#[test]
fn fire_succeeds_only_when_the_machine_accepts_the_event() {
    let mut world = World::new();
    let start = Fire::new(job, || JobEvent::Start);

    assert_eq!(start.tick(&mut world, Duration::ZERO), Success);
    assert_eq!(world.job.current_state(), JobState::Started);
    assert_eq!(start.tick(&mut world, Duration::ZERO), Failure);
    assert_eq!(world.job.current_state(), JobState::Started);
}

#[test]
fn wait_for_classifies_the_current_state() {
    let leaf = WaitFor::new(job, finished);
    let cases: [(Events, Status); 4] = [
        (&[], Running),
        (&[|| JobEvent::Start], Running),
        (&[|| JobEvent::Start, || JobEvent::Complete], Success),
        (&[|| JobEvent::Fail], Failure),
    ];
    assert_cases(&leaf, &cases);
}

#[test]
fn tree_drives_the_job_to_completion() {
    let mut world = World::new();
    let mut tree = TestTree::new(run_job()).unwrap();

    assert_eq!(tree.run(&mut world, Duration::ZERO), Running);
    assert_eq!(world.job.current_state(), JobState::Started);

    world.drive(&[|| JobEvent::Complete]);
    assert_eq!(tree.run(&mut world, Duration::ZERO), Success);
}

#[test]
fn abandoned_wait_cancels_the_job_once() {
    let mut world = World::new();
    let mut tree = TestTree::new(crate::test_support::race([
        run_job(),
        Node::leaf(Step::Deadline),
    ]))
    .unwrap();

    assert_eq!(tree.run(&mut world, Duration::ZERO), Running);
    world.deadline_passed = true;
    assert_eq!(tree.run(&mut world, Duration::ZERO), Success);
    assert_eq!(world.job.current_state(), JobState::Cancelled);

    // Halting again must not dispatch a second cancel.
    assert_eq!(tree.halt(&mut world), Halt::AlreadyIdle);
    assert_eq!(world.job.current_state(), JobState::Cancelled);
}

#[test]
fn rejected_halt_event_leaves_a_finished_job_alone() {
    let mut world = World::new();
    let mut tree = TestTree::new(run_job()).unwrap();

    assert_eq!(tree.run(&mut world, Duration::ZERO), Running);
    // The job finishes between ticks, then the tree is abandoned.
    world.drive(&[|| JobEvent::Complete]);
    assert_eq!(tree.halt(&mut world), Halt::Halted);

    assert_eq!(world.job.current_state(), JobState::Completed);
}

#[test]
fn adapters_are_copy_and_debug_without_bounds() {
    let wait = WaitFor::new(job, finished);
    let copy = wait;
    assert_eq!(format!("{copy:?}"), "WaitFor { .. }");
    assert_eq!(format!("{wait:?}"), "WaitFor { .. }");
}
