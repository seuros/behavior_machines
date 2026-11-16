# frozen_string_literal: true

require_relative 'behavior_machines/version'
require_relative 'behavior_machines/node'
require_relative 'behavior_machines/machine'

# Behavior trees with explicit halt semantics, on the behavior-machines Rust
# core. Every node is a `Data` value mirroring the core's `Node`, field for
# field, and every method takes the core's arguments in the core's order.
# Every instant and span (`now`, a `Sleep` or `Timeout` duration, a deadline)
# is Integer nanoseconds on a monotonic clock the caller picks.
#
# The native side is the authority on every value it reads: a node must be
# exactly one of the `Node` classes (a subclass is a TypeError) and its fields
# are validated again through the core, so a value forged past `initialize`
# raises and never reaches the core.
#
# Refusals are Symbols and never raise: a tick earlier than the latest
# honoured one answers `:time_regression` and runs nothing.
#
# == BehaviorMachines::Tree
#
# A compiled tree and its execution state, the core's `Tree`, built natively:
#
# - `Tree.new(root)`: compiles a node. RangeError past `Tree::CAPACITY`
#   nodes, TypeError for anything but a node.
# - `tick(ctx, now)` and `tick_observed(ctx, now, observer)`: one tick.
#   Answers `:success`, `:failure` or `:running`, or `:time_regression`.
# - `halt(ctx)` and `halt_observed(ctx, observer)`: halts every active leaf.
#   Answers `:halted`, or `:already_idle` when nothing was running.
# - `running?`, `next_deadline` (Integer nanoseconds or nil), `size`.
#
# Leaves are objects responding to `tick(ctx, now)`, returning one of
# STATUSES, and optionally `halt(ctx)`. A status that is not a Symbol is a
# TypeError, another Symbol a RangeError. An observer may respond to
# `ticked(node_id, status)` and `halted(node_id)`; node ids number the
# definition in pre-order, and children report before their parent.
#
# If a leaf or observer raises during a tick, no more Ruby runs in that tick:
# active leaves stay active and idle ones do not start. Every leaf still
# active is then halted, without observer reports, and the first error
# propagates. The raising leaf counts as finished. During a halt, every active
# leaf is halted even if a leaf's halt or the observer raises, and the first
# error propagates afterwards. A leaf calling back into the tree that is
# ticking it raises RuntimeError. A frozen tree raises FrozenError on tick and
# halt.
module BehaviorMachines
end

# Native is required: no pure-Ruby fallback. It resolves the node classes
# above once, here.
require 'behavior_machines_native/behavior_machines_native'

module BehaviorMachines
  private_class_method :native_node

  # Tree wraps native state and has no allocator: a subclass's `new` would
  # build the base class anyway, so subclassing raises too.
  Tree.define_singleton_method(:inherited) { |_| raise TypeError, "#{Tree} cannot be subclassed" }
end
