//! Node values: validated when built, and walked into the core's steps.
//!
//! Both paths read a node through [`read`], which converts every field and
//! asks the core to validate the step, so a value built by hand and a value
//! forged past `initialize` are held to the same rule.
use behavior_machines::{Limit, Otherwise, Refusal, Step, Threshold};
use magnus::value::{Opaque, ReprValue};
use magnus::{Error, RArray, RClass, Ruby, Symbol, Value};
use matryona_machines::ruby::{exact_array, instant, integer, which, which_struct_with};

use crate::classes::{ALL_KINDS, KINDS, Kind, nodes};
use crate::leaf::RubyLeaf;

/// The most nodes a Ruby tree may have: the largest capacity the binding
/// builds.
pub const CAPACITY: usize = 256;

/// The most fields a node class has.
const FIELDS: usize = 3;

/// A node's fields or fixed children: up to three Values, kept on the stack
/// where the GC sees them.
#[derive(Clone, Copy)]
struct Few {
    values: [Option<Value>; FIELDS],
}

impl Few {
    const fn new() -> Self {
        Self {
            values: [None; FIELDS],
        }
    }

    fn of(values: &[Value]) -> Self {
        let mut few = Self::new();
        for (cell, value) in few.values.iter_mut().zip(values) {
            *cell = Some(*value);
        }
        few
    }

    fn iter(self) -> impl Iterator<Item = Value> {
        self.values.into_iter().flatten()
    }
}

/// A node's children, as its fields hold them.
enum Children {
    /// A composite's `children` Array, the node's first field.
    List(RArray),
    /// A conditional's or decorator's child fields, in order.
    Fixed(Few),
}

/// A node read from Ruby: its step, whose leaf is the leaf object, and its
/// children.
struct Read {
    step: Step<Value>,
    children: Children,
}

fn range_error(ruby: &Ruby, refusal: Refusal) -> Error {
    Error::new(ruby.exception_range_error(), refusal.to_string())
}

fn type_error(ruby: &Ruby, message: &'static str) -> Error {
    Error::new(ruby.exception_type_error(), message)
}

/// The node classes `value` is exactly one of.
fn kind_of(ruby: &Ruby, classes: &[RClass; KINDS], value: Value) -> Result<Kind, Error> {
    which(value, classes)
        .and_then(|index| ALL_KINDS.get(index).copied())
        .ok_or_else(|| type_error(ruby, "a child must be a BehaviorMachines::Node value"))
}

/// `:all` or an Integer child count.
fn threshold(ruby: &Ruby, value: Value, name: &str) -> Result<Threshold, Error> {
    if let Some(symbol) = Symbol::from_value(value) {
        return if symbol == ruby.to_symbol("all") {
            Ok(Threshold::All)
        } else {
            let message = format!("{name} must be :all or an Integer");
            Err(Error::new(ruby.exception_range_error(), message))
        };
    }
    integer(value, name).map(Threshold::Count)
}

/// `:unlimited` or an Integer run count.
fn limit(ruby: &Ruby, value: Value) -> Result<Limit, Error> {
    if let Some(symbol) = Symbol::from_value(value) {
        return if symbol == ruby.to_symbol("unlimited") {
            Ok(Limit::Unlimited)
        } else {
            let message = "limit must be :unlimited or an Integer";
            Err(Error::new(ruby.exception_range_error(), message))
        };
    }
    integer(value, "limit").map(Limit::Times)
}

/// A composite's children Array, exactly an Array.
fn list(ruby: &Ruby, value: Value) -> Result<RArray, Error> {
    exact_array(ruby, value, "children must be an Array")
}

/// A leaf object, which must respond to `tick(ctx, now)`. Asked on every
/// read, so a leaf swapped in past `initialize` (by `Marshal.load`) is
/// refused when the tree is built, not when it first ticks.
fn leaf(ruby: &Ruby, value: Value) -> Result<Value, Error> {
    if value.respond_to("tick", false)? {
        Ok(value)
    } else {
        Err(type_error(ruby, "a leaf must respond to tick(ctx, now)"))
    }
}

