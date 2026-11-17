use alloc::boxed::Box;
use core::time::Duration;

use matryona_machines::testing::{refuses, unchanged};

use super::*;
use crate::test_support::Event::{Halt as Halted, Tick};
use crate::test_support::{Drive, Script, Scripted, TestTree, at, leaf, parallel_all, race, tree};
use crate::{Composite, Node, Otherwise, Threshold};

use Status::{Failure, Running, Success};

const NOW: Duration = Duration::ZERO;

#[test]
fn refuses_empty_composites_at_any_depth() {
    let cases: [(Node<Scripted>, Composite); 6] = [
        (Node::sequence([]), Composite::Sequence),
        (Node::reactive_sequence([]), Composite::ReactiveSequence),
        (Node::fallback([]), Composite::Fallback),
        (Node::reactive_fallback([]), Composite::ReactiveFallback),
        (parallel_all([]), Composite::Parallel),
        (
            Node::sequence([leaf("a"), Node::fallback([]).inverter()]),
            Composite::Fallback,
        ),
    ];
    for (root, kind) in cases {
        assert_eq!(
            TestTree::new(root).err(),
            Some(Refusal::EmptyComposite(kind)),
            "{kind}"
        );
    }
    assert_eq!(
        Refusal::EmptyComposite(Composite::ReactiveFallback).to_string(),
        "ReactiveFallback needs at least one child"
    );
}

#[test]
fn refuses_parallel_thresholds_outside_the_child_count() {
    let two = || [leaf("a"), leaf("b")];
    let cases = [
        (Threshold::Count(0), Threshold::All, 0),
        (Threshold::All, Threshold::Count(3), 3),
    ];
    for (success, failure, threshold) in cases {
        let error = TestTree::new(Node::parallel(two(), success, failure)).err();
        assert_eq!(
            error,
            Some(Refusal::ParallelThreshold {
                threshold,
                children: 2
            })
        );
    }
    assert_eq!(
        Refusal::ParallelThreshold {
            threshold: 0,
            children: 2
        }
        .to_string(),
        "Parallel threshold 0 must be between 1 and 2"
    );
}

#[test]
fn refuses_steps_that_are_not_exactly_one_tree() {
    let a = || Step::Leaf(Scripted("a"));
    let cases: [(&[Step<Scripted>], Refusal); 5] = [
        (&[], Refusal::Incomplete),
        (&[Step::Sequence(2), a()], Refusal::Incomplete),
        (
            &[Step::IfThenElse(Otherwise::Present), a(), a()],
            Refusal::Incomplete,
        ),
        (&[a(), a()], Refusal::Trailing),
        (
            &[Step::Inverter, a(), Step::AlwaysSuccess],
            Refusal::Trailing,
        ),
    ];
    for (steps, refusal) in cases {
        assert_eq!(
            TestTree::new(steps.iter().cloned()).err(),
            Some(refusal),
            "{steps:?}"
        );
    }
}

#[test]
fn refuses_a_definition_past_its_capacity_and_fits_one_exactly_at_it() {
    let steps = || [Step::Sequence(2), Step::Leaf("a"), Step::Leaf("b")];
    assert_eq!(Tree::<&str, 2>::new(steps()).err(), Some(Refusal::Full));
    let exact = Tree::<&str, 3>::new(steps()).unwrap();
    assert_eq!((exact.len(), exact.is_empty()), (3, false));
    assert_eq!(
        Refusal::Full.to_string(),
        "the definition exceeds the tree's capacity"
    );
}

#[test]
fn steps_and_the_nested_builder_compile_to_the_same_tree() {
    let nested = Node::if_then_else(
        leaf("c"),
        Node::sequence([leaf("a"), leaf("b").inverter()]),
        Node::sleep(at(5)),
    );
    let steps = [
        Step::IfThenElse(Otherwise::Present),
        Step::Leaf(Scripted("c")),
        Step::Sequence(2),
        Step::Leaf(Scripted("a")),
        Step::Inverter,
        Step::Leaf(Scripted("b")),
        Step::Sleep(at(5)),
    ];
    assert_eq!(
        nested
            .iter()
            .map(|step| step.as_ref().children())
            .sum::<usize>(),
        6
    );
    assert_eq!(tree(nested), TestTree::new(steps).unwrap());
}

