//! Ruby bindings for `behavior-machines`: the optional native backend of the
//! `behavior_machines` gem.
//!
//! `BehaviorMachinesNative::Tree.new(spec)` builds the nested-Array spec that
//! `BehaviorMachines::Definition` lowers a node definition to, and runs it on
//! the Rust engine, calling back into Ruby leaves and observers. The Ruby
//! `BehaviorMachines::Tree` wrapper validates input and guards re-entrancy;
//! this layer mirrors `BehaviorMachines::Engine`, including its error rules:
//! once a leaf or observer raises, no further Ruby runs for that tick, every
//! leaf still active is halted without observer reports, and the error
//! propagates.

use std::borrow::Cow;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::time::Duration;

use behavior_machines::{Leaf, Limit, Node, NodeId, Observer, Status, Threshold, Time, Tree};
use magnus::value::{Opaque, ReprValue};
use magnus::{
    DataTypeFunctions, Error, ExceptionClass, Module, Object, RArray, RModule, Ruby, Symbol,
    TypedData, Value, function, gc, method,
};

const SUCCESS: &str = "success";
const FAILURE: &str = "failure";
const RUNNING: &str = "running";

/// The status Symbols `BehaviorMachines::STATUSES` names, in the same order.
const STATUSES: [(&str, Status); 3] = [
    (SUCCESS, Status::Success),
    (FAILURE, Status::Failure),
    (RUNNING, Status::Running),
];

/// A Ruby leaf object; `halts` caches `respond_to?(:halt)`. `active` mirrors
/// the leaf's activation, so a tick skipped after an error can keep an active
/// leaf running (cleanup then halts it) instead of ending it unhalted.
struct RubyLeaf {
    object: Opaque<Value>,
    halts: bool,
    active: Cell<bool>,
}

/// The tree context for one call: the Ruby context and the first error a
/// leaf or observer raised, shared with [`RubyObserver`].
struct Call<'a> {
    ruby: &'a Ruby,
    ctx: Value,
    error: &'a RefCell<Option<Error>>,
}

impl<'a> Leaf<Call<'a>> for RubyLeaf {
    fn tick(&self, call: &mut Call<'a>, now: Time) -> Status {
        // After an error no more Ruby runs this tick: an active leaf stays
        // active and an idle one does not start, as in the Ruby engine.
        if call.error.borrow().is_some() {
            return if self.active.get() {
                Status::Running
            } else {
                Status::Failure
            };
        }
        let object = call.ruby.get_inner(self.object);
        let status = object
            .funcall::<_, _, Value>("tick", (call.ctx, now.as_nanos()))
            .and_then(|status| status_of(call.ruby, object, status))
            .unwrap_or_else(|error| {
                // The raising leaf counts as finished.
                *call.error.borrow_mut() = Some(error);
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
            // A halt that raises leaves the tree unspecified (documented on
            // BehaviorMachines::Tree); the first error is the one reported.
            call.error.borrow_mut().get_or_insert(error);
        }
    }
}

/// Forwards reports to a Ruby observer until the first error.
struct RubyObserver<'a> {
    ruby: &'a Ruby,
    observer: Option<Value>,
    error: &'a RefCell<Option<Error>>,
}

impl RubyObserver<'_> {
    fn report<A>(&self, hook: &str, args: A)
    where
        A: magnus::ArgList,
    {
        let Some(observer) = self.observer else {
            return;
        };
        if self.error.borrow().is_some() {
            return;
        }
        if let Err(error) = observer.funcall::<_, _, Value>(hook, args) {
            *self.error.borrow_mut() = Some(error);
        }
    }
}

impl Observer for RubyObserver<'_> {
    fn ticked(&mut self, node: NodeId, status: Status) {
        self.report("ticked", (node.index(), status_symbol(self.ruby, status)));
    }

    fn halted(&mut self, node: NodeId) {
        self.report("halted", (node.index(),));
    }
}

/// A compiled tree. `leaves` holds every leaf object so the GC keeps them.
#[derive(TypedData)]
#[magnus(class = "BehaviorMachinesNative::Tree", free_immediately, mark)]
struct NativeTree {
    tree: RefCell<Tree<RubyLeaf>>,
    leaves: Vec<Opaque<Value>>,
}

impl DataTypeFunctions for NativeTree {
    fn mark(&self, marker: &gc::Marker) {
        for leaf in &self.leaves {
            marker.mark(*leaf);
        }
    }
}

