//! The core tree at the smallest capacity that holds a definition.
use std::time::Duration;

use behavior_machines::{Halt, Observer, Refusal, Status, Step, Tree};

use crate::leaf::{Call, RubyLeaf};
use crate::nodes::CAPACITY;

/// Most trees are small; a small one should not carry room for the largest.
const SMALL: usize = 32;

/// The tree, boxed: the core holds every node inline, and the Ruby object
/// holding it should stay small.
pub enum Engine {
    Small(Box<Tree<RubyLeaf, SMALL>>),
    Large(Box<Tree<RubyLeaf, CAPACITY>>),
}

macro_rules! each {
    ($engine:expr, $tree:ident => $body:expr) => {
        match $engine {
            Engine::Small($tree) => $body,
            Engine::Large($tree) => $body,
        }
    };
}

impl Engine {
    pub fn new(steps: Vec<Step<RubyLeaf>>) -> Result<Self, Refusal> {
        if steps.len() <= SMALL {
            Ok(Self::Small(Box::new(Tree::new(steps)?)))
        } else {
            Ok(Self::Large(Box::new(Tree::new(steps)?)))
        }
    }

    pub fn tick(
        &mut self,
        call: &mut Call<'_>,
        now: Duration,
        observer: &mut (impl Observer + ?Sized),
    ) -> Result<Status, Refusal> {
        each!(self, tree => tree.tick_observed(call, now, observer))
    }

    pub fn halt(&mut self, call: &mut Call<'_>, observer: &mut (impl Observer + ?Sized)) -> Halt {
        each!(self, tree => tree.halt_observed(call, observer))
    }

    pub fn is_running(&self) -> bool {
        each!(self, tree => tree.is_running())
    }

    pub fn next_deadline(&self) -> Option<Duration> {
        each!(self, tree => tree.next_deadline())
    }

    pub const fn len(&self) -> usize {
        each!(self, tree => tree.len())
    }
}
