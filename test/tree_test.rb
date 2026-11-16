# frozen_string_literal: true

require_relative 'test_helper'
require_relative 'lifecycle'

class TreeTest < Minitest::Test
  # MOCKED: a leaf that raises from tick.
  class Exploding
    def tick(_ctx, _now) = raise(ArgumentError, 'boom')
  end

  # MOCKED: a leaf that returns a bad status.
  class Confused
    def tick(_ctx, _now) = :done
  end

  def test_halt_halts_every_active_leaf_once_and_is_idempotent
    script = Script.new
    script.set('ready', :success)
    tree = tree(BM.fallback(BM.sequence(leaf('ready'), parallel_all(leaf('a'), leaf('b'))), leaf('unreached')))

    assert_equal :running, tree.tick(script, 0)
    assert_predicate tree, :running?
    assert_equal %i[halted already_idle], Array.new(2) { tree.halt(script) }
    assert_equal [1, 1], %w[a b].map { script.count(halted(_1)) }
    refute_predicate tree, :running?
    script.assert_idle!
  end

  def test_tree_restarts_after_finishing_or_halting
    script = Script.new
    script.set('a', :success)
    tree = tree(BM.sequence(leaf('a'), leaf('b')))

    assert_equal :running, tree.tick(script, 0)
    tree.halt(script)
    assert_equal :running, tree.tick(script, 0)
    script.set('b', :success)
    2.times { assert_equal :success, tree.tick(script, 0) }
    assert_equal [3, 4], %w[a b].map { script.count(tick(_1)) }
  end

  def test_a_raising_leaf_halts_every_active_leaf_then_propagates
    script = Script.new
    root = parallel_all(leaf('a'), BM.sequence(leaf('gate'), BM.leaf(Exploding.new)), leaf('c'))
    tree = tree(root)

    assert_equal :running, tree.tick(script, 0)
    script.set('gate', :success)
    error = assert_raises(ArgumentError) { tree.tick(script, 0) }
    assert_equal 'boom', error.message
    assert_equal [1, 1], %w[a c].map { script.count(halted(_1)) }
    refute_predicate tree, :running?
    script.assert_idle!

    # The tree is usable again and starts fresh.
    script.set('gate', :running)
    assert_equal :running, tree.tick(script, 0)
    tree.halt(script)
    script.assert_idle!
  end

  def test_leaves_started_in_the_raising_tick_are_halted_too
    script = Script.new
    tree = tree(parallel_all(leaf('fresh'), BM.leaf(Exploding.new)))

    assert_raises(ArgumentError) { tree.tick(script, 0) }
    assert_equal [tick('fresh'), halted('fresh')], script.log
  end

  def test_invalid_statuses_raise_and_clean_up
    { :done => RangeError, 'success' => TypeError, nil => TypeError }.each do |status, error_class|
      script = Script.new
      confused = Class.new { define_method(:tick) { |_ctx, _now| status } }.new
      tree = tree(parallel_all(leaf('a'), BM.leaf(confused)))

      error = assert_raises(error_class, status.inspect) { tree.tick(script, 0) }
      assert_match(/a leaf status/, error.message)
      assert_equal 1, script.count(halted('a'))
    end
  end

  # A Symbol whose name is not UTF-8 names no status: RangeError, never
  # EncodingError, with the active sibling halted like any other refusal.
  def test_a_non_utf8_status_symbol_is_a_range_error
    script = Script.new
    invalid = [255].pack('C').b.to_sym
    confused = Class.new { define_method(:tick) { |_ctx, _now| invalid } }.new
    tree = tree(parallel_all(leaf('a'), BM.leaf(confused)))

    error = assert_raises(RangeError) { tree.tick(script, 0) }
    assert_equal 'a leaf status must be one of :success, :failure, :running', error.message
    assert_equal 1, script.count(halted('a'))
  end

  def test_leaves_cannot_call_back_into_their_own_tree
    tree = nil
    reentrant = Class.new { define_method(:tick) { |ctx, now| ctx.call.tick(ctx, now) } }.new
    tree = tree(BM.leaf(reentrant))
    error = assert_raises(RuntimeError) { tree.tick(-> { tree }, 0) }
    assert_match(/called back into the tree/, error.message)
    refute_predicate tree, :running?
  end

  def test_now_must_be_integer_nanoseconds
    tree = tree(BM.always_success)
    { -1 => RangeError, 2**200 => RangeError, 1.5 => TypeError, nil => TypeError }.each do |now, error|
      assert_raises(error, now.inspect) { tree.tick(nil, now) }
    end
    assert_equal :success, tree.tick(nil, (2**64) + 5)
  end

  def test_a_regressing_tick_is_refused_runs_nothing_and_records_nothing
    script = Script.new
    tree = tree(BM.sequence(leaf('a'), leaf('b')))
    assert_equal :running, tree.tick(script, ms(5))

    assert_includes BM::REFUSALS, :time_regression
    assert_equal :time_regression, tree.tick(script, ms(4))
    assert_equal :time_regression, tree.tick_observed(script, ms(4), Recorder.new)
    assert_equal [tick('a')], script.log
    assert_predicate tree, :running?
    assert_equal :running, tree.tick(script, ms(5))
    tree.halt(script)
  end

  def test_a_frozen_tree_refuses_ticks_and_halts_but_answers_queries
    script = Script.new
    tree = tree(leaf('a'))
    assert_equal :running, tree.tick(script, 0)
    tree.freeze

    assert_raises(FrozenError) { tree.tick(script, 1) }
    assert_raises(FrozenError) { tree.halt(script) }
    assert_equal [true, nil, 1], [tree.running?, tree.next_deadline, tree.size]
    assert_equal [tick('a')], script.log
  end

  def test_every_shape_resets_to_fresh_after_finishing_or_halting
    Lifecycle.shapes.each_index do |shape|
      27.times do |first|
        27.times do |second|
          case_name = "shape=#{shape} first=#{first} second=#{second}"
          script = Script.new
          reused = tree(Lifecycle.shapes[shape])
          [[0, first], [1, second]].each do |at, code|
            Lifecycle.script(script, code)
            next if reused.tick(script, ms(at)) == :running

            script.assert_idle!
            assert_nil reused.next_deadline, case_name
          end
          reused.halt(script)
          script.assert_idle!
          halted = script.log.size
          reused.halt(script)
          assert_equal halted, script.log.size, case_name

          script.log.clear
          fresh_script = Script.new
          fresh = tree(Lifecycle.shapes[shape])
          [[10, second], [11, first], [12, second]].each do |at, code|
            Lifecycle.script(script, code)
            Lifecycle.script(fresh_script, code)
            assert_equal fresh.tick(fresh_script, ms(at)), reused.tick(script, ms(at)), case_name
            assert_equal [fresh.next_deadline], [reused.next_deadline], case_name
            assert_equal fresh_script.log, script.log, case_name
          end
          reused.halt(script)
          fresh.halt(fresh_script)
          assert_equal fresh_script.log, script.log, case_name
          script.assert_idle!
        end
      end
    end
  end
end
