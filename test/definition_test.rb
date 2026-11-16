# frozen_string_literal: true

require_relative 'test_helper'

class DefinitionTest < Minitest::Test
  def test_refuses_empty_composites
    {
      -> { BM.sequence } => 'Sequence',
      -> { BM.reactive_sequence } => 'ReactiveSequence',
      -> { BM.fallback } => 'Fallback',
      -> { BM.reactive_fallback } => 'ReactiveFallback',
      -> { BM.parallel([], :all, 1) } => 'Parallel'
    }.each do |build, name|
      error = assert_raises(RangeError) { build.call }
      assert_equal "#{name} needs at least one child", error.message
    end
  end

  def test_refuses_parallel_thresholds_outside_the_child_count
    two = [leaf('a'), leaf('b')]
    { [0, :all] => RangeError, [:all, 3] => RangeError, [:some, 1] => RangeError, ['1', 1] => TypeError }
      .each do |(success, failure), error|
        assert_raises(error, [success, failure].inspect) { BM.parallel(two, success, failure) }
      end
    error = assert_raises(RangeError) { BM.parallel(two, 0, 1) }
    assert_equal 'Parallel threshold 0 must be between 1 and 2', error.message
  end

  def test_refuses_invalid_durations_limits_children_and_leaves
    {
      -> { BM.sleep(-1) } => RangeError, -> { BM.sleep(1.5) } => TypeError, -> { BM.sleep('1') } => TypeError,
      -> { leaf('a').timeout(-1) } => RangeError, -> { leaf('a').repeat(-1) } => RangeError,
      -> { leaf('a').retry_until_successful(:forever) } => RangeError, -> { leaf('a').repeat(2**64) } => RangeError,
      -> { BM.leaf(Object.new) } => TypeError, -> { BM.sequence(:not_a_node) } => TypeError,
      -> { BM::Node::Sequence.new(Class.new(Array).new([leaf('a')])) } => TypeError,
      -> { BM.if_then_else(BM.always_failure, BM.always_success, false) } => TypeError,
      -> { BM.always_success.inverter.then { |node| BM::Node::Inverter.new(child: node, extra: 1) } } => ArgumentError
    }.each { |build, error| assert_raises(error) { build.call } }
  end

  def test_definitions_are_immutable_values
    children = [leaf('a'), leaf('b')]
    node = BM.sequence(*children)
    children << leaf('c')
    assert_predicate node, :frozen?
    assert_predicate node.children, :frozen?
    assert_equal 2, node.children.size
    assert_equal BM.sleep(ms(1000)), BM.sleep(ms(1000))
    assert_equal BM.sequence(BM.always_success), BM::Node::Sequence.new([BM.always_success])
  end

  def test_the_largest_durations_and_limits_are_accepted
    max = (2**64) - 1
    assert_equal :running, tree(leaf('work').repeat(max)).tick(Script.new, 0)
    assert_equal :running, tree(BM.sleep(2**70)).tick(Script.new, 0)
  end

  def test_trees_hold_at_most_their_capacity
    deep = (BM::Tree::CAPACITY - 1).times.reduce(BM.always_success) { |node, _| node.inverter }
    assert_equal BM::Tree::CAPACITY, tree(deep).size
    error = assert_raises(RangeError) { tree(deep.inverter) }
    assert_match(/at most #{BM::Tree::CAPACITY} nodes/, error.message)
    assert_raises(TypeError) { BM::Tree.new(:not_a_node) }
  end
end