impl NativeTree {
    fn new(ruby: &Ruby, spec: RArray) -> Result<Self, Error> {
        let mut leaves = Vec::new();
        let root = node_from(ruby, spec, &mut leaves)?;
        // Definition already validated the spec, so this only fires if the
        // two sides drift apart.
        let tree = Tree::new(root)
            .map_err(|error| Error::new(ruby.exception_arg_error(), error.to_string()))?;
        Ok(Self {
            tree: RefCell::new(tree),
            leaves,
        })
    }

    /// A leaf calling back into its own tree finds it already borrowed.
    fn borrow_mut(&self, ruby: &Ruby) -> Result<RefMut<'_, Tree<RubyLeaf>>, Error> {
        self.tree.try_borrow_mut().map_err(|_| reentrant(ruby))
    }

    fn borrow(&self, ruby: &Ruby) -> Result<Ref<'_, Tree<RubyLeaf>>, Error> {
        self.tree.try_borrow().map_err(|_| reentrant(ruby))
    }

    fn tick(
        ruby: &Ruby,
        rb_self: &Self,
        ctx: Value,
        now: u64,
        observer: Option<Value>,
    ) -> Result<Symbol, Error> {
        let mut tree = rb_self.borrow_mut(ruby)?;
        let error = RefCell::new(None);
        let status = tree.tick_observed(
            &mut Call {
                ruby,
                ctx,
                error: &error,
            },
            Time::from_nanos(now),
            &mut RubyObserver {
                ruby,
                observer,
                error: &error,
            },
        );
        if let Some(error) = error.take() {
            // Cleanup halts that raise leave the tree unspecified
            // (documented); the original error is the one reported.
            let cleanup = RefCell::new(None);
            tree.halt(&mut Call {
                ruby,
                ctx,
                error: &cleanup,
            });
            return Err(error);
        }
        Ok(status_symbol(ruby, status))
    }

    fn halt(ruby: &Ruby, rb_self: &Self, ctx: Value, observer: Option<Value>) -> Result<(), Error> {
        let mut tree = rb_self.borrow_mut(ruby)?;
        let error = RefCell::new(None);
        tree.halt_observed(
            &mut Call {
                ruby,
                ctx,
                error: &error,
            },
            &mut RubyObserver {
                ruby,
                observer,
                error: &error,
            },
        );
        error.take().map_or(Ok(()), Err)
    }

    fn is_running(ruby: &Ruby, rb_self: &Self) -> Result<bool, Error> {
        Ok(rb_self.borrow(ruby)?.is_running())
    }

    fn next_deadline(ruby: &Ruby, rb_self: &Self) -> Result<Option<u64>, Error> {
        Ok(rb_self.borrow(ruby)?.next_deadline().map(Time::as_nanos))
    }
}

/// Builds the node a spec describes. Specs are `[kind, *args]`, as produced
/// by `BehaviorMachines::Definition.compile`.
fn node_from(
    ruby: &Ruby,
    spec: RArray,
    leaves: &mut Vec<Opaque<Value>>,
) -> Result<Node<RubyLeaf>, Error> {
    let kind: Symbol = spec.entry(0)?;
    let node = match name(kind).as_deref() {
        Some("leaf") => {
            let object: Value = spec.entry(1)?;
            leaves.push(object.into());
            Node::Leaf(RubyLeaf {
                object: object.into(),
                halts: spec.entry(2)?,
                active: Cell::new(false),
            })
        }
        Some("constant") if is_success(ruby, spec.entry(1)?) => Node::AlwaysSuccess,
        Some("constant") => Node::AlwaysFailure,
        Some("sleep") => Node::Sleep(Duration::from_nanos(spec.entry(1)?)),
        Some("ordered") => {
            let children = children_from(ruby, spec.entry(1)?, leaves)?;
            match (is_success(ruby, spec.entry(2)?), spec.entry::<bool>(3)?) {
                (true, false) => Node::Sequence(children),
                (true, true) => Node::ReactiveSequence(children),
                (false, false) => Node::Fallback(children),
                (false, true) => Node::ReactiveFallback(children),
            }
        }
        Some("parallel") => Node::Parallel {
            children: children_from(ruby, spec.entry(1)?, leaves)?,
            success: Threshold::Count(spec.entry(2)?),
            failure: Threshold::Count(spec.entry(3)?),
        },
        Some("conditional") => {
            let condition = node_from(ruby, spec.entry(1)?, leaves)?;
            let branch = node_from(ruby, spec.entry(2)?, leaves)?;
            let otherwise = match spec.entry::<Option<RArray>>(3)? {
                Some(otherwise) => Some(node_from(ruby, otherwise, leaves)?),
                None => None,
            };
            match (spec.entry::<bool>(4)?, otherwise) {
                (false, Some(otherwise)) => Node::if_then_else(condition, branch, otherwise),
                (false, None) => Node::if_then(condition, branch),
                (true, Some(otherwise)) => Node::while_do_else(condition, branch, otherwise),
                (true, None) => Node::while_do(condition, branch),
            }
        }
        Some("map") => {
            let child = node_from(ruby, spec.entry(1)?, leaves)?;
            let map: Symbol = spec.entry(2)?;
            match name(map).as_deref() {
                Some("inverter") => child.inverter(),
                Some("force_success") => child.force_success(),
                Some("force_failure") => child.force_failure(),
                _ => return Err(unknown(ruby, "map", map)),
            }
        }
        Some("timeout") => {
            node_from(ruby, spec.entry(2)?, leaves)?.timeout(Duration::from_nanos(spec.entry(1)?))
        }
        Some("loop") => {
            let child = node_from(ruby, spec.entry(1)?, leaves)?;
            let limit = spec
                .entry::<Option<usize>>(3)?
                .map_or(Limit::Unlimited, Limit::Times);
            if is_success(ruby, spec.entry(2)?) {
                child.repeat(limit)
            } else {
                child.retry_until_successful(limit)
            }
        }
        _ => return Err(unknown(ruby, "node", kind)),
    };
    Ok(node)
}

