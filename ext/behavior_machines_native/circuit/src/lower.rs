//! Lowering a [`Node`] definition into a circuit.
//!
//! # The three passes
//!
//! 1. **Flatten.** Walk the definition in the same depth-first pre-order the
//!    engine numbers [`NodeId`]s in, so a node's index here *is* its id. Every
//!    descendant of a node sits at a higher index, which makes the next two
//!    passes plain loops rather than recursions.
//! 2. **Arbitrate, bottom-up.** For each node compute its *raw* status: what
//!    it would return **if** it were ticked. A node's raw status depends only
//!    on its children's raw statuses and its own state cells, never on whether
//!    it is reached, so this pass needs no context from above.
//! 3. **Reach, top-down.** Compute which nodes the tick actually visits, and
//!    which running nodes get halted.
//!
//! Splitting raw from reached is what removes the recursion. A tree walk
//! interleaves "should I tick this?" with "what does it return?"; separating
//! them lets each question become a layer of arithmetic.
//!
//! # `ticked` is `reached`
//!
//! Every node in the supported set returns a status whenever it is ticked, so
//! the circuit does not mask statuses by reachability: it emits each node's raw
//! status alongside a `reached` bit and leaves the masking to the reader. That
//! saves three operations per node, and it is the reason
//! [`Outputs::status`] is documented as meaningful *only* where
//! [`Outputs::reached`] is set.
//!
//! # The ordered-composite identity
//!
//! Both sequence and fallback reduce to "the walk passes over a child, or
//! stops at it". With `advance_on` the status that continues the walk:
//!
//! ```text
//! before_i  = [i < cursor]                  the cursor already passed this child
//! blocked_i = NOT (before_i OR advance_i)   the walk cannot get past child i
//! clear_i   = [Σ_{j<i} blocked_j == 0]      every earlier child was passed
//! reach_i   = clear_i AND NOT before_i      so this child is ticked
//! stop_i    = clear_i AND blocked_i         and the walk stops here
//! ```
//!
//! `blocked` depends only on the cursor and each child's own status, never on
//! the other children, so the whole vector exists before any prefix test runs.
//! That is what lets `clear` come from one [`Builder::none_earlier`] call,
//! shared by both answers: `reach` and `stop` are each one `and` on top of it.
//!
//! `Σ blocked_j` is a non-negative integer, so the test against zero is a
//! single `ReLU` ([`Builder::is_zero`]). `stop` is at-most-one-hot by
//! construction, which makes every collapse over it free.
//!
//! A reactive composite is the same lowering with `cursor` pinned to zero,
//! where `before_i` folds to a constant and the per-child cost drops from
//! about ten operations to about seven.
//!
//! The depth does not grow with the number of children: every prefix test
//! reads a sum of bits, and a sum is free. Measured, an ordered composite
//! saturates at depth 4 reactive and 6 with memory from four children up to
//! any width, which `lower/tests.rs` pins.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Not;

use behavior_machines::{Node, Status};

use crate::builder::{Bit, Builder, Count};
use crate::plan::Plan;
use crate::status::StatusBits;
use crate::wire::{Affine, Circuit};

/// Why a definition could not be lowered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LowerError {
    /// The definition contains a node this version does not lower.
    ///
    /// The node is named rather than typed, because
    /// [`Node`] is `#[non_exhaustive]`: a node added to the engine after this
    /// crate was built lands here instead of silently compiling to something
    /// else.
    #[error("{0} is not lowered yet")]
    Unsupported(&'static str),
    /// A composite has no children, which the engine also rejects.
    #[error("a composite needs at least one child")]
    EmptyComposite,
}

/// A lowering invariant that did not hold. Every variant is a bug in this
/// crate, never in the caller.
#[derive(Clone, Copy, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum InvariantError {
    /// A value that should be 0 or 1 was neither.
    #[error("node {node}: {what} is {value}, not a bit")]
    NotABit {
        /// Which output.
        what: &'static str,
        /// The node it belongs to.
        node: usize,
        /// What it held.
        value: f32,
    },
    /// A status triple had more than one bit set.
    #[error("node {node}: status has {total} bits set, at most one is allowed")]
    NotOneHot {
        /// The node.
        node: usize,
        /// How many bits were set.
        total: f32,
    },
    /// A cursor was negative or fractional.
    #[error("cursor cell {cell} is {cursor}, not a non-negative integer")]
    BadCursor {
        /// The cursor cell.
        cell: usize,
        /// What it held.
        cursor: f32,
    },
}

