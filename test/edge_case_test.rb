# frozen_string_literal: true

require_relative 'test_helper'
require 'weakref'
require 'state_machines'

# Error paths, timer edges, GC retention, and state_machines integration.
class EdgeCaseTest < Minitest::Test
  class ProbeError < StandardError; end
  class FatalProbe < Exception; end

  class Tripwire
    attr_accessor :armed

    def tick(_ctx, _now)
      raise ProbeError, 'tick failed' if armed

      :running
    end
  end

  class Observer
    attr_accessor :trip_tick, :trip_halt
    attr_reader :reports

    def initialize = @reports = []

    def ticked(id, status)
      @reports << [:ticked, id, status]
      raise ProbeError, 'ticked failed' if id == trip_tick
    end

    def halted(id)
      @reports << [:halted, id]
      raise ProbeError, 'halted failed' if id == trip_halt
    end
  end

  def test_error_in_parallel_any_does_not_erase_active_sibling
    script = Script.new
    bomb = Tripwire.new
    subject = tree(parallel_any(BM.leaf(bomb), leaf('active')))
    assert_equal :running, subject.tick(script, 0)
    bomb.armed = true
    assert_raises(ProbeError) { subject.tick(script, 1) }
    assert_equal 1, script.count(halted('active')), script.log.inspect
    script.assert_idle!
    subject.halt(script)
    assert_equal 1, script.count(halted('active'))
    bomb.armed = false
    assert_equal :running, subject.tick(script, 2)
    subject.halt(script)
    script.assert_idle!
  end

  def test_invalid_status_in_reactive_fallback_preserves_sibling_for_cleanup
    script = Script.new
    script.set('guard', :failure)
    subject = tree(BM.reactive_fallback(leaf('guard'), leaf('active')))
    assert_equal :running, subject.tick(script, 0)
    script.set('guard', :invalid)
    assert_raises(RangeError) { subject.tick(script, 1) }
    assert_equal 1, script.count(halted('active')), script.log.inspect
    script.assert_idle!
    subject.halt(script)
    assert_equal 1, script.count(halted('active'))
  end

  def test_ticked_observer_error_preserves_unticked_active_sibling_for_cleanup
    script = Script.new
    observer = Observer.new
    subject = tree(parallel_any(leaf('left'), leaf('right')))
    assert_equal :running, subject.tick_observed(script, 0, observer)
    observer.reports.clear
    observer.trip_tick = 1
    assert_raises(ProbeError) { subject.tick_observed(script, 1, observer) }
    assert_equal [[:ticked, 1, :running]], observer.reports
    assert_equal [1, 1], %w[left right].map { script.count(halted(_1)) }
    script.assert_idle!
  end

  def test_halted_observer_error_still_cleans_up_every_active_leaf
    script = Script.new
    observer = Observer.new
    subject = tree(parallel_all(leaf('a'), leaf('b'), leaf('c')))
    subject.tick(script, 0)
    observer.trip_halt = 1
    assert_raises(ProbeError) { subject.halt_observed(script, observer) }
    assert_equal [[:halted, 1]], observer.reports
    assert_equal [1, 1, 1], %w[a b c].map { script.count(halted(_1)) }
    script.assert_idle!
    subject.halt(script)
    assert_equal [1, 1, 1], %w[a b c].map { script.count(halted(_1)) }
  end

  def test_halted_observer_error_inside_tick_does_not_hide_nested_active_leaves
    script = Script.new
    observer = Observer.new
    script.set('guard', :success)
    # ids: root 0, guard 1, parallel 2, a 3, b 4.
    subject = tree(BM.reactive_sequence(leaf('guard'), parallel_all(leaf('a'), leaf('b'))))
    assert_equal :running, subject.tick(script, 0)
    script.set('guard', :failure)
    observer.trip_halt = 3
    assert_raises(ProbeError) { subject.tick_observed(script, 1, observer) }
    assert_equal [[:ticked, 1, :failure], [:halted, 3]], observer.reports
    assert_equal [1, 1], %w[a b].map { script.count(halted(_1)) }
    script.assert_idle!
    subject.halt(script)
    assert_equal [1, 1], %w[a b].map { script.count(halted(_1)) }
  end

  class InspectRaises
    def tick(_ctx, _now) = :invalid
    def inspect = raise(ProbeError, 'inspect failed')
  end

  def test_an_invalid_status_raises_without_running_more_ruby
    script = Script.new
    subject = tree(parallel_all(leaf('active'), BM.leaf(InspectRaises.new)))
    error = assert_raises(RangeError) { subject.tick(script, 0) }
    assert_match(/must be one of :success, :failure, :running/, error.message)
    assert_equal 1, script.count(halted('active'))
    script.assert_idle!
  end

  def test_fatal_exception_cleans_up_without_observer_reports
    script = Script.new
    observer = Observer.new
    bomb = Object.new
    def bomb.tick(_ctx, _now) = raise(FatalProbe, 'fatal')
    subject = tree(parallel_all(leaf('active'), BM.leaf(bomb)))
    assert_raises(FatalProbe) { subject.tick_observed(script, 0, observer) }
    assert_equal [[:ticked, 1, :running]], observer.reports
    assert_equal [tick('active'), halted('active')], script.log
    script.assert_idle!
    refute subject.running?
  end

  def test_root_observer_exception_cleans_up_running_tree
    script = Script.new
    observer = Observer.new
    observer.trip_tick = 0
    subject = tree(parallel_all(leaf('a'), leaf('b')))
    assert_raises(ProbeError) { subject.tick_observed(script, 0, observer) }
    assert_equal [1, 1], %w[a b].map { script.count(halted(_1)) }
    refute observer.reports.any? { _1.first == :halted }
    refute subject.running?
    script.assert_idle!
  end

  def test_timers_are_exact_and_a_deadline_past_the_clock_never_fires
    script = Script.new
    max = (((2**64) - 1) * 1_000_000_000) + 999_999_999 # Duration::MAX in nanoseconds
    subject = tree(BM.sleep(max))
    assert_equal :running, subject.tick(script, 0)
    assert_equal max, subject.next_deadline
    assert_equal :success, subject.tick(script, max)
    assert_nil subject.next_deadline

    subject = tree(leaf('work').timeout(2))
    assert_equal :running, subject.tick(script, max - 1)
    assert_nil subject.next_deadline
    assert_equal :running, subject.tick(script, max)
    subject.halt(script)
    script.assert_idle!
    assert_raises(RangeError) { BM.sleep(max + 1) }
  end

  def test_definition_copies_children_but_keeps_mutable_leaf_objects
    script = Script.new
    children = [leaf('a'), leaf('b')]
    subject = tree(BM::Node::Sequence.new(children:))
    children.clear
    script.set('a', :success)
    assert_equal :running, subject.tick(script, 0)
    assert_equal [tick('a'), tick('b')], script.log
    subject.halt(script)
    script.assert_idle!
  end

  def test_reentrant_calls_raise_without_borrow_panics
    %i[tick halt running? next_deadline].each do |operation|
      leaf_object = Object.new
      leaf_object.define_singleton_method(:tick) do |ctx, _now|
        case operation
        when :tick then ctx.tick(ctx, 0)
        when :halt then ctx.halt(ctx)
        else ctx.public_send(operation)
        end
      end
      subject = tree(BM.leaf(leaf_object))
      assert_raises(RuntimeError) { subject.tick(subject, 0) }
      refute subject.running?
    end
  end

  class GcLeaf
    def tick(script, _now)
      GC.start
      GC.compact if GC.respond_to?(:compact) # not on every Ruby engine
      script.tick_leaf('gc')
    end

    def halt(script)
      GC.start
      script.halt_leaf('gc')
    end
  end

  def gc_tree
    object = GcLeaf.new
    [tree(BM.leaf(object)), WeakRef.new(object)]
  end

  def test_native_mark_keeps_leaf_alive_during_tick_halt_and_compaction
    # Bounded collections only, deliberately not GC.stress.
    subject, weak = gc_tree
    GC.start
    assert weak.weakref_alive?
    script = Script.new
    observer = Observer.new
    assert_equal :running, subject.tick_observed(script, 0, observer)
    subject.halt_observed(script, observer)
    assert_equal [[:ticked, 0, :running], [:halted, 0]], observer.reports
    script.assert_idle!
    assert weak.weakref_alive?
  end

  class GcHalt
    def tick(script, _now) = script.tick_leaf('active')

    def halt(script)
      GC.start
      GC.compact if GC.respond_to?(:compact) # not on every Ruby engine
      script.halt_leaf('active')
    end
  end

  def test_exception_survives_gc_during_native_cleanup
    script = Script.new
    bomb = Object.new
    def bomb.tick(_ctx, _now) = raise(ProbeError, 'fresh exception payload')
    subject = tree(parallel_all(BM.leaf(GcHalt.new), BM.leaf(bomb)))
    error = assert_raises(ProbeError) { subject.tick(script, 0) }
    assert_equal 'fresh exception payload', error.message
    script.assert_idle!
  end

  class Job
    state_machine :job, initial: :pending do
      event(:start) { transition pending: :started }
      event(:complete) { transition started: :completed }
      event(:cancel) { transition %i[pending started] => :cancelled }
    end
  end

  def test_real_named_state_machine_helpers_and_cancellation
    job = Job.new
    access = ->(ctx) { ctx }
    fire = BM::Machine::Fire.new(access, :job, :start)
    assert_equal :success, fire.tick(job, 0)
    assert_equal :failure, fire.tick(job, 0)
    condition = BM::Machine::InState.new(access, :job, [:started])
    assert_equal :success, condition.tick(job, 0)
    waiter = BM::Machine::WaitFor.new(access, :job, [:completed], [:cancelled], :cancel)
    subject = tree(BM.leaf(waiter))
    assert_equal :running, subject.tick(job, 0)
    assert_equal %i[halted already_idle], Array.new(2) { subject.halt(job) }
    assert_equal :cancelled, job.job_name
    assert_equal :failure, waiter.tick(job, 0)
  end
end
