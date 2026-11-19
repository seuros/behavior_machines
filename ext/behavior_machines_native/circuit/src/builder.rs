//! Bit algebra and comparisons over the circuit arena.
//!
//! Everything a lowered behavior tree computes is a bit or a small count, so
//! the builder works in [`Bit`]s: affine forms carrying the invariant that
//! they hold exactly 0 or 1. The invariant is what makes the constructions
//! here cheap.
//!
//! Two identities do most of the work, and both rely on the operand being a
//! **non-negative integer**, which every count in a lowered tree is:
//!
//! ```text
//! [count == 0]  = max(0, 1 - count)              one ReLU
//! [count >= k]  = max(0, count - k + 1) - max(0, count - k)
//! ```
//!
//! The first is the whole reason selection is shallow. A general `==` needs
//! two comparisons and four `ReLUs`; a test against zero, which is what
//! "nothing earlier matched" reduces to, needs one operation.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Not;

use crate::Refusal;
use crate::wire::{Affine, Circuit, Op, Wire};

/// Generates `assume` and `form` inherent methods for a newtype wrapping [`Affine`].
macro_rules! impl_affine_form {
    ($name:ident, $doc:expr) => {
        impl $name {
            #[doc = $doc]
            #[must_use]
            pub fn assume(wire: Wire) -> Self {
                Self(Affine::wire(wire))
            }

            /// The underlying affine form.
            #[must_use]
            pub const fn form(&self) -> &Affine {
                &self.0
            }
        }
    };
}

/// An affine form that holds exactly 0 or 1.
///
/// The type is the proof: [`Bit::not`] and [`Bit::or_disjoint`] are only valid
/// on values that are really bits, and every constructor here establishes
/// that. Build one from a wire you have already proven with
/// [`Bit::assume`].
#[derive(Clone, Debug, PartialEq)]
pub struct Bit(Affine);

impl Bit {
    /// The constant bit 0.
    pub const ZERO: Self = Self(Affine::constant(0.0));

    /// The constant bit 1.
    pub const ONE: Self = Self(Affine::constant(1.0));

    /// Consumes the bit, returning its affine form.
    #[must_use]
    pub fn into_form(self) -> Affine {
        self.0
    }

    /// The sum of bits that cannot both be 1. Free: no operation is appended.
    ///
    /// The caller promises at most one input is set. With that, `or` is just
    /// addition, which is why a one-hot selection costs nothing to collapse.
    #[must_use]
    pub fn or_disjoint(bits: impl IntoIterator<Item = Self>) -> Self {
        Self(sum_bits(bits))
    }

    /// The constant value of a bit that is already decided.
    #[must_use]
    pub fn as_constant(&self) -> Option<bool> {
        self.0.as_constant().map(|value| value != 0.0)
    }
}

/// The constant bit of a decided condition: [`Bit::ONE`] if it holds.
impl From<bool> for Bit {
    fn from(set: bool) -> Self {
        if set { Self::ONE } else { Self::ZERO }
    }
}

impl_affine_form!(
    Bit,
    concat!(
        "Treats `wire` as a bit.\n\n",
        "The caller promises the wire holds exactly 0 or 1. A false promise does\n",
        "not trip an assertion; it silently produces a circuit that computes\n",
        "something else, so establish it from an input declaration or from a\n",
        "construction in this module.",
    )
);

impl Not for Bit {
    type Output = Self;

    /// `1 - self`. Free: no operation is appended, which is why every
    /// negation in the lowering is untracked cost.
    fn not(self) -> Self {
        Self(self.0.scaled(-1.0).offset(1.0))
    }
}

/// A count: an affine form holding a non-negative integer.
///
/// Counts are what the comparison identities above accept. Producing one by
/// summing bits is free, so the builder never materializes a count unless an
/// operation reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Count(Affine);

impl Count {
    /// The number of set bits among `bits`. Free: no operation is appended.
    #[must_use]
    pub fn of(bits: impl IntoIterator<Item = Bit>) -> Self {
        Self(sum_bits(bits))
    }

    /// A constant count.
    #[must_use]
    pub const fn constant(value: f32) -> Self {
        Self(Affine::constant(value))
    }