/// A node kind, flattened. Child links are indices into the flat list, which
/// are also node ids.
#[derive(Clone, Debug)]
enum Flat {
    /// Reads its status from input cells; `slot` is its leaf index.
    Leaf { slot: usize },
    /// `AlwaysSuccess` and `AlwaysFailure`.
    Constant(Status),
    /// All four of sequence, fallback, and their reactive variants.
    Ordered {
        children: Vec<usize>,
        advance_on: Status,
        /// The cursor cell, or `None` for a reactive composite, which always
        /// restarts from its first child.
        cursor: Option<usize>,
    },
    /// `Inverter`, `ForceSuccess`, and `ForceFailure`.
    Map { child: usize, map: MapKind },
}

#[derive(Clone, Copy, Debug)]
enum MapKind {
    Invert,
    Force(Status),
}

/// What an ordered composite decided, kept for the top-down pass.
struct Arbitration {
    /// Per child: the walk reached it, so it was ticked.
    reach: Vec<Bit>,
    /// Per child: a stop strictly earlier, so a reactive composite halts it.
    after_stop: Vec<Bit>,
    /// The cursor to store for the next tick.
    next_cursor: Affine,
}

/// Everything the circuit computes, as wires into its value table.
#[derive(Clone, Debug)]
pub struct Outputs {
    /// Per node, in [`behavior_machines::NodeId`] order: the status it returns **if** reached.
    /// Meaningless where [`Outputs::reached`] is clear.
    pub status: Vec<StatusBits>,
    /// Per node: the tick visited it, so it was ticked and should be reported.
    pub reached: Vec<Bit>,
    /// Per node: it was running and this tick halted it.
    pub halted: Vec<Bit>,
    /// Per node: the `running` flag to store for the next tick.
    pub next_running: Vec<Bit>,
    /// Per cursor cell, in cell order: the cursor to store for the next tick.
    pub next_cursor: Vec<Affine>,
}

/// A lowered tree: the circuit, its cell layout, and its outputs.
#[derive(Clone, Debug)]
pub struct Lowered {
    circuit: Circuit,
    plan: Plan,
    outputs: Outputs,
}

impl Lowered {
    /// The circuit to evaluate.
    #[must_use]
    pub const fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// Where each input cell lives and what it means.
    #[must_use]
    pub const fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The wires to read a tick's results from.
    #[must_use]
    pub const fn outputs(&self) -> &Outputs {
        &self.outputs
    }

    /// Checks the invariants the lowering is supposed to hold, against an
    /// evaluated value table.
    ///
    /// Every status triple must be at-most-one-hot and every flag must be a
    /// real bit. Nothing downstream re-checks this: a lowering bug that breaks
    /// it would otherwise surface as a plausible-looking wrong status, so
    /// [`Lowered::evaluate`] runs this under `debug_assertions` and tests run
    /// it always.
    ///
    /// # Errors
    ///
    /// Returns the first broken invariant.
    pub fn validate(&self, values: &[f32]) -> Result<(), InvariantError> {
        // Exact comparisons are the point: these values are integers by
        // construction, so a bit that is near 1 rather than 1 is a bug.
        #[expect(clippy::float_cmp, reason = "the lowering holds exact integers")]
        let check_bit = |bit: &Bit, what: &'static str, node: usize| {
            let value = bit.form().value(values);
            (value == 0.0 || value == 1.0)
                .then_some(())
                .ok_or(InvariantError::NotABit { what, node, value })
        };

        for node in 0..self.plan.node_count() {
            let status = &self.outputs.status[node];
            for (what, bit) in [
                ("status.success", &status.success),
                ("status.failure", &status.failure),
                ("status.running", &status.running),
            ] {
                check_bit(bit, what, node)?;
            }
            let total = status.count().form().value(values);
            if total > 1.0 {
                return Err(InvariantError::NotOneHot { node, total });
            }
            check_bit(&self.outputs.reached[node], "reached", node)?;
            check_bit(&self.outputs.halted[node], "halted", node)?;
            check_bit(&self.outputs.next_running[node], "next_running", node)?;
        }

        for cell in 0..self.plan.cursor_count() {
            let cursor = self.outputs.next_cursor[cell].value(values);
            if cursor < 0.0 || cursor % 1.0 != 0.0 {
                return Err(InvariantError::BadCursor { cell, cursor });
            }
        }

