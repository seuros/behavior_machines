use core::time::Duration;

use crate::Node;
use crate::Status::{Failure, Running, Success};
use crate::test_support::Event::{Halt, Tick};
use crate::test_support::{Drive, Script, Scripted, TestTree, at, leaf, tree};

const NOW: Duration = at(0);

fn if_tree() -> TestTree<Scripted> {
    tree(Node::if_then_else(
        leaf("cond"),
        leaf("then"),
        leaf("otherwise"),
    ))
}

#[test]
fn if_runs_the_branch_its_condition_chose_in_the_same_tick() {
    for (condition, chosen, skipped) in [
        (Success, "then", "otherwise"),
        (Failure, "otherwise", "then"),
    ] {
        let mut script = Script::default();
        script.set("cond", condition);
        script.set(chosen, Success);
        let mut tree = if_tree();

        assert_eq!(tree.run(&mut script, NOW), Success, "cond -> {condition:?}");
        assert_eq!(script.log, [Tick("cond"), Tick(chosen)]);
        assert_eq!(script.count(Tick(skipped)), 0);
    }
}

#[test]
fn if_waits_for_its_condition_and_then_stops_checking_it() {
    let mut script = Script::default();
    let mut tree = if_tree();

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("cond", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("cond", Failure);
    assert_eq!(tree.run(&mut script, NOW), Running);

    assert_eq!(script.count(Tick("cond")), 2);
    assert_eq!(script.count(Tick("then")), 2);
    assert_eq!(script.count(Tick("otherwise")), 0);
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn if_halts_whichever_part_is_running_and_restarts_at_the_condition() {
    let mut script = Script::default();
    let mut tree = if_tree();

    assert_eq!(tree.run(&mut script, NOW), Running);
    tree.stop(&mut script);
    assert_eq!(script.count(Halt("cond")), 1);

    script.set("cond", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    tree.stop(&mut script);
    assert_eq!(script.count(Halt("then")), 1);

    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(script.count(Tick("cond")), 3);
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn if_then_without_otherwise_fails_with_its_condition() {
    let mut script = Script::default();
    script.set("cond", Failure);
    let mut tree = tree(Node::if_then(leaf("cond"), leaf("then")));

    assert_eq!(tree.run(&mut script, NOW), Failure);
    assert_eq!(script.log, [Tick("cond")]);

    // The next activation evaluates the condition again.
    script.set("cond", Success);
    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(script.count(Tick("then")), 1);
    tree.stop(&mut script);
    script.assert_idle();
}

fn while_tree() -> TestTree<Scripted> {
    tree(Node::while_do_else(
        leaf("cond"),
        leaf("body"),
        leaf("otherwise"),
    ))
}

#[test]
fn while_do_else_rechecks_its_condition_every_tick() {
    let mut script = Script::default();
    script.set("cond", Success);
    let mut tree = while_tree();

    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(tree.run(&mut script, NOW), Running);
    assert_eq!(
        script.log,
        [Tick("cond"), Tick("body"), Tick("cond"), Tick("body")]
    );

    script.set("body", Success);
    assert_eq!(tree.run(&mut script, NOW), Success);
    script.assert_idle();
}

#[test]
fn while_do_else_halts_the_running_branch_before_switching() {
    for (from, to, flip) in [
        ("body", "otherwise", Failure),
        ("otherwise", "body", Success),
    ] {
        let mut script = Script::default();
        script.set("cond", if flip == Failure { Success } else { Failure });
        let mut tree = while_tree();

        assert_eq!(tree.run(&mut script, NOW), Running);
        script.set("cond", flip);
        assert_eq!(tree.run(&mut script, NOW), Running);

        let halted = script.position(Halt(from)).unwrap();
        let started = script.position(Tick(to)).unwrap();
        assert!(halted < started, "{from} -> {to}: {:?}", script.log);
        tree.stop(&mut script);
        script.assert_idle();
    }
}

#[test]
fn while_do_else_pauses_its_branches_while_the_condition_runs() {
    let mut script = Script::default();
    script.set("cond", Success);
    let body = leaf("body").timeout(Duration::from_millis(100));
    let mut tree = tree(Node::while_do_else(leaf("cond"), body, leaf("otherwise")));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.next_deadline(), Some(at(100)));

    script.set("cond", Running);
    assert_eq!(tree.run(&mut script, at(10)), Running);
    assert_eq!(script.count(Halt("body")), 0);
    assert_eq!(script.count(Tick("body")), 1);
    // A paused branch's timer cannot act, so it is not a wake-up deadline.
    assert_eq!(tree.next_deadline(), None);

    // The same activation resumes, deadline intact.
    script.set("cond", Success);
    assert_eq!(tree.run(&mut script, at(20)), Running);
    assert_eq!(script.count(Tick("body")), 2);
    assert_eq!(tree.next_deadline(), Some(at(100)));
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn while_do_without_otherwise_fails_when_its_condition_fails() {
    let mut script = Script::default();
    script.set("cond", Success);
    let mut tree = tree(Node::while_do(leaf("cond"), leaf("body")));

    assert_eq!(tree.run(&mut script, NOW), Running);
    script.set("cond", Failure);
    assert_eq!(tree.run(&mut script, NOW), Failure);

    assert_eq!(script.count(Halt("body")), 1);
    script.assert_idle();
}