    /// `total - Σ bits`, a count when the bits cannot exceed `total`.
    #[must_use]
    pub fn shortfall(total: f32, bits: impl IntoIterator<Item = Bit>) -> Self {
        let mut form = Affine::constant(total);
        for bit in bits {
            form = form.plus(&bit.0.scaled(-1.0));
        }
        Self(form)
    }

    /// The constant value of a count that is already decided.
    #[must_use]
    pub fn as_constant(&self) -> Option<f32> {
        self.0.as_constant()
    }
}

impl_affine_form!(
    Count,
    concat!(
        "Treats `wire` as a non-negative integer count, the same kind of\n",
        "promise [`Bit::assume`] takes.",
    )
);

/// Appends operations to a [`Circuit`] using the bit algebra.
#[derive(Clone, Debug)]
pub struct Builder {
    circuit: Circuit,
}

impl Builder {
    /// A builder over a circuit with `inputs` input cells.
    #[must_use]
    pub const fn new(inputs: u32) -> Self {
        Self {
            circuit: Circuit::new(inputs),
        }
    }

    /// The circuit built so far.
    #[must_use]
    pub const fn circuit(&self) -> &Circuit {
        &self.circuit
    }

    /// Consumes the builder, returning the circuit.
    #[must_use]
    pub fn finish(self) -> Circuit {
        self.circuit
    }

    /// The wire of input cell `index`, or `None` past the circuit's input
    /// width.
    #[must_use]
    pub const fn input(&self, index: u32) -> Option<Wire> {
        self.circuit.input(index)
    }

    /// Runs a construction that appends several operations, and drops every
    /// one of them if it is refused, so a refusal leaves the circuit as it
    /// was.
    fn whole<T>(
        &mut self,
        construct: impl FnOnce(&mut Self) -> Result<T, Refusal>,
    ) -> Result<T, Refusal> {
        let before = self.circuit.ops().len();
        let built = construct(self);
        if built.is_err() {
            self.circuit.truncate(before);
        }
        built
    }

    /// `max(0, form)`, as one operation.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted.
    pub fn relu(&mut self, form: Affine) -> Result<Wire, Refusal> {
        self.circuit.push(Op::Reglu {
            left: Affine::constant(1.0),
            gate: form,
        })
    }

    /// `[count == 0]`, as one operation, or none if the count is constant.
    ///
    /// Valid only for a non-negative integer count, where `max(0, 1 - count)`
    /// is 1 at zero and 0 at every larger integer.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted.
    pub fn is_zero(&mut self, count: &Count) -> Result<Bit, Refusal> {
        if let Some(value) = count.as_constant() {
            return Ok(Bit::from(value == 0.0));
        }
        let wire = self.relu(count.0.clone().scaled(-1.0).offset(1.0))?;
        Ok(Bit::assume(wire))
    }

    /// `[count >= threshold]`, as two operations, or none if the count is
    /// constant.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted; nothing is
    /// appended.
    pub fn at_least(&mut self, count: &Count, threshold: f32) -> Result<Bit, Refusal> {
        if let Some(value) = count.as_constant() {
            return Ok(Bit::from(value >= threshold));
        }
        let shifted = count.0.clone().offset(1.0 - threshold);
        self.whole(|builder| {
            let high = builder.relu(shifted.clone())?;
            let low = builder.relu(shifted.offset(-1.0))?;
            Ok(Bit(Affine::new(0.0, [(high, 1.0), (low, -1.0)])))
        })
    }

    /// `left OR right`, as one operation: `a + b - a × b`.
    ///
    /// Prefer [`Bit::or_disjoint`] when the bits cannot both be set, which
    /// costs nothing.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted.
    pub fn or(&mut self, left: &Bit, right: &Bit) -> Result<Bit, Refusal> {
        if left.as_constant() == Some(true) || right.as_constant() == Some(true) {
            return Ok(Bit::ONE);
        }
        if left.as_constant() == Some(false) {
            return Ok(right.clone());
        }
        if right.as_constant() == Some(false) {
            return Ok(left.clone());
        }
        let both = self.and(left, right)?;
        Ok(Bit(left.0.plus(&right.0).plus(&both.0.scaled(-1.0))))
    }

