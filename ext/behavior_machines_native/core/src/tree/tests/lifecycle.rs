//! Every node shape through every pair of leaf outcomes.

use core::time::Duration;

use crate::test_support::{Drive, Script, Scripted, at, leaf, parallel_all, race, tree};
use crate::{Limit, Node, Threshold};

use crate::Status::{Failure, Running, Success};

/// Every stateful node shape, over leaves `a`, `b`, `c`.
fn lifecycle_shapes() -> [Node<Scripted>; 21] {
    let abc = || [leaf("a"), leaf("b"), leaf("c")];
    let ms = Duration::from_millis;
    let two = Threshold::Count(2);
    [
        Node::sequence(abc()),
        Node::reactive_sequence(abc()),
        Node::fallback(abc()),
        Node::reactive_fallback(abc()),
        parallel_all(abc()),
        crate::test_support::parallel_any(abc()),
        race(abc()),
        Node::parallel(abc(), two, two),
        Node::if_then_else(leaf("a"), leaf("b"), leaf("c")),
        Node::if_then(leaf("a"), leaf("b")),
        Node::while_do_else(leaf("a"), leaf("b"), leaf("c")),
        Node::while_do(leaf("a"), leaf("b")),
        Node::sequence(abc()).inverter(),
        parallel_all(abc()).force_success(),
        Node::fallback(abc()).force_failure(),
        Node::sequence(abc()).timeout(ms(2)),
        Node::sequence(abc()).repeat(Limit::Times(2)),
        Node::fallback(abc()).retry_until_successful(Limit::Times(2)),
        Node::sequence(abc()).keep_running_until_failure(),
        Node::sequence([Node::sleep(ms(2)), leaf("a")]),
        Node::fallback([Node::AlwaysFailure, leaf("a"), Node::AlwaysSuccess]),
    ]
}

/// Scripts `a`, `b`, `c` from the base-3 digits of `code`.
fn script_outcomes(script: &mut Script, mut code: usize) {
    for name in ["a", "b", "c"] {
        script.set(name, [Running, Success, Failure][code % 3]);
        code /= 3;
    }
}

/// Every node shape, through every pair of leaf outcomes: no leaked
/// activations, no double halts (the script panics on those), and a tree that
/// finished or was halted behaves exactly like a freshly compiled one.
#[test]
fn every_shape_resets_to_fresh_after_finishing_or_halting() {
    for shape in 0..lifecycle_shapes().len() {
        for first in 0..27 {
            for second in 0..27 {
                let build = || lifecycle_shapes().into_iter().nth(shape).unwrap();
                let case = format!("shape={shape}, first={first}, second={second}");
                let mut script = Script::default();
                let mut reused = tree(build());

                for (ms, code) in [(0, first), (1, second)] {
                    script_outcomes(&mut script, code);
                    if reused.run(&mut script, at(ms)) != Running {
                        script.assert_idle();
                        assert_eq!(reused.next_deadline(), None, "{case}");
                    }
                }
                reused.stop(&mut script);
                script.assert_idle();
                let halted = script.log.len();
                reused.stop(&mut script);
                assert_eq!(script.log.len(), halted, "{case}: second halt acted");
                assert_eq!(reused.next_deadline(), None, "{case}");

                script.log.clear();
                let mut fresh_script = Script::default();
                let mut fresh = tree(build());
                for (ms, code) in [(10, second), (11, first), (12, second)] {
                    script_outcomes(&mut script, code);
                    script_outcomes(&mut fresh_script, code);
                    assert_eq!(
                        reused.run(&mut script, at(ms)),
                        fresh.run(&mut fresh_script, at(ms)),
                        "{case}, ms={ms}"
                    );
                    assert_eq!(
                        reused.next_deadline(),
                        fresh.next_deadline(),
                        "{case}, ms={ms}"
                    );
                    assert_eq!(script.log, fresh_script.log, "{case}, ms={ms}");
                }
                reused.stop(&mut script);
                fresh.stop(&mut fresh_script);
                assert_eq!(script.log, fresh_script.log, "{case}");
                script.assert_idle();
            }
        }
    }
}
