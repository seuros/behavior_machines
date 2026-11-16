# frozen_string_literal: true

module BehaviorMachines
  # Leaves that drive objects powered by the state_machines gem, the core's
  # `machine` adapters. They reach their object through the helpers
  # state_machines generates for every machine: `<machine>_name` and
  # `fire_<machine>_event`.
  #
  # Each leaf finds its object in the tree context with `access`: a Symbol
  # naming a public method on the context, or anything responding to
  # call(ctx). `machine` names the state machine attribute (`:state` for the
  # default one). States and events are Symbols.
  #
  # These are plain Ruby leaves, layered on the tree: the core's adapters hold
  # Rust function pointers, which Ruby cannot pass.
  module Machine
    # The object `access` names in `ctx`.
    def self.reach(access, ctx) = access.is_a?(Symbol) ? ctx.public_send(access) : access.call(ctx)

    def self.check_access(access)
      return if access.is_a?(Symbol) || access.respond_to?(:call)

      raise TypeError, "access must be a Symbol or respond to call, got #{access.inspect}"
    end

    def self.symbols(states, name)
      raise TypeError, "#{name} must be an Array of Symbols" unless states.instance_of?(Array) && states.all?(Symbol)

      states.dup.freeze
    end

    def self.symbol(value, name)
      raise TypeError, "#{name} must be a Symbol" unless value.is_a?(Symbol)

      value
    end

    # Succeeds while the machine is in one of `states`, fails otherwise.
    InState = Data.define(:access, :machine, :states) do
      def initialize(access:, machine:, states:)
        Machine.check_access(access)
        states = Machine.symbols(states, 'states')
        raise RangeError, 'InState needs at least one state' if states.empty?

        super(access:, machine: Machine.symbol(machine, 'machine'), states:)
      end

      def tick(ctx, _now)
        states.include?(Machine.reach(access, ctx).public_send(:"#{machine}_name")) ? :success : :failure
      end
    end

    # Fires `event` each tick: succeeds if the machine accepts it, fails if it
    # is rejected.
    Fire = Data.define(:access, :machine, :event) do
      def initialize(access:, machine:, event:)
        Machine.check_access(access)
        super(access:, machine: Machine.symbol(machine, 'machine'), event: Machine.symbol(event, 'event'))
      end

      def tick(ctx, _now)
        Machine.reach(access, ctx).public_send(:"fire_#{machine}_event", event) ? :success : :failure
      end
    end

    # Waits on a machine something else drives: succeeds in a `success`
    # state, fails in a `failure` state, runs otherwise. If the tree abandons
    # it, fires `on_halt` (a Symbol, typically :cancel, or nil for none).
    WaitFor = Data.define(:access, :machine, :success, :failure, :on_halt) do
      def initialize(access:, machine:, success:, failure:, on_halt:)
        Machine.check_access(access)
        on_halt = Machine.symbol(on_halt, 'on_halt') unless on_halt.nil?
        super(
          access:, machine: Machine.symbol(machine, 'machine'), success: Machine.symbols(success, 'success'),
          failure: Machine.symbols(failure, 'failure'), on_halt:
        )
      end

      def tick(ctx, _now)
        state = Machine.reach(access, ctx).public_send(:"#{machine}_name")
        return :success if success.include?(state)
        return :failure if failure.include?(state)

        :running
      end

      # A rejected event means the machine already left the states that
      # accept it, e.g. a job that completed after the last tick. The tree
      # abandons the leaf either way, so the answer is deliberately unused.
      def halt(ctx)
        Machine.reach(access, ctx).public_send(:"fire_#{machine}_event", on_halt) if on_halt
        nil
      end
    end

    CLASSES = [InState, Fire, WaitFor].freeze

    # Sealed like the nodes: no subclass can skip the checks above by
    # overriding `initialize`, and no bare `allocate` builds a leaf that never
    # ran them.
    CLASSES.each do |leaf|
      leaf.singleton_class.undef_method(:allocate)
      leaf.define_singleton_method(:inherited) { |_| raise TypeError, "#{leaf} cannot be subclassed" }
    end
  end
end
