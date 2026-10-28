# behavior_machines

Behavior trees with explicit halt semantics for Ruby, following the
behavior-tree pattern. Pure Ruby, with an optional native backend built on the
[behavior-machines](ext/behavior_machines_native/core) Rust crate.

Each tick advances as far as the leaves allow. Every running leaf the tree
abandons, because a parallel decided, a reactive guard flipped, a timeout
expired, or the tree was halted, gets `halt` called exactly once, in the same
call. Leaves can therefore own real resources and release them reliably.

Requires Ruby 4.0 or newer.

```ruby
require 'behavior_machines'

BM = BehaviorMachines

class BatteryLow
  def tick(robot, _now) = robot.battery < 20 ? :success : :failure
end

class Charge
  def tick(robot, _now)
    return :success if robot.battery == 100

    robot.charging = true
    :running
  end

  def halt(robot) = robot.charging = false
end

class Patrol
  def tick(_robot, _now) = :running
end

tree = BM::Tree.new(
  BM.fallback(
    BM.sequence(BM.leaf(BatteryLow.new), BM.leaf(Charge.new)),
    BM.leaf(Patrol.new)
  )
)

tree.tick(robot)  # => :running, charging
tree.halt(robot)  # Charge#halt runs exactly once
```

## Nodes

- **Leaves**: any object responding to `tick(ctx, now)`, returning
  `:success`, `:failure`, or `:running`, and optionally `halt(ctx)`. Built-in:
  `BM.sleep(seconds)`, `BM.always_success`, `BM.always_failure`.
- **Control**: `BM.sequence`, `BM.reactive_sequence`, `BM.fallback`,
  `BM.reactive_fallback`, `BM.parallel(*children, success: :all, failure: 1)`,
  `BM.if_then_else(condition, then_branch, otherwise = nil)`,
  `BM.while_do_else(condition, body, otherwise = nil)`.
- **Decorators**, on any node: `inverter`, `force_success`, `force_failure`,
  `timeout(seconds)`, `repeat(limit)`, `retry_until_successful(limit)`,
  `keep_running_until_failure`. Limits are an Integer or `:unlimited`; looping
  decorators finish at most one child run per tick.

Node definitions are immutable `Data` values; `BM::Tree.new` validates and
compiles them.

## Time

Durations are seconds. The clock is an Integer count of nanoseconds, passed as
`now:` and defaulting to `BehaviorMachines.now` (monotonic).
`tree.next_deadline` returns when a running `sleep` or `timeout` next needs a
tick, so a host can sleep until then. A looping decorator that finished a run
reports that tick's time, so its next run is due at once.

## Errors

If a leaf or observer raises during a tick, no more leaf code runs in that
tick: active leaves stay active and idle ones don't start. Every leaf still
active is then halted, without observer reports, and the first error
propagates. The raising leaf counts as finished. During a halt, every active
leaf is halted even if a leaf's `halt` or the observer raises, and the first
error propagates afterwards.

A leaf returning anything but the three statuses raises
`BehaviorMachines::InvalidStatusError`. A leaf calling back into the tree
that is ticking it raises `BehaviorMachines::ReentrantCallError`. Non-local
exits from a leaf, such as `throw` or `break`, are not supported.

Both backends follow these rules identically; the test suite compares them
side by side, including with raising leaves.

## Observers

`tree.tick(ctx, observer:)` reports `ticked(node_id, status)` and
`halted(node_id)` for every node, children before parents. Node ids number the
definition in pre-order.

## state_machines

`BehaviorMachines::Machine` has leaves for objects powered by the
[state_machines](https://github.com/state-machines/state_machines) gem:

```ruby
job_leaves = BM.sequence(
  BM.leaf(BM::Machine::Fire.new(:job, :start)),
  BM.leaf(BM::Machine::WaitFor.new(:job, success: :completed, failure: :failed, on_halt: :cancel))
)
```

`InState` checks the state, `Fire` fires an event, and `WaitFor` waits for a
machine something else drives, firing `on_halt` if the tree abandons it.

## Native backend

The native backend runs the same trees on the Rust engine. It is opt-in:

```console
$ bundle exec rake compile
$ BEHAVIOR_MACHINES_NATIVE=1 ruby app.rb
```

`BM::Tree.new(root, backend: :ruby | :native | :auto)` picks one explicitly.
Leaves stay Ruby, so the gain is limited to the tree walk itself. The test
suite runs against both backends (`bundle exec rake test:both`).

## Releases

Release Please manages the Ruby gem and both publishable Rust crates. Use
conventional commits and merge its release PR to update versions, changelogs,
the release manifest, and workspace dependency requirements. Do not push
hand-written version bumps or changelog entries.

The release manifest records the last released version of each package, not
the next planned version. Circuit is registered at its published `0.1.0`;
its one-time `release-as` override requests `0.3.0` in the next release PR.
Remove that override from `release-please-config.json` after the release so
later versions are calculated from conventional commits.

Crate-only releases do not trigger Ruby gem publishing. The workflow creates
GitHub releases for the crates but does not upload them to crates.io. After
merging the release PR and checking out its release tag, run the Rust tests
and a `cargo publish --dry-run` for the relevant crate before publishing.

## License

BSD-3-Clause. See [LICENSE](LICENSE).
