//! Native entry point.
use magnus::{Error, Module, Object, RModule, Ruby, function, method, prelude::*};
use matryona_machines::ruby::{Class, SymbolNames, Symbols};

use crate::classes;
use crate::names::{Halts, Refusals, Statuses};
use crate::nodes::{self, CAPACITY};
use crate::tree::NativeTree;

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("BehaviorMachines")?;
    let node = module.const_get::<_, RModule>("Node")?;
    classes::resolve(ruby, node)?;
    module.const_set("STATUSES", Statuses::every(ruby))?;
    module.const_set("HALTS", Halts::every(ruby))?;
    module.const_set("REFUSALS", Refusals::every(ruby))?;
    module.define_singleton_method("native_node", function!(nodes::validate, 2))?;

    let class = module.define_class("Tree", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.const_set("CAPACITY", CAPACITY)?;
    class.define_singleton_method("new", function!(NativeTree::new, 1))?;
    class.define_method("tick", method!(NativeTree::tick, 2))?;
    class.define_method("tick_observed", method!(NativeTree::tick_observed, 3))?;
    class.define_method("halt", method!(NativeTree::halt, 1))?;
    class.define_method("halt_observed", method!(NativeTree::halt_observed, 2))?;
    class.define_method("running?", method!(NativeTree::is_running, 0))?;
    class.define_method("next_deadline", method!(NativeTree::next_deadline, 0))?;
    class.define_method("size", method!(NativeTree::size, 0))?;
    // `new` answers a Tree: its class is fixed now, before it runs.
    Class::pin_wrapped::<NativeTree>(ruby, class).map(drop)
}
