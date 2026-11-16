# frozen_string_literal: true

# MOCKED: the reference model's compiled nodes; see reference.rb.
module Reference
  module Engine
    # One compiled node. `@running` is set while its last tick returned
    # :running and it has not been halted since; only running slots halt.
    class Slot
      def initialize(id)
        @id = id
        @running = false
      end

      def running? = @running

      def tick(run, now)
        status = step(run, now, !@running)
        @running = status == :running
        run.report_ticked(@id, status)
        status
      end

      def halt(run)
        return unless @running

        @running = false
        stop(run)
        run.report_halted(@id)
      end

      def next_deadline = @running ? running_deadline : nil

      private

      def stop(_run) = nil
      def running_deadline = nil
      def halt_all(slots, run) = slots.each { |slot| slot.halt(run) }
      def earliest(deadlines) = deadlines.compact.min
    end

    # A user leaf.
    class LeafSlot < Slot
      def initialize(id, leaf, halts)
        super(id)
        @leaf = leaf
        @halts = halts
      end

      private

      # After an error no more leaf code runs this call: an active leaf stays
      # active (so cleanup halts it) and an idle one does not start. A leaf
      # that raises counts as finished.
      def step(run, now, starting)
        return starting ? :failure : :running if run.failed?

        Reference.check_status(@leaf.tick(run.ctx, now), @leaf)
      rescue Exception => e # rubocop:disable Lint/RescueException -- recorded, raised after cleanup
        run.record(e)
        :failure
      end

      def stop(run)
        @leaf.halt(run.ctx) if @halts
      rescue Exception => e # rubocop:disable Lint/RescueException -- recorded; the halt continues
        run.record(e)
      end
    end

    # AlwaysSuccess and AlwaysFailure.
    class ConstantSlot < Slot
      def initialize(id, status)
        super(id)
        @status = status
      end

      private

      def step(_run, _now, _starting) = @status
    end

    class SleepSlot < Slot
      def initialize(id, nanos)
        super(id)
        @nanos = nanos
        @deadline = nil
      end

      private

      def step(_run, now, starting)
        @deadline = Engine.deadline_after(now, @nanos) if starting
        Engine.reached?(@deadline, now) ? :success : :running
      end

      def running_deadline = @deadline
    end

    # Sequence and Fallback with memory: resume the running child.
    class OrderedSlot < Slot
      def initialize(id, children, advance_on)
        super(id)
        @children = children
        @advance_on = advance_on
        @cursor = 0
      end

      private

      def step(run, now, _starting)
        while (child = @children[@cursor])
          status = child.tick(run, now)
          return :running if status == :running
          return finish(status) unless status == @advance_on

          @cursor += 1
        end
        finish(@advance_on)
      end

      def finish(status)
        @cursor = 0
        status
      end

      def stop(run)
        halt_all(@children, run)
        @cursor = 0
      end

      def running_deadline = earliest(@children.map(&:next_deadline))
    end

    # ReactiveSequence and ReactiveFallback: re-walk from the first child.
    class ReactiveSlot < Slot
      def initialize(id, children, advance_on)
        super(id)
        @children = children
        @advance_on = advance_on
      end

      private

      def step(run, now, _starting)
        @children.each_with_index do |child, index|
          status = child.tick(run, now)
          next if status == @advance_on

          halt_all(@children.drop(index + 1), run)
          return status
        end
        @advance_on
      end

      def stop(run) = halt_all(@children, run)
      def running_deadline = earliest(@children.map(&:next_deadline))
    end

    class ParallelSlot < Slot
      def initialize(id, children, success, failure)
        super(id)
        @children = children
        @success = success
        @failure = failure
        @outcomes = Array.new(children.size)
      end

      private

      def step(run, now, _starting)
        decided = decide(run, now)
        return :running unless decided

        halt_all(@children, run)
        @outcomes.fill(nil)
        decided
      end

      # Children decide in order, so the leftmost wins ties.
      def decide(run, now)
        succeeded = @outcomes.count(:success)
        failed = @outcomes.count(:failure)
        @children.each_with_index do |child, index|
          next if @outcomes[index]

          status = child.tick(run, now)
          next if status == :running

          @outcomes[index] = status
          status == :success ? succeeded += 1 : failed += 1
          return :success if succeeded >= @success
          return :failure if failed >= @failure || @children.size - failed < @success
        end
        nil
      end

      def stop(run)
        halt_all(@children, run)
        @outcomes.fill(nil)
      end

      def running_deadline = earliest(@children.map(&:next_deadline))
    end

    # Shared by IfThenElse and WhileDoElse.
    class ConditionalSlot < Slot
      def initialize(id, condition, branch, otherwise)
        super(id)
        @condition = condition
        @branch = branch
        @otherwise = otherwise
      end

      private

      def stop(run)
        [@condition, @branch, @otherwise].compact.each { |slot| slot.halt(run) }
      end

      def running_deadline = earliest([@condition, @branch, @otherwise].compact.map(&:next_deadline))
    end

    class IfThenElseSlot < ConditionalSlot
      def initialize(...)
        super
        @selected = nil
      end

      private

      def step(run, now, _starting)
        unless @selected
          case @condition.tick(run, now)
          when :running then return :running
          when :success then @selected = :branch
          else @selected = :otherwise
          end
        end
        chosen = @selected == :branch ? @branch : @otherwise
        return finish(:failure) unless chosen

        status = chosen.tick(run, now)
        status == :running ? status : finish(status)
      end

      def finish(status)
        @selected = nil
        status
      end

      def stop(run)
        super
        @selected = nil
      end
    end

    # While the condition runs the branches are paused: neither ticked nor
    # halted. Otherwise the unselected branch is halted before the other ticks.
    class WhileDoElseSlot < ConditionalSlot
      private

      def step(run, now, _starting)
        case @condition.tick(run, now)
        when :running then :running
        when :success
          @otherwise&.halt(run)
          @branch.tick(run, now)
        else
          @branch.halt(run)
          @otherwise ? @otherwise.tick(run, now) : :failure
        end
      end

      # A paused branch's timers cannot act until the condition finishes.
      def running_deadline = @condition.running? ? @condition.next_deadline : super
    end

    # Inverter, ForceSuccess, and ForceFailure.
    class MapSlot < Slot
      def initialize(id, child, map)
        super(id)
        @child = child
        @map = map
      end

      private

      def step(run, now, _starting)
        status = @child.tick(run, now)
        return status if status == :running

        case @map
        when :inverter then status == :success ? :failure : :success
        when :force_success then :success
        else :failure
        end
      end

      def stop(run) = @child.halt(run)
      def running_deadline = @child.next_deadline
    end

    # The child only ticks strictly before the deadline.
    class TimeoutSlot < Slot
      def initialize(id, nanos, child)
        super(id)
        @nanos = nanos
        @child = child
        @deadline = nil
      end

      private

      def step(run, now, starting)
        @deadline = Engine.deadline_after(now, @nanos) if starting
        if Engine.reached?(@deadline, now)
          @child.halt(run)
          return :failure
        end
        @child.tick(run, now)
      end

      def stop(run) = @child.halt(run)
      def running_deadline = earliest([@deadline, @child.next_deadline])
    end

    # Repeat, RetryUntilSuccessful, and KeepRunningUntilFailure: at most one
    # finished child run per tick; the next run is due at once.
    class LoopSlot < Slot
      def initialize(id, child, again_on, limit)
        super(id)
        @child = child
        @again_on = again_on
        @limit = limit
        @runs = 0
        @due = nil
      end

      private

      def step(run, now, _starting)
        @due = nil
        return @again_on if @limit&.zero?

        status = @child.tick(run, now)
        return status if status == :running
        return finish(status) unless status == @again_on
        return again(now) if @limit.nil?
        return finish(status) if @runs + 1 == @limit

        @runs += 1
        again(now)
      end

      # A run finished and the next waits for a tick: due at once.
      def again(now)
        @due = now
        :running
      end

      def finish(status)
        @runs = 0
        status
      end

      def stop(run)
        @child.halt(run)
        @runs = 0
      end

      def running_deadline = earliest([@due, @child.next_deadline])
    end
  end
end
