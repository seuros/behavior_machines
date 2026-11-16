# frozen_string_literal: true

# MOCKED: a reference model of the core engine, kept only for the parity
# tests. It is the pure-Ruby backend the gem shipped before the native core
# became required, written independently of the Rust walk, so the two
# agreeing on every shape and script is evidence that both are right.
module Reference
  # Lowers a node definition to the nested-Array spec the model builds from.
  module Definition
    module_function

    N = BehaviorMachines::Node

    def compile(node)
      case node
      when N::Leaf then [:leaf, node.leaf, node.leaf.respond_to?(:halt)]
      when N::AlwaysSuccess then %i[constant success]
      when N::AlwaysFailure then %i[constant failure]
      when N::Sleep then [:sleep, node.duration]
      when N::Sequence then [:ordered, node.children.map { compile(_1) }, :success, false]
      when N::ReactiveSequence then [:ordered, node.children.map { compile(_1) }, :success, true]
      when N::Fallback then [:ordered, node.children.map { compile(_1) }, :failure, false]
      when N::ReactiveFallback then [:ordered, node.children.map { compile(_1) }, :failure, true]
      when N::Parallel then parallel(node)
      when N::IfThenElse then conditional(node.condition, node.then_branch, node.otherwise, false)
      when N::WhileDoElse then conditional(node.condition, node.body, node.otherwise, true)
      when N::Inverter then [:map, compile(node.child), :inverter]
      when N::ForceSuccess then [:map, compile(node.child), :force_success]
      when N::ForceFailure then [:map, compile(node.child), :force_failure]
      when N::Timeout then [:timeout, node.duration, compile(node.child)]
      when N::Repeat then [:loop, compile(node.child), :success, limit(node.limit)]
      when N::RetryUntilSuccessful then [:loop, compile(node.child), :failure, limit(node.limit)]
      when N::KeepRunningUntilFailure then [:loop, compile(node.child), :success, nil]
      end
    end

    def parallel(node)
      size = node.children.size
      [:parallel, node.children.map { compile(_1) }, threshold(node.success, size), threshold(node.failure, size)]
    end

    def threshold(value, size) = value == :all ? size : value
    def limit(value) = value == :unlimited ? nil : value

    def conditional(condition, branch, otherwise, reactive)
      [:conditional, compile(condition), compile(branch), otherwise && compile(otherwise), reactive]
    end
  end

  # The status check the native side makes: TypeError for a non-Symbol,
  # RangeError for another Symbol.
  def self.check_status(status, _leaf)
    return status if BehaviorMachines::STATUSES.include?(status)
    raise TypeError, 'a leaf status must be a Symbol' unless status.is_a?(Symbol)

    raise RangeError, 'a leaf status must be one of :success, :failure, :running'
  end

  module Engine
    # Duration::MAX, in nanoseconds.
    MAX_TIME = (((2**64) - 1) * 1_000_000_000) + 999_999_999

    # Discards observer reports.
    module NullObserver
      def self.ticked(_id, _status) = nil
      def self.halted(_id) = nil
    end

    # One tick or halt. `errors` collects what leaves and observers raise; once
    # it holds one, no more leaf ticks or observer reports run this call.
    Run = Data.define(:ctx, :observer, :errors) do
      def self.start(ctx, observer) = new(ctx, observer || NullObserver, [])

      def failed? = !errors.empty?

      def record(error)
        errors << error
        nil
      end

      def report_ticked(id, status) = report { observer.ticked(id, status) }
      def report_halted(id) = report { observer.halted(id) }

      private

      def report
        return if failed?

        yield
      rescue Exception => e # rubocop:disable Lint/RescueException -- recorded, raised after cleanup
        record(e)
      end
    end

    # A deadline past the end of the clock never fires.
    def self.deadline_after(now, nanos)
      deadline = now + nanos
      deadline > MAX_TIME ? nil : deadline
    end

    def self.reached?(deadline, now) = !deadline.nil? && now >= deadline

    # Compiled tree plus its execution state.
    class Tree
      def initialize(root)
        @root = Builder.new.build(Definition.compile(root))
      end

      # Once a leaf or observer raises, the rest of the tick runs no Ruby: an
      # active leaf stays active and an idle one does not start. Then every
      # leaf still active is halted, without observer reports, and the first
      # error propagates. Errors raised during that cleanup are dropped.
      def tick(ctx, now, observer)
        run = Run.start(ctx, observer)
        status = @root.tick(run, now)
        return status unless run.failed?

        @root.halt(Run.start(ctx, nil))
        raise run.errors.first
      end

      # Every active leaf is halted even if one, or the observer, raises; the
      # first error propagates afterwards.
      def halt(ctx, observer)
        run = Run.start(ctx, observer)
        @root.halt(run)
        raise run.errors.first if run.failed?
      end

      def running? = @root.running?
      def next_deadline = @root.next_deadline
    end

    # Builds slots from a spec, numbering nodes in pre-order.
    class Builder
      def initialize
        @next_id = 0
      end

      def build(spec)
        id = @next_id
        @next_id += 1
        kind, *args = spec
        case kind
        when :leaf then LeafSlot.new(id, *args)
        when :constant then ConstantSlot.new(id, *args)
        when :sleep then SleepSlot.new(id, *args)
        when :ordered then ordered(id, *args)
        when :parallel then parallel(id, *args)
        when :conditional then conditional(id, *args)
        when :map then MapSlot.new(id, build(args[0]), args[1])
        when :timeout then TimeoutSlot.new(id, args[0], build(args[1]))
        when :loop then LoopSlot.new(id, build(args[0]), args[1], args[2])
        else raise ArgumentError, "unknown spec #{kind.inspect}"
        end
      end

      private

      def ordered(id, children, advance_on, reactive)
        (reactive ? ReactiveSlot : OrderedSlot).new(id, children.map { |child| build(child) }, advance_on)
      end

      def parallel(id, children, success, failure)
        ParallelSlot.new(id, children.map { |child| build(child) }, success, failure)
      end

      def conditional(id, condition, branch, otherwise, reactive)
        condition = build(condition)
        branch = build(branch)
        otherwise &&= build(otherwise)
        (reactive ? WhileDoElseSlot : IfThenElseSlot).new(id, condition, branch, otherwise)
      end
    end
  end
end

require_relative 'reference_slots'
