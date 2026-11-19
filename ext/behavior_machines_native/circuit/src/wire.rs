//! The circuit arena: wires, affine forms, and the two instructions.
//!
//! A circuit is a list of input cells followed by an ordered list of
//! operations. Each operation appends exactly one value, and a [`Wire`] is a
//! position in that combined table. Operations read inputs and earlier
//! operations, never later ones, so the table is a DAG in topological order by
//! construction.
//!
//! # Why `f32` is exact here
//!
//! Every value a lowered behavior tree produces is an integer: a bit, a small
//! count, or a child index. All of them stay below `2^24`, the node limit
//! [`Lowered::MAX_NODES`](crate::Lowered::MAX_NODES) enforces, where every
//! integer is representable exactly in `f32`, so no operation in a lowered
//! tree ever rounds. That is a property of the lowering, not a hope, and
//! [`Circuit::evaluate`] checks it under `debug_assertions`.
//!
//! This is what lets the builder normalize and combine affine terms freely.
//! A general-purpose float circuit cannot: there, the order terms are summed
//! is part of the result, so combining two sums changes the answer. Here it
//! cannot, which removes an entire class of correctness rules.

use alloc::vec;
use alloc::vec::Vec;

use crate::Refusal;

/// A position in a circuit's value table.
///
/// Wires are opaque on purpose: a plain number does not type-check where a
/// wire is expected, so an index into the wrong table cannot be passed by
/// accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Wire(u32);

impl Wire {
    /// Input cell `index`, for a plan that laid the cells out itself and
    /// sized the circuit to hold them.
    pub(crate) const fn cell(index: u32) -> Self {
        Self(index)
    }

    /// The wire's position in the value table.
    #[must_use]
    pub const fn position(self) -> usize {
        self.0 as usize
    }
}

/// A bias plus a weighted sum of wires: `bias + Σ coefficient × value[wire]`.
///
/// Terms are kept sorted by wire with duplicates combined, so two affine forms
/// that describe the same sum compare equal. That is sound only because every
/// value in a lowered tree is a small integer, exactly representable in `f32`.
#[derive(Clone, Debug, PartialEq)]
pub struct Affine {
    bias: f32,
    terms: Vec<(Wire, f32)>,
}

impl Affine {
    /// A constant form with no terms.
    #[must_use]
    pub const fn constant(bias: f32) -> Self {
        Self {
            bias,
            terms: Vec::new(),
        }
    }

    /// `bias + Σ coefficient × wire`, normalized.
    #[must_use]
    pub fn new(bias: f32, terms: impl IntoIterator<Item = (Wire, f32)>) -> Self {
        let mut form = Self {
            bias,
            terms: terms.into_iter().collect(),
        };
        form.normalize();
        form
    }

    /// A form reading one wire with coefficient 1.
    #[must_use]
    pub fn wire(wire: Wire) -> Self {
        Self::new(0.0, [(wire, 1.0)])
    }

    /// This form plus `shift`.
    #[must_use]
    pub fn offset(mut self, shift: f32) -> Self {
        self.bias += shift;
        self
    }

    /// This form with every coefficient and the bias scaled by `factor`.
    #[must_use]
    pub fn scaled(mut self, factor: f32) -> Self {
        self.bias *= factor;
        for (_, coefficient) in &mut self.terms {
            *coefficient *= factor;
        }
        self.normalize();
        self
    }

    /// The sum of two forms, normalized.
    #[must_use]
    pub fn plus(&self, other: &Self) -> Self {
        // Both sides are already normalized, so one merge keeps the result
        // sorted, deduplicated, and free of zero coefficients without a sort.
        let mut terms = Vec::with_capacity(self.terms.len() + other.terms.len());
        let mut left = self.terms.iter().copied().peekable();
        let mut right = other.terms.iter().copied().peekable();
        loop {
            let next = match (left.peek(), right.peek()) {
                (Some(&(a, x)), Some(&(b, y))) if a == b => {
                    left.next();
                    right.next();
                    (a, x + y)
                }
                (Some(&(a, _)), Some(&(b, y))) if b < a => {
                    right.next();
                    (b, y)
                }
                (Some(&term), _) => {
                    left.next();
                    term
                }
                (None, Some(&term)) => {
                    right.next();
                    term
                }
                (None, None) => break,
            };
            if next.1 != 0.0 {
                terms.push(next);
            }
        }
        Self {
            bias: self.bias + other.bias,
            terms,
        }
    }

    /// The constant value of a form with no terms.
    #[must_use]
    pub fn as_constant(&self) -> Option<f32> {
        self.terms.is_empty().then_some(self.bias)
    }

    /// The form's bias.
    #[must_use]
    pub const fn bias(&self) -> f32 {
        self.bias
    }

    /// The form's terms, sorted by wire.
    #[must_use]
    pub fn terms(&self) -> &[(Wire, f32)] {
        &self.terms
    }

    /// Sorts terms by wire, combines duplicates, and drops zero coefficients.
    fn normalize(&mut self) {
        self.terms.sort_unstable_by_key(|(wire, _)| *wire);
        self.terms.dedup_by(|(wire, coefficient), (kept, total)| {
            let same = wire == kept;
            if same {
                *total += *coefficient;
            }
            same
        });
        self.terms.retain(|(_, coefficient)| *coefficient != 0.0);
    }

