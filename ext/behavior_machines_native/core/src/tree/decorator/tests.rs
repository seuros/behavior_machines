use core::time::Duration;

use crate::Status::{self, Failure, Running, Success};
use crate::test_support::Event::{Halt, Tick};
use crate::test_support::{Drive, Script, Scripted, at, leaf, parallel_all, tree};
use crate::{Limit, Node};

const MS: fn(u64) -> Duration = Duration::from_millis;

#[test]
fn status_decorators_rewrite_terminal_statuses() {
    type Wrap = fn(Node<Scripted>) -> Node<Scripted>;
    let decorators: [(&str, Wrap, [Status; 2]); 3] = [
        ("Inverter", Node::inverter, [Failure, Success]),
        ("ForceSuccess", Node::force_success, [Success, Success]),
        ("ForceFailure", Node::force_failure, [Failure, Failure]),
    ];
    for (name, wrap, [on_success, on_failure]) in decorators {
        for (child, expected) in [
            (Success, on_success),
            (Failure, on_failure),
            (Running, Running),
        ] {
            let mut script = Script::default();
            script.set("a", child);
            let mut tree = tree(wrap(leaf("a")));

            assert_eq!(tree.run(&mut script, at(0)), expected, "{name}({child:?})");
            tree.stop(&mut script);
            script.assert_idle();
        }
    }
}

#[test]
fn timeout_halts_its_child_at_the_deadline() {
    let mut script = Script::default();
    let mut tree = tree(leaf("a").timeout(MS(100)));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.next_deadline(), Some(at(100)));
    assert_eq!(tree.run(&mut script, at(99)), Running);
    assert_eq!(tree.run(&mut script, at(100)), Failure);

    assert_eq!(script.log, [Tick("a"), Tick("a"), Halt("a")]);
    assert_eq!(tree.next_deadline(), None);
    script.assert_idle();
}

#[test]
fn timeout_reports_a_child_that_finishes_in_time() {
    let mut script = Script::default();
    let mut tree = tree(leaf("a").timeout(MS(100)));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    script.set("a", Success);
    assert_eq!(tree.run(&mut script, at(50)), Success);
    assert_eq!(script.count(Halt("a")), 0);

    // The next activation gets a fresh deadline.
    script.set("a", Running);
    assert_eq!(tree.run(&mut script, at(500)), Running);
    assert_eq!(tree.next_deadline(), Some(at(600)));
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn zero_timeout_fails_without_ticking_its_child() {
    let mut script = Script::default();
    let mut tree = tree(leaf("a").timeout(Duration::ZERO));

    assert_eq!(tree.run(&mut script, at(0)), Failure);
    assert_eq!(script.log, []);
}

#[test]
fn sleep_succeeds_once_its_duration_has_passed() {
    let mut script = Script::default();
    let mut tree = tree(Node::sleep(MS(50)));

    assert_eq!(tree.run(&mut script, at(10)), Running);
    assert_eq!(tree.next_deadline(), Some(at(60)));
    assert_eq!(tree.run(&mut script, at(59)), Running);
    assert_eq!(tree.run(&mut script, at(60)), Success);

    assert_eq!(tree.run(&mut script, at(100)), Running);
    assert_eq!(tree.next_deadline(), Some(at(150)));

    let mut instant = crate::test_support::tree(Node::sleep(Duration::ZERO));
    assert_eq!(instant.run(&mut script, at(0)), Success);
}

#[test]
fn next_deadline_is_the_earliest_running_timer() {
    let mut script = Script::default();
    let mut tree = tree(parallel_all([
        Node::sleep(MS(300)),
        leaf("a").timeout(MS(100)),
    ]));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.next_deadline(), Some(at(100)));
    assert_eq!(tree.run(&mut script, at(100)), Failure);
    assert_eq!(tree.next_deadline(), None);
    script.assert_idle();
}

#[test]
fn deadlines_past_the_end_of_the_clock_never_fire() {
    // Starting 1 ns before the end of the clock, a 2 ns deadline does not
    // exist; reaching `Duration::MAX` must not count as reaching it.
    let start = Duration::new(u64::MAX, 999_999_998);
    let mut script = Script::default();

    let mut sleep = tree(Node::sleep(Duration::from_nanos(2)));
    assert_eq!(sleep.run(&mut script, start), Running);
    assert_eq!(sleep.next_deadline(), None);
    assert_eq!(sleep.run(&mut script, Duration::MAX), Running);

    let mut timeout = tree(leaf("a").timeout(Duration::from_nanos(2)));
    assert_eq!(timeout.run(&mut script, start), Running);
    assert_eq!(timeout.run(&mut script, Duration::MAX), Running);
    assert_eq!(script.count(Halt("a")), 0);
    timeout.stop(&mut script);
    script.assert_idle();
}

