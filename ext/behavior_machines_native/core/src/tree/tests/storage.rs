//! MOCKED: random definitions and leaf scripts, run through an inline and a
//! heap tree side by side; every answer must match.

use core::time::Duration;
use std::collections::HashSet;

use matryona_machines::testing::refuses;

use crate::{
    Halt, HeapTree, Leaf, Limit, Node, NodeId, Observer, Otherwise, Refusal, Status, Step,
    Threshold, Tree,
};

const CAPACITY: usize = 128;
const CASES: u64 = 2_000;
const TICKS: usize = 24;

/// splitmix64: a seeded, dependency-free source of draws.
struct Draws(u64);

impl Draws {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn size(&mut self, bound: usize) -> usize {
        usize::try_from(self.below(bound as u64)).unwrap()
    }

    fn status(&mut self) -> Status {
        [Status::Success, Status::Failure, Status::Running][self.size(3)]
    }

    fn duration(&mut self) -> Duration {
        Duration::from_millis(self.below(40))
    }

    fn limit(&mut self) -> Limit {
        if self.below(4) == 0 {
            Limit::Unlimited
        } else {
            Limit::Times(self.below(4))
        }
    }

    fn threshold(&mut self, children: usize) -> Threshold {
        if self.below(3) == 0 {
            Threshold::All
        } else {
            Threshold::Count(self.size(children + 2))
        }
    }
}

/// A leaf that answers whatever the plan scripted for its index this tick.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Probe(usize);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Plan {
    outcomes: Vec<Status>,
    log: Vec<(bool, usize)>,
    active: HashSet<usize>,
}

impl Leaf<Plan> for Probe {
    fn tick(&self, plan: &mut Plan, _now: Duration) -> Status {
        plan.log.push((true, self.0));
        let status = plan.outcomes[self.0];
        if status == Status::Running {
            plan.active.insert(self.0);
        } else {
            plan.active.remove(&self.0);
        }
        status
    }