fn otherwise(value: Value) -> (Otherwise, Option<Value>) {
    if value.is_nil() {
        (Otherwise::Absent, None)
    } else {
        (Otherwise::Present, Some(value))
    }
}

/// Converts `fields` (exactly `kind.arity()` of them) into the node's step
/// and children, and validates the step through the core.
fn read(ruby: &Ruby, kind: Kind, fields: &[Value]) -> Result<Read, Error> {
    let at = |index: usize| {
        fields
            .get(index)
            .copied()
            .unwrap_or_else(|| ruby.qnil().as_value())
    };
    let composite = |make: fn(usize) -> Step<Value>| -> Result<Read, Error> {
        let children = list(ruby, at(0))?;
        Ok(Read {
            step: make(children.len()),
            children: Children::List(children),
        })
    };
    let fixed = |step, children| Read {
        step,
        children: Children::Fixed(children),
    };
    let conditional = |make: fn(Otherwise) -> Step<Value>| {
        let (present, branch) = otherwise(at(2));
        let children = branch.map_or_else(
            || Few::of(&[at(0), at(1)]),
            |branch| Few::of(&[at(0), at(1), branch]),
        );
        fixed(make(present), children)
    };
    let read = match kind {
        Kind::Leaf => fixed(Step::Leaf(leaf(ruby, at(0))?), Few::new()),
        Kind::AlwaysSuccess => fixed(Step::AlwaysSuccess, Few::new()),
        Kind::AlwaysFailure => fixed(Step::AlwaysFailure, Few::new()),
        Kind::Sleep => fixed(Step::Sleep(instant(at(0), "duration")?), Few::new()),
        Kind::Sequence => composite(Step::Sequence)?,
        Kind::ReactiveSequence => composite(Step::ReactiveSequence)?,
        Kind::Fallback => composite(Step::Fallback)?,
        Kind::ReactiveFallback => composite(Step::ReactiveFallback)?,
        Kind::Parallel => {
            let children = list(ruby, at(0))?;
            let step = Step::Parallel {
                children: children.len(),
                success: threshold(ruby, at(1), "success")?,
                failure: threshold(ruby, at(2), "failure")?,
            };
            Read {
                step,
                children: Children::List(children),
            }
        }
        Kind::IfThenElse => conditional(Step::IfThenElse),
        Kind::WhileDoElse => conditional(Step::WhileDoElse),
        Kind::Inverter => fixed(Step::Inverter, Few::of(&[at(0)])),
        Kind::ForceSuccess => fixed(Step::ForceSuccess, Few::of(&[at(0)])),
        Kind::ForceFailure => fixed(Step::ForceFailure, Few::of(&[at(0)])),
        Kind::KeepRunningUntilFailure => fixed(Step::KeepRunningUntilFailure, Few::of(&[at(0)])),
        Kind::Timeout => fixed(
            Step::Timeout(instant(at(0), "duration")?),
            Few::of(&[at(1)]),
        ),
        Kind::Repeat => fixed(Step::Repeat(limit(ruby, at(0))?), Few::of(&[at(1)])),
        Kind::RetryUntilSuccessful => fixed(
            Step::RetryUntilSuccessful(limit(ruby, at(0))?),
            Few::of(&[at(1)]),
        ),
    };
    read.step
        .validate()
        .map_err(|refusal| range_error(ruby, refusal))?;
    Ok(read)
}

/// Calls `visit` with each child of a read node, in order. A list is read
/// by index, natively.
fn each_child(read: &Read, mut visit: impl FnMut(Value) -> Result<(), Error>) -> Result<(), Error> {
    match &read.children {
        Children::List(list) => {
            for index in 0..list.len() {
                visit(list.entry(index.cast_signed())?)?;
            }
            Ok(())
        }
        Children::Fixed(children) => children.iter().try_for_each(visit),
    }
}