#[test]
fn a_regressing_tick_is_refused_and_runs_nothing() {
    let mut state = (
        tree(Node::sequence([leaf("a"), leaf("b")])),
        Script::default(),
    );
    assert_eq!(state.0.run(&mut state.1, at(5)), Running);

    refuses(&mut state, Refusal::TimeRegression, |(tree, script)| {
        tree.tick(script, at(4))
    });
    assert_eq!(state.1.log, [Tick("a")]);

    // The refusal recorded nothing: the honoured instant is still 5 ms.
    assert_eq!(state.0.tick(&mut state.1, at(5)), Ok(Running));
    assert_eq!(
        Refusal::TimeRegression.to_string(),
        "now is earlier than the latest honoured tick"
    );
}

#[test]
fn halting_answers_whether_anything_was_running() {
    let mut script = Script::default();
    let mut tree = tree(Node::sequence([leaf("a"), leaf("b")]));

    let mut state = (tree.clone(), script.clone());
    let idle = unchanged(&mut state, |(tree, script)| tree.halt(script));
    assert_eq!(idle, Halt::AlreadyIdle);

    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(tree.halt(&mut script), Halt::Halted);
    assert_eq!(script.log, [Tick("a"), Halted("a")]);

    let mut state = (tree, script);
    let again = unchanged(&mut state, |(tree, script)| tree.halt(script));
    assert_eq!(again, Halt::AlreadyIdle);
}

#[test]
fn ordered_composites_advance_within_one_tick() {
    type Build = fn(Vec<Node<Scripted>>) -> Node<Scripted>;
    type Case = (
        Build,
        &'static [(&'static str, Status)],
        Status,
        &'static [&'static str],
    );
    let cases: [Case; 4] = [
        (
            Node::Sequence,
            &[("a", Success), ("b", Success), ("c", Success)],
            Success,
            &["a", "b", "c"],
        ),
        (
            Node::Sequence,
            &[("a", Success), ("b", Failure)],
            Failure,
            &["a", "b"],
        ),
        (
            Node::Fallback,
            &[("a", Failure), ("b", Failure), ("c", Failure)],
            Failure,
            &["a", "b", "c"],
        ),
        (
            Node::Fallback,
            &[("a", Failure), ("b", Success)],
            Success,
            &["a", "b"],
        ),
    ];
    for (build, outcomes, expected, ticked) in cases {
        let mut script = Script::default();
        for &(name, status) in outcomes {
            script.set(name, status);
        }
        let mut tree = tree(build(vec![leaf("a"), leaf("b"), leaf("c")]));

        assert_eq!(
            tree.run(&mut script, NOW),
            expected,
            "outcomes: {outcomes:?}"
        );
        let expected_log: Vec<_> = ticked.iter().map(|name| Tick(name)).collect();
        assert_eq!(script.log, expected_log, "outcomes: {outcomes:?}");
        script.assert_idle();
    }
}