    /// The deepest layer any wire this form reads sits in.
    fn depth(&self, depths: &[u32]) -> u32 {
        self.terms
            .iter()
            .map(|(wire, _)| depths.get(wire.position()).copied().unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    /// The form's value against an evaluated value table, or `None` if the
    /// table is shorter than a wire this form reads, which means it came
    /// from a different circuit.
    #[must_use]
    pub fn value(&self, values: &[f32]) -> Option<f32> {
        self.terms
            .iter()
            .try_fold(self.bias, |total, (wire, coefficient)| {
                Some(total + coefficient * values.get(wire.position())?)
            })
    }

    /// The value during an evaluation, whose table always holds every wire
    /// an operation reads: operations read only earlier positions.
    fn evaluate(&self, values: &[f32]) -> f32 {
        self.value(values).unwrap_or_else(|| {
            debug_assert!(false, "an operation reads only earlier values");
            0.0
        })
    }
}

/// One instruction. Each appends exactly one value to the table.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// The affine form itself.
    Affine(Affine),
    /// `left × max(0, gate)`, the rectified-gate product.
    ///
    /// With `left` the constant 1 this is a plain `ReLU` of `gate`, which is how
    /// every comparison in a lowered tree is built. With `left` a value and
    /// `gate` a bit it is a mask: the value when the bit is 1, zero otherwise.
    Reglu {
        /// The value being gated.
        left: Affine,
        /// The gate, rectified before multiplying.
        gate: Affine,
    },
}

impl Op {
    fn depth(&self, depths: &[u32]) -> u32 {
        match self {
            Self::Affine(form) => form.depth(depths),
            Self::Reglu { left, gate } => left.depth(depths).max(gate.depth(depths)),
        }
    }

    fn evaluate(&self, values: &[f32]) -> f32 {
        match self {
            Self::Affine(form) => form.evaluate(values),
            Self::Reglu { left, gate } => left.evaluate(values) * gate.evaluate(values).max(0.0),
        }
    }
}

/// An append-only arena of input cells and operations.
///
/// Build one with [`Builder`](crate::Builder), which adds the bit and
/// comparison constructions on top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Circuit {
    inputs: u32,
    ops: Vec<Op>,
}

impl Circuit {
    /// A circuit with `inputs` input cells and no operations.
    #[must_use]
    pub const fn new(inputs: u32) -> Self {
        Self {
            inputs,
            ops: Vec::new(),
        }
    }

    /// How many input cells the circuit reads.
    #[must_use]
    pub const fn inputs(&self) -> u32 {
        self.inputs
    }

    /// The operations, in the order they were appended.
    #[must_use]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// The wire of input cell `index`, or `None` past the circuit's input
    /// width.
    #[must_use]
    pub const fn input(&self, index: u32) -> Option<Wire> {
        if index < self.inputs {
            Some(Wire(index))
        } else {
            None
        }
    }

    /// How many values a full evaluation produces.
    #[must_use]
    pub const fn value_count(&self) -> usize {
        // Cannot overflow: `push` keeps the value count at most `u32::MAX`,
        // which fits `usize` on every supported target.
        self.inputs as usize + self.ops.len()
    }

    /// Appends an operation and returns its wire.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the value table already holds `u32::MAX`
    /// values, the most a [`Wire`] addresses; nothing is appended.
    pub fn push(&mut self, op: Op) -> Result<Wire, Refusal> {
        let position = u32::try_from(self.value_count())
            .ok()
            .filter(|position| *position < u32::MAX)
            .ok_or(Refusal::Full)?;
        self.ops.push(op);
        Ok(Wire(position))
    }

    /// Drops every operation past the first `ops`, undoing a construction
    /// that was refused part way.
    pub(crate) fn truncate(&mut self, ops: usize) {
        self.ops.truncate(ops);
    }

    /// The layer of every value, inputs first.
    ///
    /// An operation's layer is one past the deepest value it reads, so
    /// operations sharing a layer are independent and evaluate together. The
    /// last layer is the circuit's [`depth`](Self::depth).
    #[must_use]
    pub fn layers(&self) -> Vec<u32> {
        // Inputs sit in layer zero; each operation reads only earlier values,
        // so appending in order fills the table without indexing into it. A
        // depth is at most the operation count, itself bounded by the `u32`
        // wire space, so the increment never saturates.
        let mut depths = vec![0; self.inputs as usize];
        depths.reserve(self.ops.len());
        for op in &self.ops {
            let depth = op.depth(&depths).saturating_add(1);
            depths.push(depth);
        }
        depths
    }

    /// How many layers deep the circuit is: the longest chain of dependent
    /// operations, and so the number of sequential steps a tick costs.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.layers().iter().copied().max().unwrap_or(0)
    }

    /// Evaluates every value from the input cells.
    ///
    /// # Errors
    ///
    /// [`Refusal::InputCount`] if `inputs` is not exactly the circuit's input
    /// width.
    pub fn evaluate(&self, inputs: &[f32]) -> Result<Vec<f32>, Refusal> {
        if inputs.len() != self.inputs as usize {
            return Err(Refusal::InputCount {
                expected: self.inputs as usize,
                given: inputs.len(),
            });
        }
        let mut values = Vec::with_capacity(self.value_count());
        values.extend_from_slice(inputs);
        for op in &self.ops {
            let value = op.evaluate(&values);
            // `f32::fract` and `f32::abs` live in std; this crate is no_std,
            // and `%` is enough to catch a non-integer.
            debug_assert!(
                value % 1.0 == 0.0 && value > -16_777_216.0 && value < 16_777_216.0,
                "a lowered tree holds only small integers, got {value}: the lowering is wrong",
            );
            values.push(value);
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests;
