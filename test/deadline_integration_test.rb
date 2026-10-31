# frozen_string_literal: true

require 'test_helper'

class DeadlineIntegrationTest < Minitest::Test
  def setup
    @clock = DeadlineMachines::TestClock.new
    DeadlineMachines.clock = @clock
  end

  def teardown
    DeadlineMachines.clock = nil
  end

  def test_a_timeout_node_is_driven_by_the_family_clock_without_sleeping
    tree = tree(leaf('a').timeout(0.1))
    script = Script.new

    assert_equal :running, tree.tick(script)
    @clock.advance(0.099)
    assert_equal :running, tree.tick(script)
    @clock.advance(0.001)
    assert_equal :failure, tree.tick(script)
  end

  def test_an_abandoned_caller_stops_the_tree_from_ticking
    tree = tree(leaf('a'))
    script = Script.new

    DeadlineMachines.with_deadline(1.0) do
      assert_equal :running, tree.tick(script)
      @clock.advance(2.0)

      assert_raises(DeadlineMachines::Exceeded) { tree.tick(script) }
    end

    assert_equal [tick('a')], script.log
  end

  def test_without_a_deadline_ticking_is_unchanged
    tree = tree(leaf('a'))
    script = Script.new

    assert_equal :running, tree.tick(script)
    assert_equal :running, tree.tick(script)
  end
end
