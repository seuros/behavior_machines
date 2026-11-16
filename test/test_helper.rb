# frozen_string_literal: true

$LOAD_PATH.unshift File.expand_path('../lib', __dir__)
require 'behavior_machines'

require 'set'
require 'minitest/autorun'

# MOCKED: scripted leaves and a recording context. Halting a leaf that is not
# active (never started, finished, or already halted) raises immediately, so
# every test also checks the halt contract.
module TestSupport
  class Script
    attr_reader :log

    def initialize
      @outcomes = {}
      @active = Set.new
      @log = []
    end

    def set(name, status) = @outcomes[name] = status
    def count(event) = @log.count(event)
    def index(event) = @log.index(event)
    def active?(name) = @active.include?(name)

    def assert_idle!
      raise "leaves left active: #{@active.to_a.inspect}" unless @active.empty?
    end

    def tick_leaf(name)
      @log << [:tick, name]
      status = @outcomes.fetch(name, :running)
      status == :running ? @active.add(name) : @active.delete(name)
      status
    end

    def halt_leaf(name)
      raise "halted inactive leaf #{name}" unless @active.delete?(name)

      @log << [:halt, name]
    end
  end

  class Scripted
    def initialize(name) = @name = name
    def tick(script, _now) = script.tick_leaf(@name)
    def halt(script) = script.halt_leaf(@name)
    def inspect = "#<Scripted #{@name}>"
  end

  # MOCKED: records every observer report.
  class Recorder
    attr_reader :reports

    def initialize = @reports = []
    def ticked(id, status) = @reports << [:ticked, id, status]
    def halted(id) = @reports << [:halted, id]
  end

  BM = BehaviorMachines

  def leaf(name) = BM.leaf(Scripted.new(name))
  def tree(root) = BM::Tree.new(root)
  def ms(value) = value * 1_000_000
  def tick(name) = [:tick, name]
  def halted(name) = [:halt, name]
  def parallel_all(*children) = BM.parallel(children, :all, 1)
  def race(*children) = BM.parallel(children, 1, 1)
  def parallel_any(*children) = BM.parallel(children, 1, :all)
end

module Minitest
  class Test
    include TestSupport
  end
end
