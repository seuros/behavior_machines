//! Fixtures shared by the integration tests that compare the circuit with the
//! regular engine.
//!
//! Each file in `tests/` is its own crate, so this lives in a subdirectory
//! (which Cargo does not treat as a test target) and every test crate pulls it
//! in with `mod support { pub mod fixtures; }`.

use behavior_machines::{Node, NodeId, Observer, Status};

/// Records what the engine reports, to compare against the schedule.
#[derive(Default)]
pub struct Record {
    pub ticked: Vec<(usize, Status)>,
    pub halted: Vec<usize>,
}

impl Observer for Record {
    fn ticked(&mut self, node: NodeId, status: Status) {
        self.ticked.push((node.index(), status));
    }

    fn halted(&mut self, node: NodeId) {
        self.halted.push(node.index());
    }
}

/// A leaf with no payload; the tests number or classify leaves afterwards.
pub const fn leaf() -> Node<()> {
    Node::leaf(())
}

/// How many leaves `tree` has.
pub fn leaf_count(tree: &Node<()>) -> usize {
    let mut count = 0;
    tree.clone().map(|()| count += 1);
    count
}

const STATUSES: [Status; 3] = [Status::Success, Status::Failure, Status::Running];

/// Every assignment of statuses to `slots` positions, as a flat counter in
/// base 3.
pub fn each_assignment(slots: usize, mut visit: impl FnMut(&[Status])) {
    let mut assignment = vec![Status::Success; slots];
    let Ok(slots_exponent) = u32::try_from(slots) else {
        panic!("few enough slots")
    };
    let total = 3usize.pow(slots_exponent);
    for encoded in 0..total {
        let mut rest = encoded;
        for slot in &mut assignment {
            *slot = STATUSES[rest % 3];
            rest /= 3;
        }
        visit(&assignment);
    }
}
