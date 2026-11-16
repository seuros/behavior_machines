use super::*;
use crate::Node;
use crate::Status::{Running, Success};
use crate::test_support::{Drive, Script, at, leaf, race, tree};

#[derive(Debug, PartialEq, Eq)]
enum Report {
    Ticked(usize, Status),
    Halted(usize),
}

#[derive(Default)]
struct Recorder(Vec<Report>);

impl Observer for Recorder {
    fn ticked(&mut self, node: NodeId, status: Status) {
        self.0.push(Report::Ticked(node.index(), status));
    }

    fn halted(&mut self, node: NodeId) {
        self.0.push(Report::Halted(node.index()));
    }
}

#[test]
fn reports_pre_order_ids_with_children_before_parents() {
    use Report::{Halted, Ticked};

    let mut script = Script::default();
    script.set("a", Success);
    // ids: 0 parallel, 1 sequence, 2 a, 3 b, 4 c
    let mut tree = tree(race([Node::sequence([leaf("a"), leaf("b")]), leaf("c")]));
    let mut recorder = Recorder::default();

    assert_eq!(
        tree.run_observed(&mut script, at(0), &mut recorder),
        Running
    );
    tree.stop_observed(&mut script, &mut recorder);

    assert_eq!(
        recorder.0,
        [
            Ticked(2, Success),
            Ticked(3, Running),
            Ticked(1, Running),
            Ticked(4, Running),
            Ticked(0, Running),
            Halted(3),
            Halted(1),
            Halted(4),
            Halted(0),
        ]
    );
}

#[test]
fn numbers_conditions_before_branches_and_bodies() {
    use Report::Ticked;

    let mut script = Script::default();
    for name in ["cond", "then", "keep"] {
        script.set(name, Success);
    }
    // ids: 0 sequence, 1 if, 2 cond, 3 then, 4 otherwise, 5 while, 6 keep, 7 body
    let mut tree = tree(Node::sequence([
        Node::if_then_else(leaf("cond"), leaf("then"), leaf("otherwise")),
        Node::while_do(leaf("keep"), leaf("body")),
    ]));
    let mut recorder = Recorder::default();

    // Observers also work as trait objects.
    let observer: &mut dyn Observer = &mut recorder;
    assert_eq!(tree.run_observed(&mut script, at(0), observer), Running);

    assert_eq!(
        recorder.0,
        [
            Ticked(2, Success),
            Ticked(3, Success),
            Ticked(1, Success),
            Ticked(6, Success),
            Ticked(7, Running),
            Ticked(5, Running),
            Ticked(0, Running),
        ]
    );
    tree.stop(&mut script);
    script.assert_idle();
}

#[test]
fn mutable_references_forward_to_the_observer() {
    fn report(mut observer: impl Observer) {
        observer.ticked(NodeId::new(1), Running);
        observer.halted(NodeId::new(1));
    }

    let mut recorder = Recorder::default();
    report(&mut recorder);

    assert_eq!(recorder.0, [Report::Ticked(1, Running), Report::Halted(1)]);
}
