# frozen_string_literal: true

require_relative 'test_helper'

class ControlTest < Minitest::Test
  def if_tree = tree(BM.if_then_else(leaf('cond'), leaf('then'), leaf('otherwise')))
  def while_tree = tree(BM.while_do_else(leaf('cond'), leaf('body'), leaf('otherwise')))

  def test_if_runs_the_chosen_branch_in_the_same_tick
    { success: %w[then otherwise], failure: %w[otherwise then] }.each do |condition, (chosen, skipped)|
      script = Script.new
      script.set('cond', condition)
      script.set(chosen, :success)

      assert_equal :success, if_tree.tick(script, 0)
      assert_equal [tick('cond'), tick(chosen)], script.log
      assert_equal 0, script.count(tick(skipped))
    end
  end

  def test_if_stops_checking_its_condition_once_a_branch_runs
    script = Script.new
    tree = if_tree
    assert_equal :running, tree.tick(script, 0)
    script.set('cond', :success)
    assert_equal :running, tree.tick(script, 0)
    script.set('cond', :failure)
    assert_equal :running, tree.tick(script, 0)
    assert_equal [2, 2, 0], %w[cond then otherwise].map { script.count(tick(_1)) }
    tree.halt(script)
    script.assert_idle!
  end

  def test_if_then_without_otherwise_fails_with_its_condition
    script = Script.new
    script.set('cond', :failure)
    tree = tree(BM.if_then(leaf('cond'), leaf('then')))

    assert_equal :failure, tree.tick(script, 0)
    script.set('cond', :success)
    assert_equal :running, tree.tick(script, 0)
    tree.halt(script)
    script.assert_idle!
  end

  def test_while_do_else_halts_the_running_branch_before_switching
    { 'body' => ['otherwise', :failure], 'otherwise' => ['body', :success] }.each do |from, (to, flip)|
      script = Script.new
      script.set('cond', flip == :failure ? :success : :failure)
      tree = while_tree

      assert_equal :running, tree.tick(script, 0)
      script.set('cond', flip)
      assert_equal :running, tree.tick(script, 0)
      assert_operator script.index(halted(from)), :<, script.index(tick(to))
      tree.halt(script)
      script.assert_idle!
    end
  end

  def test_while_do_else_pauses_its_branches_while_the_condition_runs
    script = Script.new
    script.set('cond', :success)
    tree = tree(BM.while_do_else(leaf('cond'), leaf('body').timeout(ms(100)), leaf('otherwise')))

    assert_equal :running, tree.tick(script, 0)
    script.set('cond', :running)
    assert_equal :running, tree.tick(script, ms(10))
    assert_equal 0, script.count(halted('body'))
    assert_nil tree.next_deadline

    script.set('cond', :success)
    assert_equal :running, tree.tick(script, ms(20))
    assert_equal 2, script.count(tick('body'))
    assert_equal ms(100), tree.next_deadline
    tree.halt(script)
    script.assert_idle!
  end

  def test_while_do_without_otherwise_fails_when_its_condition_fails
    script = Script.new
    script.set('cond', :success)
    tree = tree(BM.while_do(leaf('cond'), leaf('body')))

    assert_equal :running, tree.tick(script, 0)
    script.set('cond', :failure)
    assert_equal :failure, tree.tick(script, 0)
    assert_equal 1, script.count(halted('body'))
    script.assert_idle!
  end
end