        Ok(())
    }

    /// Lowers a definition.
    ///
    /// # Errors
    ///
    /// Returns [`LowerError::Unsupported`] for a node this version does not
    /// handle, or [`LowerError::EmptyComposite`] for a childless composite.
    pub fn new<L>(root: &Node<L>) -> Result<Self, LowerError> {
        let mut flat = Vec::new();
        let mut leaves = Vec::new();
        let mut cursors = Vec::new();
        flatten(root, &mut flat, &mut leaves, &mut cursors)?;

        let mut report_order = Vec::with_capacity(flat.len());
        post_order(&flat, 0, &mut report_order);
        let plan = Plan::new(flat.len(), leaves, cursors, report_order);
        let mut builder = Builder::new(plan.cell_count());
        let (raw, arbitrations) = arbitrate(&flat, &plan, &mut builder);
        let outputs = reach(&flat, &plan, &raw, &arbitrations, &mut builder);

        Ok(Self {
            circuit: builder.finish(),
            plan,
            outputs,
        })
    }
}

/// Pass 1: depth-first pre-order, matching the engine's [`NodeId`] numbering.
fn flatten<L>(
    node: &Node<L>,
    flat: &mut Vec<Flat>,
    leaves: &mut Vec<usize>,
    cursors: &mut Vec<usize>,
) -> Result<usize, LowerError> {
    let index = flat.len();
    // Reserve this node's slot before descending, so children get the higher
    // ids that pre-order demands.
    flat.push(Flat::Constant(Status::Success));

    let kind = match node {
        Node::Leaf(_) => {
            leaves.push(index);
            Flat::Leaf {
                slot: leaves.len() - 1,
            }
        }
        Node::AlwaysSuccess => Flat::Constant(Status::Success),
        Node::AlwaysFailure => Flat::Constant(Status::Failure),
        _ if let Some((children, advance_on, reactive)) = ordered_shape(node) => {
            if children.is_empty() {
                return Err(LowerError::EmptyComposite);
            }
            // A memory composite owns a cursor cell; a reactive one restarts
            // from its first child, so it needs none. Claim the cell before
            // descending, so cells follow the same pre-order as node ids.
            let cursor = (!reactive).then(|| {
                cursors.push(index);
                cursors.len() - 1
            });
            let mut links = Vec::with_capacity(children.len());
            for child in children {
                links.push(flatten(child, flat, leaves, cursors)?);
            }
            Flat::Ordered {
                children: links,
                advance_on,
                cursor,
            }
        }
        _ if let Some((child, map)) = map_shape(node) => Flat::Map {
            child: flatten(child, flat, leaves, cursors)?,
            map,
        },
        other => return Err(LowerError::Unsupported(name_of(other))),
    };

    flat[index] = kind;
    Ok(index)
}

/// The four ordered composites as data: their children, the status that
/// continues the walk, and whether they restart every tick.
///
/// Sequence and fallback differ only in that status, and the reactive variants
/// only in the cursor, so the lowering reads them from here instead of
/// repeating itself four times.
fn ordered_shape<L>(node: &Node<L>) -> Option<(&[Node<L>], Status, bool)> {
    match node {
        Node::Sequence(children) => Some((children, Status::Success, false)),
        Node::ReactiveSequence(children) => Some((children, Status::Success, true)),
        Node::Fallback(children) => Some((children, Status::Failure, false)),
        Node::ReactiveFallback(children) => Some((children, Status::Failure, true)),
        _ => None,
    }
}

/// The status-rewrite decorators as data: their child and the rewrite.
fn map_shape<L>(node: &Node<L>) -> Option<(&Node<L>, MapKind)> {
    match node {
        Node::Inverter(child) => Some((child, MapKind::Invert)),
        Node::ForceSuccess(child) => Some((child, MapKind::Force(Status::Success))),
        Node::ForceFailure(child) => Some((child, MapKind::Force(Status::Failure))),
        _ => None,
    }
}

/// Appends the subtree rooted at `index` in post-order, children first.
///
/// This is the order the engine's observer reports nodes in, because
/// `Slot::tick` reports itself only after ticking its children.
fn post_order(flat: &[Flat], index: usize, order: &mut Vec<usize>) {
    match &flat[index] {
        Flat::Leaf { .. } | Flat::Constant(_) => {}
        Flat::Map { child, .. } => post_order(flat, *child, order),
        Flat::Ordered { children, .. } => {
            for child in children {
                post_order(flat, *child, order);
            }
        }
    }
    order.push(index);
}

