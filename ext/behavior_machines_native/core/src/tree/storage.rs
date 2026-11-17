//! Where a compiled tree keeps its nodes.

#[cfg(any(feature = "alloc", test))]
use alloc::vec::Vec;

use super::slot::Slot;
use crate::Refusal;

/// Where a [`Behavior`](crate::Behavior) keeps its compiled nodes and leaves.
///
/// [`Inline`] holds a fixed capacity in place, and
#[cfg_attr(feature = "alloc", doc = " [`Heap`] (feature `alloc`)")]
#[cfg_attr(not(feature = "alloc"), doc = " `Heap` (feature `alloc`)")]
/// grows to fit its definition. Sealed: the tree semantics
/// never depend on which one holds the nodes. Its leaf type is `S::Leaf`.
pub trait Storage: sealed::Sealed {}

/// At most `N` nodes held inline: building and ticking allocate nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline<L, const N: usize> {
    slots: [Slot; N],
    leaves: [Option<L>; N],
    len: usize,
}

/// Nodes on the heap, as many as the definition has (feature `alloc`). Only
/// building allocates; ticking and halting do not.
#[cfg(any(feature = "alloc", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heap<L> {
    slots: Vec<Slot>,
    leaves: Vec<Option<L>>,
}

impl<L, const N: usize> Storage for Inline<L, N> {}

#[cfg(any(feature = "alloc", test))]
impl<L> Storage for Heap<L> {}

pub(super) mod sealed {
    use super::{Inline, Refusal, Slot};

    pub trait Sealed: Sized {
        /// The leaf type the tree runs.
        type Leaf;
        /// The indices of nodes still waiting for children while building.
        type Open: Stack;

        fn empty() -> Self;

        /// Appends a node and answers its index, or [`Refusal::Full`].
        fn push(&mut self, slot: Slot, leaf: Option<Self::Leaf>) -> Result<usize, Refusal>;

        fn slots(&self) -> &[Slot];

        fn slot_mut(&mut self, at: usize) -> Option<&mut Slot>;

        fn parts(&mut self) -> (&mut [Slot], &[Option<Self::Leaf>]);
    }

    pub trait Stack {
        fn empty() -> Self;

        fn enter(&mut self, at: usize) -> Result<(), Refusal>;

        fn innermost(&self) -> Option<usize>;

        fn leave(&mut self);

        fn none_open(&self) -> bool;
    }

    impl<L, const N: usize> Sealed for Inline<L, N> {
        type Leaf = L;
        type Open = Indices<N>;

        fn empty() -> Self {
            const { assert!(N > 0, "a tree needs room for at least its root") };
            Self {
                slots: [Slot::idle(); N],
                leaves: core::array::from_fn(|_| None),
                len: 0,
            }
        }

        fn push(&mut self, slot: Slot, leaf: Option<Self::Leaf>) -> Result<usize, Refusal> {
            let at = self.len;
            let (Some(slot_cell), Some(leaf_cell)) =
                (self.slots.get_mut(at), self.leaves.get_mut(at))
            else {
                return Err(Refusal::Full);
            };
            *slot_cell = slot;
            *leaf_cell = leaf;
            self.len += 1;
            Ok(at)
        }

        fn slots(&self) -> &[Slot] {
            self.slots.get(..self.len).unwrap_or_default()
        }

        fn slot_mut(&mut self, at: usize) -> Option<&mut Slot> {
            self.slots.get_mut(..self.len)?.get_mut(at)
        }

        fn parts(&mut self) -> (&mut [Slot], &[Option<Self::Leaf>]) {
            (
                self.slots.get_mut(..self.len).unwrap_or_default(),
                &self.leaves,
            )
        }
    }

    /// At most `N` open indices, inline.
    pub struct Indices<const N: usize> {
        items: [usize; N],
        len: usize,
    }

    impl<const N: usize> Stack for Indices<N> {
        fn empty() -> Self {
            Self {
                items: [0; N],
                len: 0,
            }
        }

        fn enter(&mut self, at: usize) -> Result<(), Refusal> {
            let cell = self.items.get_mut(self.len).ok_or(Refusal::Full)?;
            *cell = at;
            self.len += 1;
            Ok(())
        }

        fn innermost(&self) -> Option<usize> {
            self.items.get(self.len.checked_sub(1)?).copied()
        }

        fn leave(&mut self) {
            self.len = self.len.saturating_sub(1);
        }

        fn none_open(&self) -> bool {
            self.len == 0
        }
    }

    #[cfg(any(feature = "alloc", test))]
    mod heap {
        use alloc::vec::Vec;

        use super::super::Heap;
        use super::{Refusal, Sealed, Slot, Stack};

        impl<L> Sealed for Heap<L> {
            type Leaf = L;
            type Open = Vec<usize>;

            fn empty() -> Self {
                Self {
                    slots: Vec::new(),
                    leaves: Vec::new(),
                }
            }

            fn push(&mut self, slot: Slot, leaf: Option<Self::Leaf>) -> Result<usize, Refusal> {
                self.slots.try_reserve(1).map_err(|_| Refusal::Full)?;
                self.leaves.try_reserve(1).map_err(|_| Refusal::Full)?;
                let at = self.slots.len();
                self.slots.push(slot);
                self.leaves.push(leaf);
                Ok(at)
            }

            fn slots(&self) -> &[Slot] {
                &self.slots
            }

            fn slot_mut(&mut self, at: usize) -> Option<&mut Slot> {
                self.slots.get_mut(at)
            }

            fn parts(&mut self) -> (&mut [Slot], &[Option<Self::Leaf>]) {
                (&mut self.slots, &self.leaves)
            }
        }

        impl Stack for Vec<usize> {
            fn empty() -> Self {
                Self::new()
            }

            fn enter(&mut self, at: usize) -> Result<(), Refusal> {
                self.try_reserve(1).map_err(|_| Refusal::Full)?;
                self.push(at);
                Ok(())
            }

            fn innermost(&self) -> Option<usize> {
                self.last().copied()
            }

            fn leave(&mut self) {
                self.truncate(self.len().saturating_sub(1));
            }

            fn none_open(&self) -> bool {
                self.is_empty()
            }
        }
    }
}
