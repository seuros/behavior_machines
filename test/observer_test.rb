# frozen_string_literal: true

require_relative 'test_helper'

class ObserverTest < Minitest::Test
  def test_reports_pre_order_ids_with_children_before_parents
    script = Script.new
    script.set('a', :success)
    # ids: 0 parallel, 1 sequence, 2 a, 3 b, 4 c
    tree = tree(race(BM.sequence(leaf('a'), leaf('b')), leaf('c')))
    recorder = Recorder.new

    assert_equal :running, tree.tick_observed(script, 0, recorder)
    tree.halt_observed(script, recorder)

    expected = [
      [:ticked, 2, :success], [:ticked, 3, :running], [:ticked, 1, :running],
      [:ticked, 4, :running], [:ticked, 0, :running],
      [:halted, 3], [:halted, 1], [:halted, 4], [:halted, 0]
    ]
    assert_equal expected, recorder.reports
  end

  def test_numbers_conditions_before_branches
    script = Script.new
    %w[cond then keep].each { script.set(_1, :success) }
    # ids: 0 sequence, 1 if, 2 cond, 3 then, 4 otherwise, 5 while, 6 keep, 7 body
    tree = tree(BM.sequence(BM.if_then_else(leaf('cond'), leaf('then'), leaf('otherwise')),
                            BM.while_do(leaf('keep'), leaf('body'))))
    recorder = Recorder.new

    assert_equal :running, tree.tick_observed(script, 0, recorder)
    expected = [[2, :success], [3, :success], [1, :success], [6, :success], [7, :running], [5, :running], [0, :running]]
    assert_equal(expected.map { [:ticked, *_1] }, recorder.reports)
    tree.halt(script)
    script.assert_idle!
  end

  def test_observers_may_implement_only_one_hook
    script = Script.new
    ticks = []
    tree = tree(leaf('a'))
    observer = Object.new.tap { |o| o.define_singleton_method(:ticked) { |id, status| ticks << [id, status] } }

    assert_equal :running, tree.tick_observed(script, 0, observer)
    tree.halt_observed(script, observer)
    assert_equal [[0, :running]], ticks
  end
end
