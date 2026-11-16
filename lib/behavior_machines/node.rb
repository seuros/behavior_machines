# frozen_string_literal: true

module BehaviorMachines
  # Immutable node definitions, the core's `Node`: one `Data` class per
  # variant, with the variant's fields in the same order. Each validates at
  # construction through the core: TypeError for the wrong type, RangeError
  # for a value the core refuses (an empty composite, a Parallel threshold
  # outside its child count, a negative duration). A composite keeps a frozen
  # copy of its children.
  #
  # Durations are Integer nanoseconds. A Parallel threshold is :all or an
  # Integer; a loop limit is :unlimited or an Integer. An `otherwise` branch
  # is a node or nil.
  module Node
    # Decorators available on every node, the core's `Node` methods.
    module Decorators
      def inverter = Inverter.new(self)
      def force_success = ForceSuccess.new(self)
      def force_failure = ForceFailure.new(self)
      def timeout(duration) = Timeout.new(duration, self)
      def repeat(limit) = Repeat.new(limit, self)
      def retry_until_successful(limit) = RetryUntilSuccessful.new(limit, self)
      def keep_running_until_failure = KeepRunningUntilFailure.new(self)
    end

    # A node class whose `initialize` hands its fields to the core first.
    def self.define(*members)
      Data.define(*members) do
        include Decorators

        define_method(:initialize) do |**fields|
          missing = members - fields.keys
          extra = fields.keys - members
          raise ArgumentError, "missing keyword: #{missing.map(&:inspect).join(', ')}" unless missing.empty?
          raise ArgumentError, "unknown keyword: #{extra.map(&:inspect).join(', ')}" unless extra.empty?

          checked = BehaviorMachines.__send__(:native_node, self, members.map { |name| fields[name] })
          super(**members.zip(checked).to_h)
        end
      end
    end
    private_class_method :define

    Leaf = define(:leaf)
    AlwaysSuccess = define
    AlwaysFailure = define
    Sleep = define(:duration)
    Sequence = define(:children)
    ReactiveSequence = define(:children)
    Fallback = define(:children)
    ReactiveFallback = define(:children)
    Parallel = define(:children, :success, :failure)
    IfThenElse = define(:condition, :then_branch, :otherwise)
    WhileDoElse = define(:condition, :body, :otherwise)
    Inverter = define(:child)
    ForceSuccess = define(:child)
    ForceFailure = define(:child)
    Timeout = define(:duration, :child)
    Repeat = define(:limit, :child)
    RetryUntilSuccessful = define(:limit, :child)
    KeepRunningUntilFailure = define(:child)

    CLASSES = [
      Leaf, AlwaysSuccess, AlwaysFailure, Sleep, Sequence, ReactiveSequence, Fallback, ReactiveFallback,
      Parallel, IfThenElse, WhileDoElse, Inverter, ForceSuccess, ForceFailure, Timeout, Repeat,
      RetryUntilSuccessful, KeepRunningUntilFailure
    ].freeze

    # Sealed: no subclass can skip the checks above by overriding
    # `initialize`, and no bare `allocate` builds a value that never ran
    # them. Ruby still binds a subclass constant before `inherited` raises;
    # the native side refuses its instances.
    CLASSES.each do |node|
      node.singleton_class.undef_method(:allocate)
      node.define_singleton_method(:inherited) { |_| raise TypeError, "#{node} cannot be subclassed" }
    end
  end

  class << self
    # The core's `Node::leaf`: any object responding to tick(ctx, now) and,
    # optionally, halt(ctx).
    def leaf(leaf) = Node::Leaf.new(leaf)

    # Succeeds immediately.
    def always_success = Node::AlwaysSuccess.new

    # Fails immediately.
    def always_failure = Node::AlwaysFailure.new

    # Runs until `duration` nanoseconds have passed since it started, then
    # succeeds.
    def sleep(duration) = Node::Sleep.new(duration)

    # Ticks children in order; fails on the first failure, succeeds once all
    # succeed. A running child is resumed on the next tick.
    def sequence(*children) = Node::Sequence.new(children)

    # A sequence that restarts from its first child every tick, halting a
    # running later child once an earlier one stops succeeding.
    def reactive_sequence(*children) = Node::ReactiveSequence.new(children)

    # Ticks children in order; succeeds on the first success, fails once all
    # fail. A running child is resumed on the next tick.
    def fallback(*children) = Node::Fallback.new(children)

    # A fallback that restarts from its first child every tick, so a
    # higher-priority child preempts (halts) a running lower-priority one.
    def reactive_fallback(*children) = Node::ReactiveFallback.new(children)

    # Ticks every unfinished child each tick. Succeeds once `success` children
    # succeeded; fails once `failure` failed or success became unreachable.
    def parallel(children, success, failure) = Node::Parallel.new(children, success, failure)

    # Ticks `condition` until it finishes, then runs `then_branch` or
    # `otherwise`.
    def if_then_else(condition, then_branch, otherwise) = Node::IfThenElse.new(condition, then_branch, otherwise)

    # An if_then_else without `otherwise`: a failed condition fails the node.
    def if_then(condition, then_branch) = Node::IfThenElse.new(condition, then_branch, nil)

    # The reactive if_then_else: re-checks `condition` every tick, pausing the
    # branches while it runs and halting the unselected branch when it flips.
    def while_do_else(condition, body, otherwise) = Node::WhileDoElse.new(condition, body, otherwise)

    # A while_do_else without `otherwise`: a failed condition fails the node.
    def while_do(condition, body) = Node::WhileDoElse.new(condition, body, nil)
  end
end