/// A fresh frozen Array holding `values`.
fn frozen(ruby: &Ruby, values: impl Iterator<Item = Value>) -> Result<RArray, Error> {
    let array = ruby.ary_new();
    for value in values {
        array.push(value)?;
    }
    array.freeze();
    Ok(array)
}

/// `BehaviorMachines.native_node`: what every node's `initialize` validates,
/// handed the node being built. Its kind is its exact class, asked natively,
/// never through a redefinable `class`. Answers the fields to store, in
/// member order and frozen, a composite's children as a frozen copy so the
/// value cannot change after it was checked.
pub fn validate(ruby: &Ruby, node: Value, fields: Value) -> Result<RArray, Error> {
    let classes = nodes(ruby)?;
    let kind = which(node, &classes)
        .and_then(|index| ALL_KINDS.get(index).copied())
        .ok_or_else(|| type_error(ruby, "not a BehaviorMachines::Node class"))?;
    let given = exact_array(ruby, fields, "fields must be an Array")?;
    if given.len() != kind.arity() {
        return Err(type_error(ruby, "the node's fields do not match its class"));
    }
    let mut fields = [ruby.qnil().as_value(); FIELDS];
    for (index, cell) in fields.iter_mut().take(kind.arity()).enumerate() {
        *cell = given.entry(index.cast_signed())?;
    }
    let fields = fields.get(..kind.arity()).unwrap_or_default();
    let read = read(ruby, kind, fields)?;
    each_child(&read, |child| kind_of(ruby, &classes, child).map(drop))?;
    let mut stored = Few::of(fields);
    if let (Children::List(children), Some(first)) = (&read.children, stored.values.first_mut()) {
        let copy = ruby.ary_new_capa(children.len());
        each_child(&read, |child| copy.push(child))?;
        copy.freeze();
        *first = Some(copy.as_value());
    }
    frozen(ruby, stored.iter())
}

/// A tree definition read from Ruby: its steps in pre-order, and every leaf
/// object, which the tree marks for the GC.
pub struct Definition {
    pub steps: Vec<Step<RubyLeaf>>,
    pub leaves: Vec<Opaque<Value>>,
}

/// Walks `root` into the core's steps. Every node is read by its exact class,
/// its fields from the struct, and validated again.
pub fn definition(ruby: &Ruby, root: Value) -> Result<Definition, Error> {
    let classes = nodes(ruby)?;
    let mut definition = Definition {
        steps: Vec::new(),
        leaves: Vec::new(),
    };
    walk(ruby, &classes, root, &mut definition)?;
    Ok(definition)
}

fn walk(
    ruby: &Ruby,
    classes: &[RClass; KINDS],
    value: Value,
    definition: &mut Definition,
) -> Result<(), Error> {
    if definition.steps.len() >= CAPACITY {
        let message = format!("a tree holds at most {CAPACITY} nodes");
        return Err(Error::new(ruby.exception_range_error(), message));
    }
    let message = || "a node must be a BehaviorMachines::Node value";
    let (index, held) = which_struct_with(ruby, value, classes, message)?;
    let Some(kind) = ALL_KINDS.get(index).copied() else {
        return Err(type_error(ruby, message()));
    };
    let fields = match kind.arity() {
        0 => Few::new(),
        1 => Few::of(&held.fields::<1>(ruby, message())?),
        2 => Few::of(&held.fields::<2>(ruby, message())?),
        _ => Few::of(&held.fields::<3>(ruby, message())?),
    };
    let mut buffer = [ruby.qnil().as_value(); FIELDS];
    let mut count = 0;
    for (cell, value) in buffer.iter_mut().zip(fields.iter()) {
        *cell = value;
        count += 1;
    }
    let fields = buffer.get(..count).unwrap_or_default();
    let read = read(ruby, kind, fields)?;
    if let Step::Leaf(object) = read.step {
        definition.leaves.push(object.into());
    }
    let step = read.step.try_map(RubyLeaf::new)?;
    definition.steps.push(step);
    each_child(&read, |child| walk(ruby, classes, child, definition))
}
