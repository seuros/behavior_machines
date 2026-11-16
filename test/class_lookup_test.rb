# frozen_string_literal: true

require_relative 'test_helper'
require 'open3'
require 'rbconfig'

# The class of the value the extension wraps (Tree) is fixed when it loads.
# Nothing a caller does later, redefining the constant or `Object.const_get`,
# reaches a tree being built. Each case runs in a fresh process: the class is
# fixed once per process, at load.
class ClassLookupTest < Minitest::Test
  LIB = File.expand_path('../lib', __dir__)

  PRELUDE = <<~RUBY
    require 'behavior_machines'
    BM = BehaviorMachines
    ROOT = BM.sequence(BM.always_success)
  RUBY

  # Runs `body` after PRELUDE in a new Ruby; answers its last line of output.
  def isolated(body)
    out, err, status = Open3.capture3(RbConfig.ruby, '-I', LIB, '-e', PRELUDE + body)
    assert status.success?, "exited #{status.exitstatus}: #{err}"
    out.lines.last&.chomp
  end

  # Bug: a Tree constant redefined before the first tree made it an instance
  # of the new class, without any of the tree's methods.
  def test_a_redefined_tree_constant_does_not_change_the_tree_class
    assert_equal 'true success 2', isolated(<<~RUBY)
      original = BM::Tree
      BM.send(:remove_const, :Tree)
      BM.const_set(:Tree, Class.new)
      tree = original.new(ROOT)
      puts [tree.instance_of?(original), tree.tick(nil, 0), tree.size].join(' ')
    RUBY
  end

  # Bug: building the first tree ran `Object.const_get`, a caller's Ruby code
  # in the middle of a native call.
  def test_building_a_tree_runs_no_const_get
    assert_equal '0 Tree', isolated(<<~RUBY)
      hits = 0
      Object.define_singleton_method(:const_get) do |*args|
        hits += 1
        super(*args)
      end
      tree = BM::Tree.new(ROOT)
      puts [hits, tree.class.name.split('::').last].join(' ')
    RUBY
  end

  # Bug: a redefined `Object.const_get` that raised hit an `unwrap` in the
  # extension: an uncatchable fatal, and the process exited.
  def test_a_raising_const_get_does_not_kill_the_process
    assert_equal 'success', isolated(<<~RUBY)
      Object.define_singleton_method(:const_get) { |*| raise IOError, 'lookup ran mid-call' }
      puts BM::Tree.new(ROOT).tick(nil, 0)
    RUBY
  end
end
