//! MOCKED: scripted leaves and a recording context for tree tests.
//!
//! The context also enforces the halt contract on every test: halting a leaf
//! that is not active (never started, already finished, or already halted)
//! panics immediately.

use core::time::Duration;
use std::collections::{HashMap, HashSet};

use crate::{Halt, Leaf, Node, Observer, Status, Threshold, Tree};

/// The capacity every test tree is built with.
pub const CAPACITY: usize = 64;

/// A test tree.
pub type TestTree<L> = Tree<L, CAPACITY>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Tick(&'static str),
    Halt(&'static str),
}

/// A leaf whose result is whatever the test scripted for its name; unscripted
/// leaves keep running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scripted(pub &'static str);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    pub log: Vec<Event>,
    outcomes: HashMap<&'static str, Status>,
    active: HashSet<&'static str>,
}

impl Script {
    /// Makes every later tick of `name` return `status`.
    pub fn set(&mut self, name: &'static str, status: Status) {
        self.outcomes.insert(name, status);
    }

    pub fn count(&self, event: Event) -> usize {
        self.log.iter().filter(|logged| **logged == event).count()
    }

    pub fn position(&self, event: Event) -> Option<usize> {
        self.log.iter().position(|logged| *logged == event)
    }

    /// Asserts that no leaf is left active, i.e. none leaked an activation.
    pub fn assert_idle(&self) {
        assert!(
            self.active.is_empty(),
            "leaves left active: {:?}",
            self.active
        );
    }
}

impl Leaf<Script> for Scripted {
    fn tick(&self, script: &mut Script, _now: Duration) -> Status {
        script.log.push(Event::Tick(self.0));
        let status = script
            .outcomes
            .get(self.0)
            .copied()
            .unwrap_or(Status::Running);
        if status == Status::Running {
            script.active.insert(self.0);
        } else {
            script.active.remove(self.0);
        }
        status
    }

    fn halt(&self, script: &mut Script) {
        assert!(
            script.active.remove(self.0),
            "halted inactive leaf `{}`",
            self.0
        );
        script.log.push(Event::Halt(self.0));
    }
}

pub const fn leaf(name: &'static str) -> Node<Scripted> {
    Node::leaf(Scripted(name))
}

pub fn tree(root: Node<Scripted>) -> TestTree<Scripted> {
    Tree::new(root).unwrap()
}

/// `ms` milliseconds after the clock origin.
pub const fn at(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// Ticks that the test expects honoured: a refusal fails the test.
pub trait Drive<C: ?Sized> {
    /// [`Tree::tick`], honoured.
    fn run(&mut self, ctx: &mut C, now: Duration) -> Status;

    /// [`Tree::tick_observed`], honoured.
    fn run_observed(
        &mut self,
        ctx: &mut C,
        now: Duration,
        observer: &mut (impl Observer + ?Sized),
    ) -> Status;

    /// [`Tree::halt`] for cleanup, whatever it answers; the halt tests
    /// check the answer itself.
    fn stop(&mut self, ctx: &mut C);

    /// [`Tree::halt_observed`] for cleanup, whatever it answers.
    fn stop_observed(&mut self, ctx: &mut C, observer: &mut (impl Observer + ?Sized));
}

impl<L: Leaf<C>, C: ?Sized, const N: usize> Drive<C> for Tree<L, N> {
    fn run(&mut self, ctx: &mut C, now: Duration) -> Status {
        self.tick(ctx, now).expect("the tick is honoured")
    }

    fn run_observed(
        &mut self,
        ctx: &mut C,
        now: Duration,
        observer: &mut (impl Observer + ?Sized),
    ) -> Status {
        self.tick_observed(ctx, now, observer)
            .expect("the tick is honoured")
    }

    fn stop(&mut self, ctx: &mut C) {
        match self.halt(ctx) {
            Halt::Halted | Halt::AlreadyIdle => {}
        }
    }

    fn stop_observed(&mut self, ctx: &mut C, observer: &mut (impl Observer + ?Sized)) {
        match self.halt_observed(ctx, observer) {
            Halt::Halted | Halt::AlreadyIdle => {}
        }
    }
}

/// `Parallel` that needs every child to succeed and fails on the first failure.
pub fn parallel_all<L>(children: impl IntoIterator<Item = Node<L>>) -> Node<L> {
    Node::parallel(children, Threshold::All, Threshold::Count(1))
}

/// `Parallel` that succeeds on the first success and fails once all fail.
pub fn parallel_any<L>(children: impl IntoIterator<Item = Node<L>>) -> Node<L> {
    Node::parallel(children, Threshold::Count(1), Threshold::All)
}

/// `Parallel` decided by the first child to finish either way.
pub fn race<L>(children: impl IntoIterator<Item = Node<L>>) -> Node<L> {
    Node::parallel(children, Threshold::Count(1), Threshold::Count(1))
}
