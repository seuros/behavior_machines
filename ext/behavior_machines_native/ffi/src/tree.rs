//! `BehaviorMachines::Tree`: a compiled tree and its execution state.
//!
//! A tick runs Ruby (leaves, observers) from inside the core, so the core
//! cannot decide on a copy: the leaves' effects are real. The tree is taken
//! out of its cell for the call instead, so no borrow is held while Ruby
//! runs; a leaf that calls back into its own tree finds it gone and gets a
//! `RuntimeError`. Every answer is a Symbol, so the tree is updated in place.
use std::cell::RefCell;

use behavior_machines::Halt;
use magnus::typed_data::Obj;
use magnus::value::{Opaque, ReprValue};
use magnus::{DataTypeFunctions, Error, Integer, Ruby, Symbol, TypedData, Value, gc};
use matryona_machines::ruby::{SymbolNames, Symbols, instant, nanoseconds};

use crate::engine::Engine;
use crate::leaf::{Call, RubyObserver};
use crate::names::{Halts, Refusals, Statuses};
use crate::nodes;

/// A compiled tree. `leaves` holds every leaf object, outside the cell, so
/// the GC still sees them while a tick has the engine out.
#[derive(TypedData)]
#[magnus(class = "BehaviorMachines::Tree", free_immediately, mark)]
pub struct NativeTree {
    engine: RefCell<Option<Engine>>,
    leaves: Vec<Opaque<Value>>,
}

impl DataTypeFunctions for NativeTree {
    fn mark(&self, marker: &gc::Marker) {
        for leaf in &self.leaves {
            marker.mark(*leaf);
        }
    }
}

const REENTRANT: &str = "a leaf called back into the tree that is ticking it";

fn reentrant(ruby: &Ruby) -> Error {
    Error::new(ruby.exception_runtime_error(), REENTRANT)
}

impl NativeTree {
    /// `Tree.new(root)`: `RangeError` past the capacity, `TypeError` for
    /// anything but a node.
    pub fn new(ruby: &Ruby, root: Value) -> Result<Self, Error> {
        let definition = nodes::definition(ruby, root)?;
        let engine = Engine::new(definition.steps)
            .map_err(|refusal| Error::new(ruby.exception_range_error(), refusal.to_string()))?;
        Ok(Self {
            engine: RefCell::new(Some(engine)),
            leaves: definition.leaves,
        })
    }

    /// Takes the engine for one call, runs `call` with no borrow held, and
    /// puts it back whatever `call` answered.
    fn with_engine<R>(
        &self,
        ruby: &Ruby,
        call: impl FnOnce(&mut Engine) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let taken = self
            .engine
            .try_borrow_mut()
            .ok()
            .and_then(|mut cell| cell.take());
        let Some(mut engine) = taken else {
            return Err(reentrant(ruby));
        };
        let answer = call(&mut engine);
        if let Ok(mut cell) = self.engine.try_borrow_mut() {
            *cell = Some(engine);
        }
        answer
    }

    /// A query: the engine must be home.
    fn read<R>(&self, ruby: &Ruby, query: impl FnOnce(&Engine) -> R) -> Result<R, Error> {
        let cell = self.engine.try_borrow().map_err(|_| reentrant(ruby))?;
        cell.as_ref().map(query).ok_or_else(|| reentrant(ruby))
    }

    pub fn tick(ruby: &Ruby, this: Obj<Self>, ctx: Value, now: Value) -> Result<Symbol, Error> {
        Self::ticked(ruby, this, ctx, now, None)
    }

    pub fn tick_observed(
        ruby: &Ruby,
        this: Obj<Self>,
        ctx: Value,
        now: Value,
        observer: Value,
    ) -> Result<Symbol, Error> {
        Self::ticked(ruby, this, ctx, now, Some(observer))
    }

    /// Frozen first, then the arguments, then the core. Once a leaf or
    /// observer raises, the rest of the tick runs no Ruby; every leaf still
    /// active is then halted without reports, and the first error raises.
    fn ticked(
        ruby: &Ruby,
        this: Obj<Self>,
        ctx: Value,
        now: Value,
        observer: Option<Value>,
    ) -> Result<Symbol, Error> {
        this.check_frozen()?;
        let instant = instant(now, "now")?;
        let error = RefCell::new(None);
        let mut observer = observer
            .map(|object| RubyObserver::new(ruby, object, &error))
            .transpose()?;
        this.with_engine(ruby, |engine| {
            let call = &mut Call {
                ruby,
                ctx,
                now,
                error: &error,
            };
            let answer = match &mut observer {
                Some(observer) => engine.tick(call, instant, observer),
                None => engine.tick(call, instant, &mut ()),
            };
            if let Some(raised) = error.take() {
                let cleanup = RefCell::new(None);
                let call = &mut Call {
                    error: &cleanup,
                    ..*call
                };
                match engine.halt(call, &mut ()) {
                    Halt::Halted | Halt::AlreadyIdle => {}
                }
                return Err(raised);
            }
            Ok(match answer {
                Ok(status) => Statuses::symbol(ruby, status),
                Err(refusal) => Refusals::symbol(ruby, &refusal),
            })
        })
    }

    pub fn halt(ruby: &Ruby, this: Obj<Self>, ctx: Value) -> Result<Symbol, Error> {
        Self::halted(ruby, this, ctx, None)
    }

    pub fn halt_observed(
        ruby: &Ruby,
        this: Obj<Self>,
        ctx: Value,
        observer: Value,
    ) -> Result<Symbol, Error> {
        Self::halted(ruby, this, ctx, Some(observer))
    }

    /// Every active leaf is halted even if a leaf's halt or the observer
    /// raises; the first error raises afterwards.
    fn halted(
        ruby: &Ruby,
        this: Obj<Self>,
        ctx: Value,
        observer: Option<Value>,
    ) -> Result<Symbol, Error> {
        this.check_frozen()?;
        let error = RefCell::new(None);
        let mut observer = observer
            .map(|object| RubyObserver::new(ruby, object, &error))
            .transpose()?;
        let now = ruby.qnil().as_value();
        this.with_engine(ruby, |engine| {
            let call = &mut Call {
                ruby,
                ctx,
                now,
                error: &error,
            };
            let halt = match &mut observer {
                Some(observer) => engine.halt(call, observer),
                None => engine.halt(call, &mut ()),
            };
            error
                .take()
                .map_or_else(|| Ok(Halts::symbol(ruby, halt)), Err)
        })
    }

    pub fn is_running(ruby: &Ruby, this: Obj<Self>) -> Result<bool, Error> {
        this.read(ruby, Engine::is_running)
    }

    pub fn next_deadline(ruby: &Ruby, this: Obj<Self>) -> Result<Option<Integer>, Error> {
        let deadline = this.read(ruby, Engine::next_deadline)?;
        Ok(deadline.map(|deadline| nanoseconds(ruby, deadline)))
    }

    pub fn size(ruby: &Ruby, this: Obj<Self>) -> Result<usize, Error> {
        this.read(ruby, Engine::len)
    }
}