#[test]
fn sequence_resumes_the_running_child() {
    let mut script = Script::default();
    let mut tree = tree(Node::sequence([leaf("a"), leaf("b")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("a", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Success);
    assert_eq!(tree.run(&mut script, NOW), Success);

    assert_eq!(script.log, [Tick("a"), Tick("a"), Tick("b"), Tick("b")]);
    script.assert_idle();
}

#[test]
fn parallel_all_succeeds_without_reticking_finished_children() {
    let mut script = Script::default();
    let mut tree = tree(parallel_all([leaf("a"), leaf("b")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("a", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Success);
    assert_eq!(tree.run(&mut script, NOW), Success);

    assert_eq!(script.count(Tick("a")), 2);
    assert_eq!(script.count(Tick("b")), 3);
    script.assert_idle();
}

#[test]
fn parallel_failure_halts_running_siblings_on_both_sides() {
    let mut script = Script::default();
    let mut tree = tree(parallel_all([leaf("a"), leaf("b"), leaf("c")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Halted("a")), 1);
    assert_eq!(script.count(Halted("b")), 0);
    assert_eq!(script.count(Halted("c")), 1);
    assert!(!tree.is_running());
    script.assert_idle();
}

#[test]
fn immediate_decisions_never_start_or_halt_later_children() {
    let cases: [(Node<Scripted>, Status); 2] = [
        (parallel_all([leaf("first"), leaf("later")]), Failure),
        (race([leaf("first"), leaf("later")]), Failure),
    ];
    for (root, expected) in cases {
        let mut script = Script::default();
        script.set("first", expected);
        let mut tree = tree(root);

        assert_eq!(tree.run(&mut script, NOW), expected);
        assert_eq!(script.log, [Tick("first")]);
        script.assert_idle();
    }
}

#[test]
fn race_winner_decides_and_halts_losers_on_both_sides() {
    for winner_status in [Success, Failure] {
        let mut script = Script::default();
        let mut tree = tree(race([leaf("a"), leaf("b"), leaf("winner"), leaf("d")]));

        assert_eq!(tree.run(&mut script, NOW), Running);
        script.set("winner", winner_status);
        assert_eq!(tree.run(&mut script, NOW), winner_status);

        for loser in ["a", "b", "d"] {
            assert_eq!(
                script.count(Halted(loser)),
                1,
                "{loser} with {winner_status:?}"
            );
        }
        assert_eq!(script.count(Halted("winner")), 0);
        script.assert_idle();
    }
}

#[test]
fn race_tie_goes_to_the_leftmost_child() {
    let mut script = Script::default();
    let mut tree = tree(race([leaf("left"), leaf("right")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("left", Failure);
    script.set("right", Success);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Halted("right")), 1);
    script.assert_idle();
}

#[test]
fn abandoned_nested_branch_is_halted_before_the_next_child_starts() {
    let mut script = Script::default();
    let mut tree = tree(Node::sequence([
        race([leaf("winner"), leaf("loser")]),
        leaf("next"),
    ]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("winner", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);

    let halted = script.position(Halted("loser")).unwrap();
    let started = script.position(Tick("next")).unwrap();
    assert!(halted < started, "log: {:?}", script.log);
    assert_eq!(script.count(Halted("loser")), 1);
}

#[test]
fn finished_branches_are_not_halted_again() {
    let mut script = Script::default();
    let mut tree = tree(parallel_all([
        race([leaf("winner"), leaf("loser")]),
        leaf("other"),
    ]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("winner", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("other", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Halted("loser")), 1);
    assert_eq!(script.count(Halted("winner")), 0);
    script.assert_idle();
}

#[test]
fn tree_halt_halts_every_active_leaf_once() {
    let mut script = Script::default();
    script.set("ready", Success);
    let mut tree = tree(Node::fallback([
        Node::sequence([leaf("ready"), parallel_all([leaf("a"), leaf("b")])]),
        leaf("unreached"),
    ]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    tree.stop(&mut script);
    tree.stop(&mut script);

    assert_eq!(script.count(Halted("a")), 1);
    assert_eq!(script.count(Halted("b")), 1);
    assert_eq!(script.count(Tick("unreached")), 0);
    assert!(!tree.is_running());
    script.assert_idle();
}

#[test]
fn tree_restarts_after_finishing_or_halting() {
    let mut script = Script::default();
    script.set("a", Success);
    let mut tree = tree(Node::sequence([leaf("a"), leaf("b")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    tree.stop(&mut script);
    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Success);
    assert_eq!(tree.run(&mut script, NOW), Success);
    assert_eq!(tree.run(&mut script, NOW), Success);

    assert_eq!(script.count(Tick("a")), 3);
    assert_eq!(script.count(Tick("b")), 4);
    assert_eq!(script.count(Halted("b")), 1);
    script.assert_idle();
}

#[test]
fn boxed_leaves_forward_tick_and_halt() {
    let mut script = Script::default();
    let boxed: Box<dyn Leaf<Script>> = Box::new(Scripted("boxed"));
    let mut tree = TestTree::new(race([Node::leaf(boxed)])).unwrap();

    assert_eq!(tree.run(&mut script, NOW), Running);
    tree.stop(&mut script);

    assert_eq!(script.log, [Tick("boxed"), Halted("boxed")]);
    script.assert_idle();
}

mod lifecycle;
mod storage;
