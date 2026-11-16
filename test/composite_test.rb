# frozen_string_literal: true

require_relative 'test_helper'

class CompositeTest < Minitest::Test
  def test_ordered_composites_advance_within_one_tick
    cases = [
      [:sequence, { 'a' => :success, 'b' => :success, 'c' => :success }, :success, %w[a b c]],
      [:sequence, { 'a' => :success, 'b' => :failure }, :failure, %w[a b]],
      [:fallback, { 'a' => :failure, 'b' => :failure, 'c' => :failure }, :failure, %w[a b c]],
      [:fallback, { 'a' => :failure, 'b' => :success }, :success, %w[a b]]
    ]
    cases.each do |kind, outcomes, expected, ticked|
      script = Script.new
      outcomes.each { |name, status| script.set(name, status) }
      tree = tree(BM.public_send(kind, leaf('a'), leaf('b'), leaf('c')))

      assert_equal expected, tree.tick(script, 0), outcomes.inspect
      assert_equal(ticked.map { tick(_1) }, script.log)
      script.assert_idle!
    end
  end

  def test_sequence_resumes_the_running_child
    script = Script.new
    tree = tree(BM.sequence(leaf('a'), leaf('b')))

    assert_equal :running, tree.tick(script, 0)
    script.set('a', :success)
    assert_equal :running, tree.tick(script, 0)
    script.set('b', :success)
    assert_equal :success, tree.tick(script, 0)
    assert_equal [tick('a'), tick('a'), tick('b'), tick('b')], script.log
  end

  def test_reactive_sequence_rechecks_its_guard_and_halts_on_failure
    script = Script.new
    script.set('guard', :success)
    tree = tree(BM.reactive_sequence(leaf('guard'), leaf('work')))

    2.times { assert_equal :running, tree.tick(script, 0) }
    assert_equal 2, script.count(tick('guard'))
    script.set('guard', :failure)
    assert_equal :failure, tree.tick(script, 0)
    assert_equal 1, script.count(halted('work'))
    script.assert_idle!
  end

  def test_reactive_fallback_lets_a_higher_priority_child_preempt
    %i[success running].each do |preempting|
      script = Script.new
      script.set('high', :failure)
      tree = tree(BM.reactive_fallback(leaf('high'), leaf('low')))

      assert_equal :running, tree.tick(script, 0)
      script.set('high', preempting)
      assert_equal preempting, tree.tick(script, 0)
      assert_equal 1, script.count(halted('low')), preempting
      tree.halt(script)
      script.assert_idle!
    end
  end

  def test_parallel_all_halts_running_siblings_on_failure
    script = Script.new
    tree = tree(parallel_all(leaf('a'), leaf('b'), leaf('c')))

    assert_equal :running, tree.tick(script, 0)
    script.set('b', :failure)
    assert_equal :failure, tree.tick(script, 0)
    assert_equal [1, 0, 1], %w[a b c].map { script.count(halted(_1)) }
    script.assert_idle!
  end

  def test_race_winner_decides_and_the_leftmost_wins_ties
    script = Script.new
    tree = tree(race(leaf('left'), leaf('right')))

    assert_equal :running, tree.tick(script, 0)
    script.set('left', :failure)
    script.set('right', :success)
    assert_equal :failure, tree.tick(script, 0)
    assert_equal 1, script.count(halted('right'))
    script.assert_idle!
  end

  def test_parallel_any_succeeds_on_first_success_and_fails_once_all_fail
    script = Script.new
    tree = tree(parallel_any(leaf('a'), leaf('b')))
    assert_equal :running, tree.tick(script, 0)
    script.set('b', :success)
    assert_equal :success, tree.tick(script, 0)
    assert_equal 1, script.count(halted('a'))

    script = Script.new
    script.set('a', :failure)
    assert_equal :running, tree.tick(script, 0)
    script.set('b', :failure)
    assert_equal :failure, tree.tick(script, 0)
    script.assert_idle!
  end

  def test_parallel_fails_as_soon_as_success_is_unreachable
    script = Script.new
    tree = tree(BM.parallel([leaf('a'), leaf('b'), leaf('c')], :all, :all))

    assert_equal :running, tree.tick(script, 0)
    script.set('b', :failure)
    assert_equal :failure, tree.tick(script, 0)
    assert_equal [1, 1], %w[a c].map { script.count(halted(_1)) }
    script.assert_idle!
  end

  def test_abandoned_nested_branch_is_halted_before_the_next_child_starts
    script = Script.new
    tree = tree(BM.sequence(race(leaf('winner'), leaf('loser')), leaf('next')))

    assert_equal :running, tree.tick(script, 0)
    script.set('winner', :success)
    assert_equal :running, tree.tick(script, 0)
    assert_operator script.index(halted('loser')), :<, script.index(tick('next'))
  end
end
