//! Ruby leaves and observers, called from inside a core tick.
use std::cell::{Cell, RefCell};
use std::time::Duration;

use behavior_machines::{Leaf, NodeId, Observer, Status};
use magnus::value::{Opaque, ReprValue};
use magnus::{Error, Ruby, Value};
use matryona_machines::ruby::Symbols;

use crate::names::Statuses;

/// A Ruby leaf object; `halts` caches `respond_to?(:halt)`. `active` mirrors
/// the leaf's activation, so a tick skipped after an error can keep an active
/// leaf running (cleanup then halts it) instead of ending it unhalted.
pub struct RubyLeaf {
    object: Opaque<Value>,
    halts: bool,
    active: Cell<bool>,
}

impl RubyLeaf {
    /// Wraps `object`, asking once whether it responds to `halt`.
    pub fn new(object: Value) -> Result<Self, Error> {
        Ok(Self {
            object: object.into(),
            halts: object.respond_to("halt", false)?,
            active: Cell::new(false),
        })
    }
}

/// The tree context for one call: the Ruby context, the tick's `now` as the
/// Integer the caller passed, and the first error a leaf or observer raised,
/// shared with [`RubyObserver`].
pub struct Call<'a> {
    pub ruby: &'a Ruby,
    pub ctx: Value,
    pub now: Value,
    pub error: &'a RefCell<Option<Error>>,
}

impl Call<'_> {
    fn failed(&self) -> bool {
        self.error.borrow().is_some()
    }

    fn record(&self, error: Error) {
        self.error.borrow_mut().get_or_insert(error);
    }
}

impl<'a> Leaf<Call<'a>> for RubyLeaf {
    fn tick(&self, call: &mut Call<'a>, _now: Duration) -> Status {
        // After an error no more Ruby runs this tick: an active leaf stays
        // active and an idle one does not start.
        if call.failed() {
            return if self.active.get() {
                Status::Running
            } else {
                Status::Failure
            };
        }
        let object = call.ruby.get_inner(self.object);
        let status = object
            .funcall::<_, _, Value>("tick", (call.ctx, call.now))
            // TypeError for a non-Symbol, RangeError for any other Symbol,
            // one whose name is not UTF-8 included.
            .and_then(|status| Statuses::parse(call.ruby, status))
            .unwrap_or_else(|error| {
                // The raising leaf counts as finished.
                call.record(error);
                Status::Failure
            });
        self.active.set(status == Status::Running);
        status
    }

    fn halt(&self, call: &mut Call<'a>) {
        self.active.set(false);
        if !self.halts {
            return;
        }
        let object = call.ruby.get_inner(self.object);
        if let Err(error) = object.funcall::<_, _, Value>("halt", (call.ctx,)) {
            // Every active leaf is still halted; the first error is the one
            // reported.
            call.record(error);
        }
    }
}

/// Forwards reports to a Ruby observer, for the hooks it implements, until
/// the first error.
pub struct RubyObserver<'a> {
    ruby: &'a Ruby,
    object: Value,
    ticked: bool,
    halted: bool,
    error: &'a RefCell<Option<Error>>,
}

impl<'a> RubyObserver<'a> {
    /// Asks the observer once which hooks it implements. Runs Ruby, so it is
    /// called before the tree is taken for the call.
    pub fn new(
        ruby: &'a Ruby,
        object: Value,
        error: &'a RefCell<Option<Error>>,
    ) -> Result<Self, Error> {
        Ok(Self {
            ruby,
            object,
            ticked: object.respond_to("ticked", false)?,
            halted: object.respond_to("halted", false)?,
            error,
        })
    }

    fn report<A: magnus::ArgList>(&self, hook: &str, args: A) {
        if self.error.borrow().is_some() {
            return;
        }
        if let Err(error) = self.object.funcall::<_, _, Value>(hook, args) {
            self.error.borrow_mut().get_or_insert(error);
        }
    }
}

impl Observer for RubyObserver<'_> {
    fn ticked(&mut self, node: NodeId, status: Status) {
        if self.ticked {
            let status = Statuses::symbol(self.ruby, status);
            self.report("ticked", (node.index(), status));
        }
    }

    fn halted(&mut self, node: NodeId) {
        if self.halted {
            self.report("halted", (node.index(),));
        }
    }
}