/// The node's name for an [`LowerError::Unsupported`] message. The wildcard
/// covers both the nodes this version skips and any the engine adds later.
const fn name_of<L>(node: &Node<L>) -> &'static str {
    match node {
        Node::Sleep(_) => "Sleep",
        Node::Parallel { .. } => "Parallel",
        Node::IfThenElse { .. } => "IfThenElse",
        Node::WhileDoElse { .. } => "WhileDoElse",
        Node::Timeout(..) => "Timeout",
        Node::Repeat(..) => "Repeat",
        Node::RetryUntilSuccessful(..) => "RetryUntilSuccessful",
        Node::KeepRunningUntilFailure(_) => "KeepRunningUntilFailure",
        _ => "this node",
    }
}

/// Pass 2: each node's raw status, bottom-up.
///
/// Pre-order guarantees every child sits at a higher index, so iterating in
/// reverse visits children before parents.
fn arbitrate(
    flat: &[Flat],
    plan: &Plan,
    builder: &mut Builder,
) -> (Vec<StatusBits>, Vec<Option<Arbitration>>) {
    let mut raw: Vec<StatusBits> = vec![StatusBits::unticked(); flat.len()];
    let mut arbitrations: Vec<Option<Arbitration>> = (0..flat.len()).map(|_| None).collect();

    for index in (0..flat.len()).rev() {
        match &flat[index] {
            Flat::Leaf { slot } => {
                raw[index] = plan.leaf_status(*slot, builder);
            }
            Flat::Constant(status) => {
                raw[index] = StatusBits::constant(*status);
            }
            Flat::Map { child, map } => {
                let child_status = raw[*child].clone();
                raw[index] = match map {
                    MapKind::Invert => child_status.inverted(),
                    MapKind::Force(forced) => child_status.forced(*forced),
                };
            }
            Flat::Ordered {
                children,
                advance_on,
                cursor,
            } => {
                let child_status: Vec<StatusBits> =
                    children.iter().map(|child| raw[*child].clone()).collect();
                let (status, arbitration) =
                    lower_ordered(&child_status, *advance_on, *cursor, plan, builder);
                raw[index] = status;
                arbitrations[index] = Some(arbitration);
            }
        }
    }

    (raw, arbitrations)
}

/// The ordered-composite identity from the [module docs](self).
fn lower_ordered(
    children: &[StatusBits],
    advance_on: Status,
    cursor: Option<usize>,
    plan: &Plan,
    builder: &mut Builder,
) -> (StatusBits, Arbitration) {
    let cursor_count = cursor.map_or_else(
        || Count::constant(0.0),
        |cell| Count::assume(plan.cursor_wire(cell, builder)),
    );

    // Whether the walk gets past each child. This depends only on the cursor
    // and that child's own status, never on the other children, so the whole
    // vector is available before any prefix test runs.
    let mut before: Vec<Bit> = Vec::with_capacity(children.len());
    let mut blocked: Vec<Bit> = Vec::with_capacity(children.len());
    for (index, child) in children.iter().enumerate() {
        // Children the cursor already passed over. Constant-folds away for a
        // reactive composite, whose cursor is pinned to zero.
        #[expect(clippy::cast_precision_loss, reason = "child counts are small")]
        let position = index as f32;
        let starts_later = builder.at_least(&cursor_count, position + 1.0);
        let advance = child.bit(advance_on).clone();
        let pass = builder.or(&starts_later, &advance);
        before.push(starts_later);
        // The walk stops at the first child it cannot pass. Negation is free.
        blocked.push(!pass);
    }

    // One prefix test per child, shared by both answers: `none_earlier` here
    // means every earlier child was passed over.
    let all_passed = builder.none_earlier(&blocked);

    let mut reach: Vec<Bit> = Vec::with_capacity(children.len());
    let mut stop: Vec<Bit> = Vec::with_capacity(children.len());
    let mut after_stop: Vec<Bit> = Vec::with_capacity(children.len());
    for index in 0..children.len() {
        // `stop` is at-most-one-hot, so the running total of earlier stops is
        // itself a bit and costs nothing to form.
        after_stop.push(if cursor.is_some() {
            Bit::constant(false)
        } else {
            Bit::or_disjoint(stop.iter().cloned())
        });
        reach.push(builder.and(&all_passed[index], &!before[index].clone()));
        stop.push(builder.and(&all_passed[index], &blocked[index]));
    }

    // The composite reports the stopping child's status, or `advance_on` when
    // the walk ran off the end. A stopping child never holds the `advance_on`
    // bit, so that bit stays free for the ran-off-the-end case.
    let mut status = StatusBits::unticked();
    let mut running_picks = Vec::with_capacity(children.len());
    for (index, child) in children.iter().enumerate() {
        for picked in [Status::Success, Status::Failure, Status::Running] {
            if picked == advance_on {
                continue;
            }
            let bit = child.bit(picked);
            if bit.as_constant() == Some(false) || stop[index].as_constant() == Some(false) {
                continue;
            }
            let masked = builder.and(&stop[index], bit);
            if picked == Status::Running {
                running_picks.push((index, masked.clone()));
            }
            *status.bit_mut(picked) = Bit::or_disjoint([status.bit(picked).clone(), masked]);
        }
    }
    let any_stop = Bit::or_disjoint(stop.iter().cloned());
    *status.bit_mut(advance_on) = any_stop.not();

    // The cursor remembers where to resume: the stopping child's position when
    // it is still running, and zero otherwise, which is also what the engine
    // stores when a composite finishes.
    let mut next_cursor = Affine::constant(0.0);
    for (index, running) in running_picks {
        #[expect(clippy::cast_precision_loss, reason = "child counts are small")]
        let position = index as f32;
        next_cursor = next_cursor.plus(&running.into_form().scaled(position));
    }

    (
        status,
        Arbitration {
            reach,
            after_stop,
            next_cursor,
        },
    )
}

