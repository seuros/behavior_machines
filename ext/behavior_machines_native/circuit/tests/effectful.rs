//! Compare effects, activation lifecycles and complete schedules with the
//! regular engine, not just root statuses.

use core::time::Duration;

use behavior_machines::{Leaf, Node, NodeId, Observer, Status, Time, Tree};
use behavior_machines_circuit::{Action, Condition, ExecutionLeaf, Executor, LowerError, Schedule};

#[derive(Clone, Debug)]
struct Scripted(usize);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Effect {
    Start(usize, Time),
    Poll(usize, Time),
    Halt(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Context {
    statuses: Vec<Status>,
    active: Vec<bool>,
    effects: Vec<Effect>,
}

impl Context {
    fn new(leaves: usize) -> Self {
        Self {
            statuses: vec![Status::Running; leaves],
            active: vec![false; leaves],
            effects: Vec::new(),
        }
    }
}

impl Condition<Context> for Scripted {
    fn evaluate(&self, ctx: &Context, _now: Time) -> Status {
        ctx.statuses[self.0]
    }
}

impl Action<Context> for Scripted {
    fn start(&self, ctx: &mut Context, now: Time) -> Status {
        assert!(!ctx.active[self.0], "an active action must be polled");
        ctx.effects.push(Effect::Start(self.0, now));
        ctx.active[self.0] = ctx.statuses[self.0] == Status::Running;
        ctx.statuses[self.0]
    }

    fn poll(&self, ctx: &mut Context, now: Time) -> Status {
        assert!(ctx.active[self.0], "an inactive action must be started");
        ctx.effects.push(Effect::Poll(self.0, now));
        ctx.active[self.0] = ctx.statuses[self.0] == Status::Running;
        ctx.statuses[self.0]
    }

    fn halt(&self, ctx: &mut Context) {
        assert!(ctx.active[self.0], "halt must happen exactly once");
        ctx.effects.push(Effect::Halt(self.0));
        ctx.active[self.0] = false;
    }
}

/// Gives the regular engine the same actions, with the activation bookkeeping
/// its Leaf API leaves to the host.
struct Reference(ExecutionLeaf<Scripted, Scripted>);

impl Leaf<Context> for Reference {
    fn tick(&self, ctx: &mut Context, now: Time) -> Status {
        match &self.0 {
            ExecutionLeaf::Condition(condition) => condition.evaluate(ctx, now),
            ExecutionLeaf::Action(action) if ctx.active[action.0] => action.poll(ctx, now),
            ExecutionLeaf::Action(action) => action.start(ctx, now),
        }
    }

    fn halt(&self, ctx: &mut Context) {
        if let ExecutionLeaf::Action(action) = &self.0 {
            action.halt(ctx);
        }
    }
}

#[derive(Default)]
struct Record {
    ticked: Vec<(usize, Status)>,
    halted: Vec<usize>,
}

impl Observer for Record {
    fn ticked(&mut self, node: NodeId, status: Status) {
        self.ticked.push((node.index(), status));
    }

    fn halted(&mut self, node: NodeId) {
        self.halted.push(node.index());
    }
}

fn executor(tree: Node<ExecutionLeaf<Scripted, Scripted>>) -> Executor<Scripted, Scripted> {
    let Ok(executor) = Executor::new(tree) else {
        panic!("the fixture is supported")
    };
    executor
}

fn tick(executor: &mut Executor<Scripted, Scripted>, ctx: &mut Context, now: Time) -> Schedule {
    let Ok(schedule) = executor.tick(ctx, now) else {
        panic!("the executor's internal shapes match")
    };
    schedule
}

const fn action(slot: usize) -> Node<ExecutionLeaf<Scripted, Scripted>> {
    Node::leaf(ExecutionLeaf::Action(Scripted(slot)))
}

const fn condition(slot: usize) -> Node<ExecutionLeaf<Scripted, Scripted>> {
    Node::leaf(ExecutionLeaf::Condition(Scripted(slot)))
}

#[test]
fn sequence_advances_in_the_same_tick_without_replaying_completed_actions() {
    let mut executor = executor(Node::sequence([action(0), action(1), action(2)]));
    let mut ctx = Context::new(3);
    ctx.statuses = vec![Status::Success, Status::Running, Status::Success];
    let first = Time::from_nanos(17);
    assert_eq!(
        tick(&mut executor, &mut ctx, first).root(),
        Some(Status::Running)
    );
    assert_eq!(
        ctx.effects,
        [Effect::Start(0, first), Effect::Start(1, first)]
    );

    ctx.statuses[1] = Status::Success;
    let second = Time::from_nanos(29);
    assert_eq!(
        tick(&mut executor, &mut ctx, second).root(),
        Some(Status::Success)
    );
    assert_eq!(
        ctx.effects,
        [
            Effect::Start(0, first),
            Effect::Start(1, first),
            Effect::Poll(1, second),
            Effect::Start(2, second),
        ]
    );
    assert!(!executor.is_running());

    executor.halt(&mut ctx);
    assert!(
        !ctx.effects
            .iter()
            .any(|effect| matches!(effect, Effect::Halt(_)))
    );
    ctx.effects.clear();
    tick(&mut executor, &mut ctx, second);
    assert_eq!(
        ctx.effects,
        [
            Effect::Start(0, second),
            Effect::Start(1, second),
            Effect::Start(2, second),
        ],
        "a completed root starts a fresh activation on the next tick",
    );
}

#[test]
fn provisional_running_does_not_preempt_an_action_before_its_guard_is_known() {
    let mut executor = executor(Node::reactive_sequence([condition(0), action(1)]));
    let mut ctx = Context::new(2);
    ctx.statuses[0] = Status::Success;
    tick(&mut executor, &mut ctx, Time::ZERO);
    tick(&mut executor, &mut ctx, Time::ZERO);
    assert_eq!(
        ctx.effects,
        [Effect::Start(1, Time::ZERO), Effect::Poll(1, Time::ZERO)],
        "an unresolved guard is not a reason to cancel a running action",
    );

    ctx.statuses[0] = Status::Failure;
    tick(&mut executor, &mut ctx, Time::ZERO);
    executor.halt(&mut ctx);
    executor.halt(&mut ctx);
    assert_eq!(ctx.effects.last(), Some(&Effect::Halt(1)));
    assert_eq!(ctx.effects.len(), 3);
}

#[test]
fn halt_and_restart_reset_memory_cursors_and_activation_state() {
    let mut executor = executor(Node::sequence([action(0), action(1)]));
    let mut ctx = Context::new(2);
    ctx.statuses[0] = Status::Success;
    tick(&mut executor, &mut ctx, Time::ZERO);
    executor.halt(&mut ctx);
    executor.halt(&mut ctx);
    assert!(!executor.is_running());
    tick(&mut executor, &mut ctx, Time::ZERO);
    assert_eq!(
        ctx.effects,
        [
            Effect::Start(0, Time::ZERO),
            Effect::Start(1, Time::ZERO),
            Effect::Halt(1),
            Effect::Start(0, Time::ZERO),
            Effect::Start(1, Time::ZERO),
        ],
    );
    executor.halt(&mut ctx);
}

#[test]
fn unsupported_nodes_and_empty_composites_are_rejected_during_construction() {
    for (tree, name) in [
        (action(0).timeout(Duration::from_secs(1)), "Timeout"),
        (Node::sleep(Duration::ZERO), "Sleep"),
        (
            Node::if_then_else(condition(0), action(1), action(2)),
            "IfThenElse",
        ),
    ] {
        assert!(
            matches!(Executor::new(tree), Err(LowerError::Unsupported(found)) if found == name)
        );
    }
    let empty = Node::<ExecutionLeaf<Scripted, Scripted>>::sequence([]);
    assert!(matches!(
        Executor::new(empty),
        Err(LowerError::EmptyComposite)
    ));
}

fn compare(tree: &Node<()>, script: &[Status], ticks: usize, action_mask: usize) {
    let mut count = 0;
    let classified = tree.clone().map(|()| {
        let slot = count;
        count += 1;
        if action_mask & (1 << slot) == 0 {
            ExecutionLeaf::Condition(Scripted(slot))
        } else {
            ExecutionLeaf::Action(Scripted(slot))
        }
    });
    let Ok(mut reference) = Tree::new(classified.clone().map(Reference)) else {
        panic!("the fixture is valid")
    };
    let mut executor = executor(classified);
    let mut expected = Context::new(count);
    let mut actual = expected.clone();
    for step in 0..ticks {
        let now = Time::from_nanos(step as u64);
        expected.statuses = script[step * count..(step + 1) * count].to_vec();
        actual.statuses.clone_from(&expected.statuses);
        let mut record = Record::default();
        let root = reference.tick_observed(&mut expected, now, &mut record);
        let schedule = tick(&mut executor, &mut actual, now);
        let message = || format!("{tree:?}, script={script:?}, mask={action_mask}, step={step}");
        assert_eq!(schedule.root(), Some(root), "{}", message());
        assert_eq!(
            schedule
                .ticked(executor.lowered().plan())
                .collect::<Vec<_>>(),
            record.ticked,
            "{}",
            message(),
        );
        assert_eq!(
            schedule
                .halts(executor.lowered().plan())
                .collect::<Vec<_>>(),
            record.halted,
            "{}",
            message(),
        );
        assert_eq!(actual, expected, "effect ordering: {}", message());
        assert_eq!(
            executor.is_running(),
            reference.is_running(),
            "{}",
            message()
        );
    }
    reference.halt(&mut expected);
    executor.halt(&mut actual);
    executor.halt(&mut actual);
    assert_eq!(actual, expected, "shutdown: {tree:?}, script={script:?}");
}

fn exhaustive(tree: &Node<()>, ticks: usize) {
    let mut leaves = 0;
    tree.clone().map(|()| leaves += 1);
    let slots = leaves * ticks;
    let mut script = vec![Status::Success; slots];
    let statuses = [Status::Success, Status::Failure, Status::Running];
    let Ok(exponent) = u32::try_from(slots) else {
        panic!("the fixture is small")
    };
    for encoded in 0..3usize.pow(exponent) {
        let mut remaining = encoded;
        for status in &mut script {
            *status = statuses[remaining % 3];
            remaining /= 3;
        }
        for mask in 0..1 << leaves {
            compare(tree, &script, ticks, mask);
        }
    }
}

const fn leaf() -> Node<()> {
    Node::leaf(())
}

#[test]
fn all_classifications_and_status_scripts_match_the_engine() {
    for tree in [
        Node::sequence([leaf(), leaf(), leaf()]),
        Node::fallback([leaf(), leaf(), leaf()]),
        Node::reactive_sequence([leaf(), leaf(), leaf()]),
        Node::reactive_fallback([leaf(), leaf(), leaf()]),
        Node::sequence([
            leaf().inverter(),
            leaf().force_success(),
            leaf().force_failure(),
        ]),
        Node::reactive_fallback([Node::sequence([leaf(), leaf()]), leaf().inverter()]),
        Node::sequence([
            Node::reactive_sequence([leaf(), leaf()]).force_success(),
            leaf(),
        ]),
        Node::reactive_sequence([Node::fallback([leaf(), leaf()]).force_failure(), leaf()]),
        Node::sequence([Node::AlwaysSuccess, leaf(), Node::AlwaysFailure]),
        Node::AlwaysSuccess,
        Node::AlwaysFailure,
    ] {
        exhaustive(&tree, 2);
    }
}

#[derive(Default)]
struct Shared {
    flag: bool,
    running: bool,
    events: Vec<&'static str>,
}

#[derive(Clone)]
struct Flag;

impl Condition<Shared> for Flag {
    fn evaluate(&self, ctx: &Shared, _now: Time) -> Status {
        if ctx.flag {
            Status::Success
        } else {
            Status::Failure
        }
    }
}

#[derive(Clone)]
enum Change {
    SetFlag,
    Wait,
    AfterCancellation,
}

impl Action<Shared> for Change {
    fn start(&self, ctx: &mut Shared, _now: Time) -> Status {
        match self {
            Self::SetFlag => {
                ctx.flag = true;
                ctx.events.push("set");
                Status::Success
            }
            Self::Wait => {
                assert!(!ctx.running);
                ctx.running = true;
                ctx.events.push("start");
                Status::Running
            }
            Self::AfterCancellation => {
                assert!(!ctx.running, "earlier subtree cancellation must have run");
                ctx.events.push("after");
                Status::Success
            }
        }
    }

    fn poll(&self, ctx: &mut Shared, _now: Time) -> Status {
        assert!(ctx.running);
        ctx.events.push("poll");
        Status::Running
    }

    fn halt(&self, ctx: &mut Shared) {
        assert!(ctx.running);
        ctx.running = false;
        ctx.events.push("halt");
    }
}

fn shared_executor(tree: Node<ExecutionLeaf<Flag, Change>>) -> Executor<Flag, Change> {
    let Ok(executor) = Executor::new(tree) else {
        panic!("the fixture is valid")
    };
    executor
}

#[test]
fn a_later_condition_observes_changes_from_an_earlier_action() {
    let mut executor = shared_executor(Node::sequence([
        Node::leaf(ExecutionLeaf::Action(Change::SetFlag)),
        Node::leaf(ExecutionLeaf::Condition(Flag)),
    ]));
    let mut ctx = Shared::default();
    let Ok(schedule) = executor.tick(&mut ctx, Time::ZERO) else {
        panic!("the internal shapes match")
    };
    assert_eq!(schedule.root(), Some(Status::Success));
    assert_eq!(ctx.events, ["set"]);
}

#[test]
fn abandoned_subtree_halts_before_a_later_sibling_starts() {
    let mut executor = shared_executor(Node::sequence([
        Node::reactive_sequence([
            Node::leaf(ExecutionLeaf::Condition(Flag)),
            Node::leaf(ExecutionLeaf::Action(Change::Wait)),
        ])
        .force_success(),
        Node::leaf(ExecutionLeaf::Action(Change::AfterCancellation)),
    ]));
    let mut ctx = Shared {
        flag: true,
        ..Shared::default()
    };
    assert!(executor.tick(&mut ctx, Time::ZERO).is_ok());
    ctx.flag = false;
    assert!(executor.tick(&mut ctx, Time::ZERO).is_ok());
    assert_eq!(ctx.events, ["start", "halt", "after"]);
}

#[test]
fn conditions_are_not_speculatively_evaluated() {
    struct MustNotRun;
    impl Condition<Context> for MustNotRun {
        fn evaluate(&self, _ctx: &Context, _now: Time) -> Status {
            panic!("an unreachable condition must not be evaluated");
        }
    }
    let Ok(mut executor) = Executor::new(Node::sequence([
        Node::leaf(ExecutionLeaf::Action(Scripted(0))),
        Node::leaf(ExecutionLeaf::Condition(MustNotRun)),
    ])) else {
        panic!("the fixture is valid")
    };
    let mut ctx = Context::new(1);
    assert!(executor.tick(&mut ctx, Time::ZERO).is_ok());
    ctx.statuses[0] = Status::Failure;
    assert!(executor.tick(&mut ctx, Time::ZERO).is_ok());
}