fn children_from(
    ruby: &Ruby,
    specs: RArray,
    leaves: &mut Vec<Opaque<Value>>,
) -> Result<Vec<Node<RubyLeaf>>, Error> {
    (0..specs.len())
        .map(|index| node_from(ruby, specs.entry(index.cast_signed())?, leaves))
        .collect()
}

fn name(symbol: Symbol) -> Option<Cow<'static, str>> {
    symbol.name().ok()
}

fn is(ruby: &Ruby, symbol: Symbol, name: &str) -> bool {
    symbol == ruby.sym_new(name)
}

fn is_success(ruby: &Ruby, symbol: Symbol) -> bool {
    is(ruby, symbol, SUCCESS)
}

/// Accepts exactly the three status Symbols, like `BehaviorMachines.check_status`.
fn status_of(ruby: &Ruby, leaf: Value, status: Value) -> Result<Status, Error> {
    if let Some(symbol) = Symbol::from_value(status) {
        for (name, known) in STATUSES {
            if is(ruby, symbol, name) {
                return Ok(known);
            }
        }
    }
    // Through Ruby, so an `inspect` that raises propagates as in the Ruby engine.
    let leaf = leaf.funcall::<_, _, String>("inspect", ())?;
    let status = status.funcall::<_, _, String>("inspect", ())?;
    let message = format!("{leaf} returned {status}; expected :success, :failure, or :running");
    Err(Error::new(
        library_error(ruby, "InvalidStatusError")?,
        message,
    ))
}

fn status_symbol(ruby: &Ruby, status: Status) -> Symbol {
    ruby.to_symbol(match status {
        Status::Success => SUCCESS,
        Status::Failure => FAILURE,
        Status::Running => RUNNING,
    })
}

fn library_error(ruby: &Ruby, class: &str) -> Result<ExceptionClass, Error> {
    ruby.class_object()
        .const_get::<_, RModule>("BehaviorMachines")?
        .const_get(class)
}

fn reentrant(ruby: &Ruby) -> Error {
    let message = "a leaf called back into the tree that is ticking it";
    match library_error(ruby, "ReentrantCallError") {
        Ok(class) => Error::new(class, message),
        Err(error) => error,
    }
}

fn unknown(ruby: &Ruby, what: &str, spec: Symbol) -> Error {
    Error::new(
        ruby.exception_arg_error(),
        format!("unknown {what} spec {}", spec.inspect()),
    )
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let class = ruby
        .define_module("BehaviorMachinesNative")?
        .define_class("Tree", ruby.class_object())?;
    class.define_singleton_method("new", function!(NativeTree::new, 1))?;
    class.define_method("tick", method!(NativeTree::tick, 3))?;
    class.define_method("halt", method!(NativeTree::halt, 2))?;
    class.define_method("running?", method!(NativeTree::is_running, 0))?;
    class.define_method("next_deadline", method!(NativeTree::next_deadline, 0))?;
    Ok(())
}