    /// `left AND right`, as one operation.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted.
    pub fn and(&mut self, left: &Bit, right: &Bit) -> Result<Bit, Refusal> {
        if left.as_constant() == Some(true) {
            return Ok(right.clone());
        }
        if right.as_constant() == Some(true) {
            return Ok(left.clone());
        }
        if left.as_constant() == Some(false) || right.as_constant() == Some(false) {
            return Ok(Bit::ZERO);
        }
        let wire = self.circuit.push(Op::Reglu {
            left: left.0.clone(),
            gate: right.0.clone(),
        })?;
        Ok(Bit::assume(wire))
    }

    /// `value` when `bit` is 1, and 0 when it is 0, as one operation.
    ///
    /// The gate is rectified, so this is exact for any `value`, including a
    /// negative one.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted.
    pub fn mask(&mut self, value: &Affine, bit: &Bit) -> Result<Wire, Refusal> {
        self.circuit.push(Op::Reglu {
            left: value.clone(),
            gate: bit.0.clone(),
        })
    }

    /// Per position: no earlier bit is set. One operation each, plus one per
    /// block of 64 bits past the first.
    ///
    /// This is the prefix test every ordered composite turns on — "did the
    /// walk get this far" — and it is one rectified gate per position because
    /// [`Builder::is_zero`] is. The result is at-most-one-hot when `and`-ed
    /// with the bits themselves, which is how a stopping child is picked.
    ///
    /// Up to 64 bits, every prefix sum is read inline and the test sits one
    /// layer above the bits. Wider, each block of 64 has its sum materialized
    /// and the block offsets are the same problem one level up: stored terms
    /// stay `O(n log n)` instead of `O(n²)`, for one more layer per factor of
    /// 64 in width. A run of constant bits still costs nothing to sum.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] if the circuit's wire space is exhausted; nothing is
    /// appended.
    pub fn none_earlier(&mut self, bits: &[Bit]) -> Result<Vec<Bit>, Refusal> {
        self.none_earlier_in_blocks(bits, PREFIX_RADIX)
    }

    /// [`Builder::none_earlier`] with a block sum every `radix` bits.
    fn none_earlier_in_blocks(&mut self, bits: &[Bit], radix: usize) -> Result<Vec<Bit>, Refusal> {
        self.whole(|builder| {
            let forms: Vec<&Affine> = bits.iter().map(Bit::form).collect();
            builder
                .prefix_sums(&forms, radix)?
                .into_iter()
                .map(|prefix| builder.is_zero(&Count(prefix)))
                .collect()
        })
    }

    /// Exclusive prefix sums of `items`, reading at most `radix - 1` forms per level.
    fn prefix_sums(&mut self, items: &[&Affine], radix: usize) -> Result<Vec<Affine>, Refusal> {
        let radix = radix.max(2);
        let offsets = if items.len() > radix {
            let blocks = items.len().div_ceil(radix);
            let mut sums = Vec::with_capacity(blocks);
            for block in items.chunks(radix).take(blocks - 1) {
                let sum = block
                    .iter()
                    .fold(Affine::constant(0.0), |total, item| total.plus(item));
                sums.push(self.materialize(sum)?);
            }
            // No prefix reads the last block's total: a free placeholder.
            sums.push(Affine::constant(0.0));
            let sums: Vec<&Affine> = sums.iter().collect();
            self.prefix_sums(&sums, radix)?
        } else {
            vec![Affine::constant(0.0)]
        };

        let mut result = Vec::with_capacity(items.len());
        for (block, offset) in items.chunks(radix).zip(offsets) {
            let mut running = offset;
            if let Some((_, read)) = block.split_last() {
                for item in read {
                    let next = running.plus(item);
                    result.push(core::mem::replace(&mut running, next));
                }
            }
            result.push(running);
        }
        Ok(result)
    }

    /// `form` as one wire, unless it already reads at most one.
    fn materialize(&mut self, form: Affine) -> Result<Affine, Refusal> {
        if form.terms().len() <= 1 {
            return Ok(form);
        }
        Ok(Affine::wire(self.circuit.push(Op::Affine(form))?))
    }
}

/// Bits [`Builder::none_earlier`] sums inline before materializing block sums.
const PREFIX_RADIX: usize = 64;

/// The sum of a sequence of bits, normalized. Free: addition only.
fn sum_bits(bits: impl IntoIterator<Item = Bit>) -> Affine {
    bits.into_iter()
        .fold(Affine::constant(0.0), |total, bit| total.plus(&bit.0))
}

#[cfg(test)]
mod tests;