/// Pass 3: which nodes the tick visits, and which running nodes it halts.
///
/// Forward iteration visits a parent before its children, which is what both
/// reachability and halt propagation need.
fn reach(
    flat: &[Flat],
    plan: &Plan,
    raw: &[StatusBits],
    arbitrations: &[Option<Arbitration>],
    builder: &mut Builder,
) -> Outputs {
    let mut reached: Vec<Bit> = vec![Bit::constant(false); flat.len()];
    // The engine's `halt` returns early on a node that is not running, so a
    // halt never reaches the children of a stopped node.
    let mut halt_called: Vec<Bit> = vec![Bit::constant(false); flat.len()];
    let mut halted: Vec<Bit> = vec![Bit::constant(false); flat.len()];
    reached[0] = Bit::constant(true);

    let mut next_cursor = vec![Affine::constant(0.0); plan.cursor_count()];

    // Filled as the loop reaches each node, and read again afterwards for the
    // `running` write-back. `untouched` is one operation per node, so it is
    // computed here once rather than at both use sites.
    let mut was_running: Vec<Bit> = Vec::with_capacity(flat.len());
    let mut untouched: Vec<Bit> = Vec::with_capacity(flat.len());

    for index in 0..flat.len() {
        was_running.push(Bit::assume(plan.running_wire(index, builder)));
        halted[index] = builder.and(&halt_called[index], &was_running[index]);
        untouched.push(!Bit::or_disjoint([
            reached[index].clone(),
            halted[index].clone(),
        ]));

        match &flat[index] {
            Flat::Leaf { .. } | Flat::Constant(_) => {}
            Flat::Map { child, .. } => {
                reached[*child] = reached[index].clone();
                halt_called[*child] = halted[index].clone();
            }
            Flat::Ordered {
                children, cursor, ..
            } => {
                let Some(arbitration) = &arbitrations[index] else {
                    // `arbitrate` records one for every ordered node.
                    continue;
                };
                if let Some(cell) = cursor {
                    // The arbitrated cursor is what the composite *would*
                    // store if ticked, so it only applies where the tick
                    // reached it. An untouched composite keeps the cursor it
                    // had, and a halted one resets to its first child, which
                    // is what `Slot::halt` does.
                    let current = Affine::wire(plan.cursor_wire(*cell, builder));
                    let advanced = builder.mask(&arbitration.next_cursor, &reached[index]);
                    let retained = builder.mask(&current, &untouched[index]);
                    next_cursor[*cell] = Affine::wire(advanced).plus(&Affine::wire(retained));
                }
                for (slot, child) in children.iter().enumerate() {
                    reached[*child] = builder.and(&reached[index], &arbitration.reach[slot]);
                    // A reactive composite halts every child after the stop;
                    // `after_stop` is constant zero for a memory composite, so
                    // the `or` folds to the inherited halt.
                    let sibling = builder.and(&reached[index], &arbitration.after_stop[slot]);
                    halt_called[*child] = builder.or(&halted[index], &sibling);
                }
            }
        }
    }

    let next_running = (0..flat.len())
        .map(|index| {
            // A ticked node's flag is its own result; an untouched one keeps
            // the flag it had. The two cases are disjoint, so the collapse is
            // free.
            let now_running = builder.and(&reached[index], &raw[index].running);
            let still = builder.and(&untouched[index], &was_running[index]);
            Bit::or_disjoint([now_running, still])
        })
        .collect();

    Outputs {
        status: raw.to_vec(),
        reached,
        halted,
        next_running,
        next_cursor,
    }
}

#[cfg(test)]
mod tests;
