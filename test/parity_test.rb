# frozen_string_literal: true

require_relative 'test_helper'
require_relative 'lifecycle'

# Runs identical scenarios on both backends and requires identical leaf calls,
# statuses, deadlines, and observer reports. Needs the native extension.
class ParityTest < Minitest::Test
  # MOCKED: raises from tick once armed.
  class Tripwire
    attr_writer :armed

    def tick(_ctx, _now) = @armed ? raise(ArgumentError, 'trip') : :running
  end

  # MOCKED: a script whose leaves can also raise, when scripted as :raise.
  class RaisingScript < Script
    def tick_leaf(name)
      raise ArgumentError, "#{name} raised" if @outcomes[name] == :raise

      super
    end
  end

  # MOCKED: ticks whatever it was given, status or not.
  class BadLeaf
    def initialize(status) = @status = status
    def tick(_ctx, _now) = @status
    def inspect = '#<BadLeaf>'
  end

  INVALID_UTF8_SYMBOL = [255].pack('C').b.to_sym
  BAD_STATUSES = [INVALID_UTF8_SYMBOL, :bogus, 'success', nil].freeze

  def setup
    skip 'native extension not loaded' unless BM.native_available?
  end

  def test_a_bad_status_raises_identically_on_both_backends
    BAD_STATUSES.each do |bad|
      runs = %i[ruby native].map do |backend|
        tree = BM::Tree.new(BM.leaf(BadLeaf.new(bad)), backend:)
        assert_raises(BM::InvalidStatusError) { tree.tick(nil, now: 0) }.message
      end
      assert_equal runs[0], runs[1], "status=#{bad.inspect}"
    end
  end

  def test_every_shape_behaves_identically_on_both_backends
    Lifecycle.shapes.each_index do |shape|
      27.times do |first|
        27.times do |second|
          runs = %i[ruby native].map do |backend|
            script = Script.new
            recorder = Recorder.new
            tree = BM::Tree.new(Lifecycle.shapes[shape], backend:)
            trace = [[0, first], [1, second], [2, first]].map do |at, code|
              Lifecycle.script(script, code)
              [tree.tick(script, now: ms(at), observer: recorder), tree.next_deadline]
            end
            tree.halt(script, observer: recorder)
            [trace, script.log, recorder.reports]
          end
          assert_equal runs[0], runs[1], "shape=#{shape} first=#{first} second=#{second}"
        end
      end
    end
  end

  def test_every_shape_handles_a_raising_leaf_identically_on_both_backends
    Lifecycle.shapes.each_index do |shape|
      27.times do |first|
        %w[a b c].each do |raiser|
          9.times do |others|
            runs = %i[ruby native].map do |backend|
              script = RaisingScript.new
              recorder = Recorder.new
              tree = BM::Tree.new(Lifecycle.shapes[shape], backend:)
              Lifecycle.script(script, first)
              first_status = tree.tick(script, now: 0, observer: recorder)
              Lifecycle.script(script, others)
              script.set(raiser, :raise)
              outcome = begin
                tree.tick(script, now: ms(1), observer: recorder)
              rescue ArgumentError => e
                e.message
              end
              [first_status, outcome, script.log, recorder.reports, tree.running?]
            end
            assert_equal runs[0], runs[1], "shape=#{shape} first=#{first} raiser=#{raiser} others=#{others}"
          end
        end
      end
    end
  end

  def test_a_raising_leaf_cleans_up_identically_on_both_backends
    runs = %i[ruby native].map do |backend|
      script = Script.new
      recorder = Recorder.new
      tripwire = Tripwire.new
      root = BM.parallel(leaf('a'), BM.sequence(leaf('gate'), BM.leaf(tripwire)), leaf('c'))
      tree = BM::Tree.new(root, backend:)

      tree.tick(script, now: 0, observer: recorder)
      script.set('gate', :success)
      tripwire.armed = true
      error = assert_raises(ArgumentError) { tree.tick(script, now: 1, observer: recorder) }
      [error.message, script.log, recorder.reports, tree.running?]
    end
    assert_equal runs[0], runs[1]
  end
end