#[test]
fn a_deadline_of_exactly_the_end_of_the_clock_fires() {
    let start = Duration::new(u64::MAX, 999_999_998);
    let mut script = Script::default();

    let mut sleep = tree(Node::sleep(Duration::from_nanos(1)));
    assert_eq!(sleep.run(&mut script, start), Running);
    assert_eq!(sleep.next_deadline(), Some(Duration::MAX));
    assert_eq!(sleep.run(&mut script, Duration::MAX), Success);

    let mut timeout = tree(leaf("a").timeout(Duration::from_nanos(1)));
    assert_eq!(timeout.run(&mut script, start), Running);
    assert_eq!(timeout.run(&mut script, Duration::MAX), Failure);
    assert_eq!(script.log, [Tick("a"), Halt("a")]);
}

#[test]
fn repeat_runs_its_child_once_per_tick_until_the_limit() {
    let mut script = Script::default();
    script.set("a", Success);
    let mut tree = tree(leaf("a").repeat(Limit::Times(3)));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.run(&mut script, at(0)), Success);
    assert_eq!(script.count(Tick("a")), 3);

    // A fresh activation counts from zero again.
    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(script.count(Tick("a")), 4);
}

#[test]
fn looping_decorators_stop_on_the_other_status() {
    type Wrap = fn(Node<Scripted>) -> Node<Scripted>;
    let cases: [(&str, Wrap, Status); 3] = [
        ("Repeat", |node| node.repeat(Limit::Unlimited), Failure),
        (
            "RetryUntilSuccessful",
            |node| node.retry_until_successful(Limit::Unlimited),
            Success,
        ),
        (
            "KeepRunningUntilFailure",
            Node::keep_running_until_failure,
            Failure,
        ),
    ];
    for (name, wrap, stop) in cases {
        let mut script = Script::default();
        let again = if stop == Success { Failure } else { Success };
        script.set("a", again);
        let mut tree = tree(wrap(leaf("a")));

        for _ in 0..5 {
            assert_eq!(tree.run(&mut script, at(0)), Running, "{name}");
        }
        script.set("a", stop);
        assert_eq!(tree.run(&mut script, at(0)), stop, "{name}");
        assert_eq!(script.count(Tick("a")), 6, "{name}");
        script.assert_idle();
    }
}

#[test]
fn retry_until_successful_fails_after_the_limit() {
    let mut script = Script::default();
    script.set("a", Failure);
    let mut tree = tree(leaf("a").retry_until_successful(Limit::Times(2)));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.run(&mut script, at(0)), Failure);
    assert_eq!(script.count(Tick("a")), 2);
}

#[test]
fn zero_limits_finish_without_ticking_the_child() {
    let mut script = Script::default();
    let mut repeat = tree(leaf("a").repeat(Limit::Times(0)));
    let mut retry = tree(leaf("a").retry_until_successful(Limit::Times(0)));

    assert_eq!(repeat.run(&mut script, at(0)), Success);
    assert_eq!(retry.run(&mut script, at(0)), Failure);
    assert_eq!(script.log, []);
}

#[test]
fn halting_a_loop_halts_its_child_and_resets_the_count() {
    let mut script = Script::default();
    script.set("a", Success);
    let mut tree = tree(Node::sequence([leaf("a"), leaf("b")]).repeat(Limit::Times(2)));

    assert_eq!(tree.run(&mut script, at(0)), Running);
    tree.stop(&mut script);
    assert_eq!(script.count(Halt("b")), 1);

    script.set("b", Success);
    assert_eq!(tree.run(&mut script, at(0)), Running);
    assert_eq!(tree.run(&mut script, at(0)), Success);
    script.assert_idle();
}

#[test]
fn loops_are_due_at_once_between_runs() {
    let mut script = Script::default();
    let mut tree = tree(parallel_all([
        Node::sleep(MS(300)),
        leaf("a").repeat(Limit::Times(2)),
    ]));

    // A running child leaves only the sleep's deadline.
    assert_eq!(tree.run(&mut script, at(5)), Running);
    assert_eq!(tree.next_deadline(), Some(at(305)));

    // A finished run makes the next one due at the tick that finished it.
    script.set("a", Success);
    assert_eq!(tree.run(&mut script, at(7)), Running);
    assert_eq!(tree.next_deadline(), Some(at(7)));

    // The last run clears it.
    assert_eq!(tree.run(&mut script, at(9)), Running);
    assert_eq!(tree.next_deadline(), Some(at(305)));
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn unlimited_loops_do_not_count_runs() {
    // Seeds the counter instead of running 2^64 child runs; an unlimited loop
    // must keep going no matter how many runs came before.
    use super::super::slot::Run;
    use super::super::storage::sealed::Sealed as _;
    use super::Looping;

    for again_on in [Success, Failure] {
        let mut script = Script::default();
        script.set("a", again_on);
        let mut child = tree(leaf("a"));
        let (slots, leaves) = child.store.parts();
        let (mut runs, mut due) = (u64::MAX, None);
        let looping = Looping {
            again_on,
            limit: None,
            runs: &mut runs,
            due: &mut due,
        };
        let run = &mut Run {
            ctx: &mut script,
            observer: &mut (),
            leaves,
        };

        assert_eq!(
            super::looped(&mut slots[..1], 0, looping, run, at(3)),
            Running
        );
        assert_eq!((runs, due), (u64::MAX, Some(at(3))));
    }
}
