# frozen_string_literal: true

require_relative 'behavior_machines/version'
require_relative 'behavior_machines/errors'
require_relative 'behavior_machines/node'
require_relative 'behavior_machines/definition'
require_relative 'behavior_machines/engine'
require_relative 'behavior_machines/tree'
require_relative 'behavior_machines/machine'
require 'deadline_machines'

require_relative 'behavior_machines/native_extension'

# Behavior trees with explicit halt semantics, following the behavior-tree
# pattern. Pure Ruby, with an optional native backend built on the behavior-machines Rust crate.
module BehaviorMachines
  STATUSES = %i[success failure running].freeze

  class << self
    # The current monotonic time in nanoseconds, from the family's clock.
    def now = DeadlineMachines.clock.now_ns

    # Whether the native backend is loaded (opt-in, see NativeExtension).
    def native_available? = NativeExtension.loaded?

    def resolve_backend(backend)
      case backend
      when :ruby then :ruby
      when :auto then native_available? ? :native : :ruby
      when :native
        return :native if native_available?

        raise Error, "native backend not loaded (set BEHAVIOR_MACHINES_NATIVE=1#{"; #{NativeExtension.error}" if NativeExtension.error})"
      else raise ArgumentError, "backend must be :ruby, :native, or :auto, got #{backend.inspect}"
      end
    end

    def check_status(status, leaf)
      return status if STATUSES.include?(status)

      raise InvalidStatusError, "#{leaf.inspect} returned #{status.inspect}; expected :success, :failure, or :running"
    end

    def check_time(now)
      return now if now.is_a?(Integer) && now.between?(0, Definition::MAX_NANOS)

      raise ArgumentError, "now must be an Integer number of nanoseconds in 0..2**64-1, got #{now.inspect}"
    end
  end
end

BehaviorMachines::NativeExtension.load!
