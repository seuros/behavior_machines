# frozen_string_literal: true

module BehaviorMachines
  # A compiled behavior tree and its execution state.
  #
  # Each tick advances as far as the leaves allow; looping decorators finish
  # at most one child run per tick. When the tree abandons a running node,
  # every active leaf under it is halted exactly once before the call returns.
  #
  # Leaves are objects responding to `tick(ctx, now)`, returning :success,
  # :failure, or :running, and optionally `halt(ctx)`.
  #
  # If a leaf or observer raises during a tick, the rest of that tick runs no
  # leaf code (active leaves stay active, idle ones do not start), every leaf
  # still active is then halted without observer reports, and the first error
  # propagates. The raising leaf counts as finished. During a halt, every
  # active leaf is halted even if a leaf's halt or the observer raises; the
  # first error propagates afterwards. Non-local exits from a leaf, such as
  # `throw` or `break`, are not supported.
  #
  # Time is an Integer count of nanoseconds on a monotonic clock, passed as
  # `now:` (default BehaviorMachines.now). Durations in node definitions are
  # seconds.
  class Tree
    attr_reader :backend

    # Compiles `root`. `backend:` is :ruby, :native, or :auto (native when the
    # extension is loaded, see BehaviorMachines.native_available?).
    def initialize(root, backend: :auto)
      spec = Definition.compile(root)
      @backend = BehaviorMachines.resolve_backend(backend)
      @engine = @backend == :native ? BehaviorMachinesNative::Tree.new(spec) : Engine::Tree.new(spec)
      @busy = false
    end

    # Ticks once and returns the root's status. `observer` may respond to
    # `ticked(node_id, status)` and `halted(node_id)`; node ids number the
    # definition in pre-order, and children report before their parent.
    def tick(ctx = nil, now: BehaviorMachines.now, observer: nil)
      DeadlineMachines.check!
      now = BehaviorMachines.check_time(now)
      exclusive { @engine.tick(ctx, now, Observers.wrap(observer)) }
    end

    # Halts every active leaf and resets the tree. Does nothing if not running.
    def halt(ctx = nil, observer: nil)
      exclusive { @engine.halt(ctx, Observers.wrap(observer)) }
      nil
    end

    # Whether the last tick returned :running and the tree was not halted since.
    def running?
      check_idle
      @engine.running?
    end

    # The earliest time, in nanoseconds, a running Sleep or Timeout will fire,
    # or a looping decorator wants its next run (already due).
    def next_deadline
      check_idle
      @engine.next_deadline
    end

    private

    def check_idle
      raise ReentrantCallError, 'a leaf called back into the tree that is ticking it' if @busy
    end

    def exclusive
      check_idle

      begin
        @busy = true
        yield
      ensure
        @busy = false
      end
    end
  end

  # Normalizes user observers so engines can call both hooks unconditionally.
  module Observers
    # Forwards only the hooks the wrapped observer implements.
    Partial = Data.define(:target) do
      def ticked(id, status) = (target.ticked(id, status) if target.respond_to?(:ticked))
      def halted(id) = (target.halted(id) if target.respond_to?(:halted))
    end

    def self.wrap(observer)
      return nil if observer.nil?
      return observer if observer.respond_to?(:ticked) && observer.respond_to?(:halted)

      Partial.new(observer)
    end
  end
end
