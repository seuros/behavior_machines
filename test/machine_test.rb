# frozen_string_literal: true

require_relative 'test_helper'
require 'state_machines'

class MachineTest < Minitest::Test
  # REAL: a state_machines-powered object shaped like a task lifecycle.
  class Job
    state_machine :state, initial: :pending do
      event(:start) { transition pending: :started }
      event(:complete) { transition started: :completed }
      event(:fault) { transition %i[pending started] => :failed }
      event(:cancel) { transition %i[pending started] => :cancelled }
    end
  end

  World = Data.define(:job, :deadline) do
    def deadline_passed? = deadline.call
  end

  # MOCKED: stands in for a timer or competing branch.
  class Deadline
    def tick(world, _now) = world.deadline_passed? ? :success : :running
  end

  def run_job
    BM.sequence(
      BM.leaf(BM::Machine::Fire.new(:job, :state, :start)),
      BM.leaf(BM::Machine::WaitFor.new(:job, :state, [:completed], %i[failed cancelled], :cancel))
    )
  end

  def world(passed: -> { false }) = World.new(job: Job.new, deadline: passed)

  def test_in_state_reports_whether_the_state_is_accepted
    leaf = BM::Machine::InState.new(->(world) { world.job }, :state, [:started])
    world = world()
    assert_equal :failure, leaf.tick(world, 0)
    world.job.start
    assert_equal :success, leaf.tick(world, 0)
  end

  def test_fire_succeeds_only_when_the_machine_accepts_the_event
    world = world()
    fire = BM::Machine::Fire.new(:job, :state, :start)
    assert_equal :success, fire.tick(world, 0)
    assert_equal :started, world.job.state_name
    assert_equal :failure, fire.tick(world, 0)
  end

  def test_tree_drives_the_job_to_completion
    world = world()
    tree = tree(run_job)

    assert_equal :running, tree.tick(world, 0)
    assert_equal :started, world.job.state_name
    world.job.complete
    assert_equal :success, tree.tick(world, 0)
  end

  def test_abandoned_wait_cancels_the_job_once
    passed = false
    world = world(passed: -> { passed })
    tree = tree(race(run_job, BM.leaf(Deadline.new)))

    assert_equal :running, tree.tick(world, 0)
    passed = true
    assert_equal :success, tree.tick(world, 0)
    assert_equal :cancelled, world.job.state_name
    assert_equal :already_idle, tree.halt(world)
    assert_equal :cancelled, world.job.state_name
  end

  def test_rejected_halt_event_leaves_a_finished_job_alone
    world = world()
    tree = tree(run_job)

    assert_equal :running, tree.tick(world, 0)
    world.job.complete
    assert_equal :halted, tree.halt(world)
    assert_equal :completed, world.job.state_name
  end

  def test_constructors_validate_their_arguments
    assert_raises(ArgumentError) { BM::Machine::InState.new(:job) }
    assert_raises(TypeError) { BM::Machine::Fire.new(42, :state, :start) }
    assert_raises(TypeError) { BM::Machine::InState.new(:job, :state, :started) }
    assert_raises(RangeError) { BM::Machine::InState.new(:job, :state, []) }
    assert_raises(TypeError) { BM::Machine::WaitFor.new(:job, :state, [:done], [], 'cancel') }
    assert_predicate BM::Machine::WaitFor.new(:job, :state, [:done], [], nil), :frozen?
  end

  def test_leaf_classes_are_sealed
    assert_equal [BM::Machine::InState, BM::Machine::Fire, BM::Machine::WaitFor], BM::Machine::CLASSES
    BM::Machine::CLASSES.each do |leaf|
      refute_respond_to leaf, :allocate
      assert_raises(TypeError) { Class.new(leaf) }
    end
  end
end
