# frozen_string_literal: true

require_relative 'test_helper'

# The native side is the authority: values forged past `initialize`, sealed
# classes, and classes moved by compaction.
class ForgedTest < Minitest::Test
  # Builds a value past `initialize`: Marshal.load skips it. `from` and `to`
  # are the marshalled bytes of one field, before and after.
  def forge(value, from, to)
    bytes = Marshal.dump(value)
    raise ArgumentError, 'field not found' unless bytes.include?(from)

    Marshal.load(bytes.sub(from, to)) # rubocop:disable Security/MarshalLoad
  end

  def test_a_forged_node_never_reaches_the_core
    negative = forge(BM.sleep(5), "i\n".b, "i\xFA".b)
    assert_equal(-1, negative.duration)
    assert_raises(RangeError) { tree(negative) }

    threshold = forge(BM.parallel([BM.always_success], 1, 1), "i\x06".b, "i\n".b)
    assert_equal 5, threshold.success
    error = assert_raises(RangeError) { tree(threshold) }
    assert_equal 'Parallel threshold 5 must be between 1 and 1', error.message

    nested = BM.sequence(BM.always_success, BM.sleep(5).inverter)
    assert_raises(RangeError) { tree(forge(nested, "i\n".b, "i\xFA".b)) }
  end

  def test_node_classes_are_sealed
    BM::Node::CLASSES.each do |node|
      refute_respond_to node, :allocate
      assert_raises(TypeError) { Class.new(node) }
    end
    assert_raises(TypeError) { BM::Tree.allocate }
    assert_raises(TypeError) { Class.new(BM::Tree) }
  end

  def test_only_node_values_are_nodes
    impostor = Data.define(:children).new([BM.always_success])
    assert_raises(TypeError) { tree(impostor) }
    assert_raises(TypeError) { BM.sequence(impostor) }
    assert_raises(TypeError) { BM::Node::Inverter.new(Object.new) }
    assert_raises(NoMethodError) { BehaviorMachines.native_node(BM::Node::Sleep.new(1), [1]) }
    assert_raises(TypeError) { BehaviorMachines.__send__(:native_node, BM::Node::Sleep.new(1), 1) }
    assert_raises(TypeError) { BehaviorMachines.__send__(:native_node, Data.define(:a).new(1), [1]) }
    assert_raises(TypeError) { BehaviorMachines.__send__(:native_node, BM::Node::Sleep, [1]) }
  end

  # A node's kind is its exact class, asked natively: a redefined `class`
  # neither turns a Sleep into a Timeout nor lets an impostor pass.
  # Each test runs in its own forked cell, so the redefinition dies with it.
  def test_a_redefined_class_is_not_consulted
    BM::Node::Sleep.define_method(:class) { BM::Node::Timeout }
    BM::Node::Sleep.define_method(:instance_of?) { |_| false }

    assert_equal({ duration: 5 }, BM::Node::Sleep.new(5).to_h)
    assert_raises(RangeError) { BM::Node::Sleep.new(-1) }
    assert_raises(TypeError) { BM::Node::Sleep.new(1.5) }
  end

  # Each test runs in its own forked cell, so the redefinition dies with it.
  def test_redefined_readers_are_not_consulted
    node = BM.sequence(leaf('a'))
    BM::Node::Sequence.define_method(:children) { raise 'the binding must read the struct' }
    script = Script.new
    assert_equal :running, tree(node).tick(script, 0)
    assert_equal [tick('a')], script.log
  end

  def test_trees_and_nodes_build_after_compaction
    GC.compact
    script = Script.new
    subject = tree(BM.reactive_fallback(BM.always_failure, leaf('a').timeout(ms(5))))
    GC.compact
    assert_equal :running, subject.tick(script, 0)
    assert_equal ms(5), subject.next_deadline
    assert_equal :halted, subject.halt(script)
  end

  def test_symbol_tables_list_every_answer
    assert_equal %i[success failure running], BM::STATUSES
    assert_equal %i[halted already_idle], BM::HALTS
    assert_equal %i[full empty_composite parallel_threshold incomplete trailing time_regression], BM::REFUSALS
    assert [BM::STATUSES, BM::HALTS, BM::REFUSALS].all?(&:frozen?)
  end
end
