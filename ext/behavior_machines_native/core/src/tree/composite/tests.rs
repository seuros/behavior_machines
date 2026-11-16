use core::time::Duration;

use crate::Node;
use crate::Status::{self, Failure, Running, Success};
use crate::Threshold;
use crate::test_support::Event::{Halt, Tick};
use crate::test_support::{Drive, Script, Scripted, at, leaf, parallel_any, tree};

const NOW: Duration = at(0);

#[test]
fn reactive_sequence_rechecks_its_guard_every_tick_and_halts_on_failure() {
    let mut script = Script::default();
    script.set("guard", Success);
    let mut tree = tree(Node::reactive_sequence([leaf("guard"), leaf("work")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(script.count(Tick("guard")), 2);

    script.set("guard", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);
    assert_eq!(script.count(Halt("work")), 1);
    script.assert_idle();
}

#[test]
fn reactive_sequence_halts_work_while_its_guard_runs_again() {
    let mut script = Script::default();
    script.set("guard", Success);
    let mut tree = tree(Node::reactive_sequence([leaf("guard"), leaf("work")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("guard", Running);
    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(script.count(Halt("work")), 1);

    script.set("guard", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(script.count(Tick("work")), 2);
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn reactive_fallback_lets_a_higher_priority_child_preempt() {
    for preempting in [Success, Running] {
        let mut script = Script::default();
        script.set("high", Failure);
        let mut tree = tree(Node::reactive_fallback([leaf("high"), leaf("low")]));

        assert_eq!(tree.run(&mut script, NOW), Running);
        script.set("high", preempting);
        assert_eq!(tree.run(&mut script, NOW), preempting);

        assert_eq!(script.count(Halt("low")), 1, "high -> {preempting:?}");
        tree.stop(&mut script);
        script.assert_idle();
    }
}

#[test]
fn reactive_composites_finish_within_one_tick() {
    type Build = fn(Vec<Node<Scripted>>) -> Node<Scripted>;
    let cases: [(Build, Status); 2] = [
        (Node::ReactiveSequence, Success),
        (Node::ReactiveFallback, Failure),
    ];
    for (build, advance) in cases {
        let mut script = Script::default();
        script.set("a", advance);
        script.set("b", advance);
        let mut tree = tree(build(vec![leaf("a"), leaf("b")]));

        assert_eq!(tree.run(&mut script, NOW), advance);
        assert_eq!(script.log, [Tick("a"), Tick("b")]);
    }
}

#[test]
fn any_success_parallel_succeeds_on_the_first_success_and_halts_the_rest() {
    let mut script = Script::default();
    let mut tree = tree(parallel_any([leaf("a"), leaf("b"), leaf("c")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Success);
    assert_eq!(tree.run(&mut script, NOW), Success);

    assert_eq!(script.count(Halt("a")), 1);
    assert_eq!(script.count(Halt("b")), 0);
    assert_eq!(script.count(Halt("c")), 1);
    script.assert_idle();
}

#[test]
fn any_success_parallel_fails_once_every_child_failed() {
    let mut script = Script::default();
    script.set("a", Failure);
    let mut tree = tree(parallel_any([leaf("a"), leaf("b")]));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Tick("a")), 1);
    script.assert_idle();
}

#[test]
fn parallel_decides_on_custom_thresholds() {
    // Two of three must succeed; two failures fail it.
    let two = Threshold::Count(2);
    let cases = [
        (["a", "c"], Success, Success, "b"),
        (["a", "c"], Failure, Failure, "b"),
    ];
    for (finishing, status, expected, still_running) in cases {
        let mut script = Script::default();
        let mut tree = tree(Node::parallel([leaf("a"), leaf("b"), leaf("c")], two, two));

        assert_eq!(tree.run(&mut script, NOW), Running);
        script.set(finishing[0], status);
        assert_eq!(tree.run(&mut script, NOW), Running);
        script.set(finishing[1], status);
        assert_eq!(
            tree.run(&mut script, NOW),
            expected,
            "{finishing:?} -> {status:?}"
        );

        assert_eq!(script.count(Halt(still_running)), 1);
        script.assert_idle();
    }
}

#[test]
fn parallel_fails_as_soon_as_success_is_unreachable() {
    // All must succeed and three failures are tolerated, but one failure
    // already makes success impossible.
    let mut script = Script::default();
    let mut tree = tree(Node::parallel(
        [leaf("a"), leaf("b"), leaf("c")],
        Threshold::All,
        Threshold::All,
    ));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("b", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Halt("a")), 1);
    assert_eq!(script.count(Halt("c")), 1);
    script.assert_idle();
}

#[test]
fn reactive_preemption_resets_a_nested_timeout_and_sequence() {
    type Build = fn(Vec<Node<Scripted>>) -> Node<Scripted>;
    let cases: [(Build, Status); 2] = [
        (Node::ReactiveSequence, Success),
        (Node::ReactiveFallback, Failure),
    ];
    for (build, advance) in cases {
        let mut script = Script::default();
        script.set("guard", advance);
        script.set("prefix", Success);
        let guarded =
            Node::sequence([leaf("prefix"), leaf("work")]).timeout(Duration::from_millis(10));
        let mut tree = tree(build(vec![leaf("guard"), guarded]));

        assert_eq!(tree.run(&mut script, at(0)), Running);
        assert_eq!(tree.next_deadline(), Some(at(10)));
        script.set("guard", Running);
        assert_eq!(tree.run(&mut script, at(1)), Running);
        assert_eq!(script.count(Halt("work")), 1);
        assert_eq!(tree.next_deadline(), None);

        // Restarted: a fresh deadline and a sequence that begins again.
        script.set("guard", advance);
        assert_eq!(tree.run(&mut script, at(20)), Running);
        assert_eq!(tree.next_deadline(), Some(at(30)));
        assert_eq!(script.count(Tick("prefix")), 2);
        tree.stop(&mut script);
        script.assert_idle();
    }
}