    fn halt(&self, plan: &mut Plan) {
        assert!(
            plan.active.remove(&self.0),
            "halted inactive leaf {}",
            self.0
        );
        plan.log.push((false, self.0));
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Record(Vec<(NodeId, Option<Status>)>);

impl Observer for Record {
    fn ticked(&mut self, node: NodeId, status: Status) {
        self.0.push((node, Some(status)));
    }

    fn halted(&mut self, node: NodeId) {
        self.0.push((node, None));
    }
}

fn definition(draws: &mut Draws, depth: usize) -> Node<()> {
    let pick = if depth == 0 {
        draws.below(4)
    } else {
        draws.below(19)
    };
    let child = |draws: &mut Draws| definition(draws, depth.saturating_sub(1));
    let children = |draws: &mut Draws| {
        let count = draws.size(4);
        (0..count).map(|_| child(draws)).collect::<Vec<_>>()
    };
    match pick {
        0 | 1 => Node::leaf(()),
        2 => [Node::AlwaysSuccess, Node::AlwaysFailure][draws.size(2)].clone(),
        3 => Node::sleep(draws.duration()),
        4 => Node::sequence(children(draws)),
        5 => Node::reactive_sequence(children(draws)),
        6 => Node::fallback(children(draws)),
        7 => Node::reactive_fallback(children(draws)),
        8 => {
            let children = children(draws);
            let success = draws.threshold(children.len());
            let failure = draws.threshold(children.len());
            Node::parallel(children, success, failure)
        }
        9 => Node::if_then_else(child(draws), child(draws), child(draws)),
        10 => Node::if_then(child(draws), child(draws)),
        11 => Node::while_do_else(child(draws), child(draws), child(draws)),
        12 => Node::while_do(child(draws), child(draws)),
        13 => child(draws).inverter(),
        14 => [child(draws).force_success(), child(draws).force_failure()][draws.size(2)].clone(),
        15 => child(draws).timeout(draws.duration()),
        16 => child(draws).repeat(draws.limit()),
        17 => child(draws).retry_until_successful(draws.limit()),
        _ => child(draws).keep_running_until_failure(),
    }
}

fn numbered(root: Node<()>) -> (Node<Probe>, usize) {
    let mut leaves = 0;
    let root = root.map(|()| {
        leaves += 1;
        Probe(leaves - 1)
    });
    (root, leaves)
}

/// A raw step stream, often malformed, to compare the build refusals.
fn steps(draws: &mut Draws) -> Vec<Step<Probe>> {
    (0..draws.size(10))
        .map(|at| match draws.below(6) {
            0 | 1 => Step::Leaf(Probe(at)),
            2 => Step::Sequence(draws.size(3)),
            3 => Step::IfThenElse(Otherwise::Absent),
            4 => Step::Inverter,
            _ => Step::Parallel {
                children: 2,
                success: draws.threshold(2),
                failure: Threshold::All,
            },
        })
        .collect()
}

/// Both builds, which must agree; `CAPACITY` holds any generated definition.
fn built(
    steps: impl IntoIterator<Item = Step<Probe>> + Clone,
) -> Option<(Tree<Probe, CAPACITY>, HeapTree<Probe>)> {
    let inline = Tree::<Probe, CAPACITY>::new(steps.clone());
    let heap = HeapTree::new(steps);
    match (inline, heap) {
        (Ok(inline), Ok(heap)) => {
            assert_eq!(inline.len(), heap.len());
            assert!(!heap.is_empty());
            Some((inline, heap))
        }
        (inline, heap) => {
            assert_eq!(inline.err(), heap.err());
            None
        }
    }
}

#[test]
fn inline_and_heap_trees_refuse_the_same_definitions() {
    let mut draws = Draws(0x5EED);
    for _ in 0..CASES {
        built(steps(&mut draws));
    }
}

#[test]
fn inline_and_heap_trees_answer_every_tick_and_halt_alike() {
    let mut draws = Draws(0xB1A5);
    let mut compared = 0;
    for case in 0..CASES {
        let (root, leaves) = numbered(definition(&mut draws, 4));
        let Some((mut inline, mut heap)) = built(root.clone()) else {
            continue;
        };
        compared += 1;
        let (mut left, mut right) = (Plan::default(), Plan::default());
        let mut now = Duration::ZERO;
        for tick in 0..TICKS {
            let context = || format!("case {case} tick {tick}\n{root:?}");
            let outcomes: Vec<Status> = (0..leaves).map(|_| draws.status()).collect();
            left.outcomes.clone_from(&outcomes);
            right.outcomes = outcomes;
            let (mut seen_left, mut seen_right) = (Record::default(), Record::default());
            match draws.below(10) {
                0 => {
                    let answer = inline.halt_observed(&mut left, &mut seen_left);
                    let other = heap.halt_observed(&mut right, &mut seen_right);
                    assert_eq!(answer, other, "{}", context());
                }
                pick => {
                    now = if pick == 1 {
                        now.saturating_sub(Duration::from_millis(5))
                    } else {
                        now + draws.duration()
                    };
                    let answer = inline.tick_observed(&mut left, now, &mut seen_left);
                    let other = heap.tick_observed(&mut right, now, &mut seen_right);
                    assert_eq!(answer, other, "{}", context());
                }
            }
            assert_eq!(seen_left, seen_right, "{}", context());
            assert_eq!(left, right, "{}", context());
            assert_eq!(inline.is_running(), heap.is_running(), "{}", context());
            assert_eq!(
                inline.next_deadline(),
                heap.next_deadline(),
                "{}",
                context()
            );
        }
        let (halted, other) = (inline.halt(&mut left), heap.halt(&mut right));
        assert_eq!(halted, other);
        assert_eq!(left, right);
        assert!(
            left.active.is_empty(),
            "case {case} leaked {:?}",
            left.active
        );
    }
    assert!(compared > CASES / 2, "only {compared} definitions compiled");
}

#[test]
fn a_heap_tree_grows_past_any_inline_capacity() {
    let wide = Node::sequence((0..1_000).map(Probe).map(Node::leaf));
    assert_eq!(
        Tree::<Probe, CAPACITY>::new(wide.clone()).err(),
        Some(Refusal::Full)
    );
    let Ok(mut heap) = HeapTree::new(wide) else {
        panic!("a heap tree has room for any definition");
    };
    assert_eq!(heap.len(), 1_001);
    let mut plan = Plan {
        outcomes: vec![Status::Success; 1_000],
        ..Plan::default()
    };
    assert_eq!(heap.tick(&mut plan, Duration::ZERO), Ok(Status::Success));
    assert_eq!(plan.log.len(), 1_000);
}

#[test]
fn a_heap_tree_refusal_changes_nothing() {
    let root = Node::sequence([Node::leaf(Probe(0)), Node::leaf(Probe(1))]);
    let plan = Plan {
        outcomes: vec![Status::Running; 2],
        ..Plan::default()
    };
    let mut state = (HeapTree::new(root).unwrap(), plan);
    let later = Duration::from_millis(5);
    assert_eq!(state.0.tick(&mut state.1, later), Ok(Status::Running));
    refuses(&mut state, Refusal::TimeRegression, |(tree, plan)| {
        tree.tick(plan, Duration::from_millis(4))
    });
    assert_eq!(state.0.halt(&mut state.1), Halt::Halted);
}
