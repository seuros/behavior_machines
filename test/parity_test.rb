# frozen_string_literal: true

require_relative 'test_helper'
require_relative 'lifecycle'
require_relative 'support/reference'

# Runs identical scenarios on the native tree and the reference model and
# requires identical leaf calls, statuses, deadlines, and observer reports.
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
  end

  INVALID_UTF8_SYMBOL = [255].pack('C').b.to_sym
  BAD_STATUSES = [INVALID_UTF8_SYMBOL, :bogus, 'success', nil].freeze

  # The native tree and the reference model of the same root.
  def both(root) = [BM::Tree.new(root), Reference::Engine::Tree.new(root)]

  # The reference model's calls, spelled like the native tree's.
  def tick_on(subject, ctx, now, observer = nil)
    return subject.tick_observed(ctx, now, observer) if subject.is_a?(BM::Tree) && observer
    return subject.tick(ctx, now) if subject.is_a?(BM::Tree)

    subject.tick(ctx, now, observer)
  end

  def halt_on(subject, ctx, observer = nil)
    return subject.halt_observed(ctx, observer) if subject.is_a?(BM::Tree) && observer
    return subject.halt(ctx) if subject.is_a?(BM::Tree)

    subject.halt(ctx, observer)
  end

  def test_a_bad_status_raises_the_same_class_on_both
    BAD_STATUSES.each do |bad|
      classes = both(BM.leaf(BadLeaf.new(bad))).map do |subject|
        assert_raises(TypeError, RangeError) { tick_on(subject, nil, 0) }.class
      end
      assert_equal classes[0], classes[1], "status=#{bad.inspect}"
    end
  end

  def test_every_shape_behaves_identically_on_both
    Lifecycle.shapes.each_index do |shape|
      27.times do |first|
        27.times do |second|
          runs = both(Lifecycle.shapes[shape]).map do |subject|
            script = Script.new
            recorder = Recorder.new
            trace = [[0, first], [1, second], [2, first]].map do |at, code|
              Lifecycle.script(script, code)
              [tick_on(subject, script, ms(at), recorder), subject.next_deadline]
            end
            halt_on(subject, script, recorder)
            [trace, script.log, recorder.reports]
          end
          assert_equal runs[0], runs[1], "shape=#{shape} first=#{first} second=#{second}"
        end
      end
    end
  end

  def test_every_shape_handles_a_raising_leaf_identically_on_both
    Lifecycle.shapes.each_index do |shape|
      27.times do |first|
        %w[a b c].each do |raiser|
          9.times do |others|
            runs = both(Lifecycle.shapes[shape]).map do |subject|
              script = RaisingScript.new
              recorder = Recorder.new
              Lifecycle.script(script, first)
              first_status = tick_on(subject, script, 0, recorder)
              Lifecycle.script(script, others)
              script.set(raiser, :raise)
              outcome = begin
                tick_on(subject, script, ms(1), recorder)
              rescue ArgumentError => e
                e.message
              end
              [first_status, outcome, script.log, recorder.reports, subject.running?]
            end
            assert_equal runs[0], runs[1], "shape=#{shape} first=#{first} raiser=#{raiser} others=#{others}"
          end
        end
      end
    end
  end

  def test_a_raising_leaf_cleans_up_identically_on_both
    runs = [BM::Tree, Reference::Engine::Tree].map do |kind|
      script = Script.new
      recorder = Recorder.new
      tripwire = Tripwire.new
      root = parallel_all(leaf('a'), BM.sequence(leaf('gate'), BM.leaf(tripwire)), leaf('c'))
      subject = kind.new(root)

      tick_on(subject, script, 0, recorder)
      script.set('gate', :success)
      tripwire.armed = true
      error = assert_raises(ArgumentError) { tick_on(subject, script, 1, recorder) }
      [error.message, script.log, recorder.reports, subject.running?]
    end
    assert_equal runs[0], runs[1]
  end
end
