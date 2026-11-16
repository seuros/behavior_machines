# behavior_machines

Behavior trees with explicit halt semantics for Ruby, following the
behavior-tree pattern. Thin bindings to the allocation-free
[behavior-machines](ext/behavior_machines_native/core) Rust core; the native
extension is required.

Each tick advances as far as the leaves allow. Every running leaf the tree
abandons, because a parallel decided, a reactive guard flipped, a timeout
expired, or the tree was halted, gets `halt` called exactly once, in the same
call. Leaves can therefore own real resources and release them reliably.

Requires Ruby 4.0 or newer and a Rust toolchain to build.

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

now = Process.clock_gettime(Process::CLOCK_MONOTONIC, :nanosecond)
tree.tick(robot, now)  # => :running, charging
tree.halt(robot)       # => :halted; Charge#halt ran exactly once
tree.halt(robot)       # => :already_idle
```

## Nodes

Every node is a `Data` value mirroring the core's `Node`, built with the
module functions or the classes under `BM::Node`, positionally, in the core's
order. Each validates through the core when built: TypeError for the wrong
type, RangeError for a value the core refuses (an empty composite, a parallel
threshold outside its child count, a negative duration).

- **Leaves**: any object responding to `tick(ctx, now)`, returning
  `:success`, `:failure`, or `:running`, and optionally `halt(ctx)`. Built-in:
  `BM.sleep(duration)`, `BM.always_success`, `BM.always_failure`.
- **Control**: `BM.sequence`, `BM.reactive_sequence`, `BM.fallback`,
  `BM.reactive_fallback`, `BM.parallel(children, success, failure)`,
  `BM.if_then_else(condition, then_branch, otherwise)`, `BM.if_then`,
  `BM.while_do_else(condition, body, otherwise)`, `BM.while_do`.
  Thresholds are `:all` or an Integer; `otherwise` is a node or nil.
- **Decorators**, on any node: `inverter`, `force_success`, `force_failure`,
  `timeout(duration)`, `repeat(limit)`, `retry_until_successful(limit)`,
  `keep_running_until_failure`. Limits are an Integer or `:unlimited`; looping
  decorators finish at most one child run per tick.

A tree holds at most `BM::Tree::CAPACITY` (256) nodes.

## Time

Every instant and span is Integer nanoseconds on a monotonic clock the caller
picks, passed positionally: `tree.tick(ctx, now)`. `tree.next_deadline`
returns when a running `sleep` or `timeout` next needs a tick, so a host can
sleep until then. A looping decorator that finished a run reports that tick's
time, so its next run is due at once.

A tick earlier than the latest honoured one is refused: it answers
`:time_regression`, runs no leaf and records nothing. Refusals are Symbols and
never raise.

## Errors

If a leaf or observer raises during a tick, no more leaf code runs in that
tick: active leaves stay active and idle ones don't start. Every leaf still
active is then halted, without observer reports, and the first error
propagates. The raising leaf counts as finished. During a halt, every active
leaf is halted even if a leaf's `halt` or the observer raises, and the first
error propagates afterwards.

A leaf returning a non-Symbol raises TypeError, another Symbol RangeError. A
leaf calling back into the tree that is ticking it raises RuntimeError. A
frozen tree raises FrozenError on `tick` and `halt`.

## Observers

`tree.tick_observed(ctx, now, observer)` and `tree.halt_observed(ctx,
observer)` report `ticked(node_id, status)` and `halted(node_id)` for every
node, children before parents, to whichever hooks the observer implements.
Node ids number the definition in pre-order.

## state_machines

`BehaviorMachines::Machine` has leaves for objects powered by the
[state_machines](https://github.com/state-machines/state_machines) gem:

```ruby
job_leaves = BM.sequence(
  BM.leaf(BM::Machine::Fire.new(:job, :state, :start)),
  BM.leaf(BM::Machine::WaitFor.new(:job, :state, [:completed], [:failed], :cancel))
)
```

`InState` checks the state, `Fire` fires an event, and `WaitFor` waits for a
machine something else drives, firing its last argument if the tree abandons
it (nil for none).

## Releases

Release Please manages the Ruby gem and both publishable Rust crates. Use
conventional commits and merge its release PR to update versions, changelogs,
the release manifest, and workspace dependency requirements. Do not push
hand-written version bumps or changelog entries.

The release manifest records the last released version of each package, not
the next planned version.

Crate-only releases do not trigger Ruby gem publishing. The workflow creates
GitHub releases for the crates but does not upload them to crates.io. After
merging the release PR and checking out its release tag, run the Rust tests
and a `cargo publish --dry-run` for the relevant crate before publishing.

## 📡 Credits

**AMS Isla**, NCC-2005, *Isla*-class flight computer.

Named for Damian Isla's *Handling Complexity in the Halo 2 AI* (GDC 2005), the
talk that brought behavior trees out of one studio and into the trade: a
hierarchy that decides what to do next and knows what to stop when it changes
its mind.

## License

BSD-3-Clause. See [LICENSE](LICENSE).
