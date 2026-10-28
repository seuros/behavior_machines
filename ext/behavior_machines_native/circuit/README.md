# behavior-machines-circuit

Compiles a [behavior-machines](../core) tree into a branchless arithmetic
circuit. A pure tick becomes a fixed, layered evaluation of affine forms and
rectified gates instead of a recursive walk. A staged executor also supports
effectful actions without running unreachable leaves.

```rust
use behavior_machines::{Node, Status};
use behavior_machines_circuit::{Lowered, State};

// A reactive fallback: charge while the battery is low, otherwise patrol.
let tree = Node::reactive_fallback([
    Node::sequence([Node::leaf("battery-low"), Node::leaf("charge")]),
    Node::leaf("patrol"),
]);

let lowered = Lowered::new(&tree)?;
let mut state = State::initial(&lowered);

// One status per leaf, in leaf order, evaluated by the host.
let schedule = lowered.evaluate(&mut state, &[
    Status::Failure, // battery-low
    Status::Success, // charge
    Status::Running, // patrol
])?;

assert_eq!(schedule.root(), Some(Status::Running));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## What it is for

**Not speed on a single tree.** The engine's walk is already a few hundred
lines of flat Rust over a slot arena; a tens-of-nodes tree evaluated as a
circuit is a wash at best. Three things do pay:

- **A batch axis.** The circuit is the same shape for every evaluation, so *N*
  agents sharing one tree is one wide evaluation rather than *N* walks. This
  is where the prior art finds its wins, and it needs *N* to be large.
- **Constant-time ticks.** A walk's duration depends on the path taken; the
  circuit's is a fixed number of layers. For a real-time or embedded host with
  a deadline to prove, a worst case that equals the average case is worth
  paying for.
- **A differentiable tree.** Affine forms and rectified gates are what a
  neural net is made of, so the arbitration has a gradient. Freezing the
  structure while training the thresholds — a `Parallel` threshold that tunes
  itself, a `Fallback` that learns its priority order — is the interesting
  experiment, and the published work on trainable trees is all on *decision*
  trees, not behavior trees.

A fourth, immediate benefit: the circuit backend has to agree with the engine
on every tick, which makes it a test that arbitration is pure and that effects
live only in leaves.

## Pure, single-pass evaluation

The circuit needs every leaf's status before it can say which leaves the tick
reaches, so the host evaluates all of them up front. That is only sound when
ticking a leaf has no effect beyond its returned status — a condition, not an
action.

An ordered composite ticks its next child only if the previous one succeeded,
*within the same tick*, so one circuit evaluation cannot both sequence effects
and compute the arbitration. Keep using `Lowered::evaluate` for pure,
single-pass evaluation; use `Executor` for effects.

## Effectful, staged execution

`Executor` accepts a `Node<ExecutionLeaf<C, A>>`:

- `Condition` evaluates a read-only host context and must have no observable
  effects, including through interior mutability.
- `Action` has `start`, `poll`, and `halt` methods over mutable host context.
  Definitions stay immutable; activation state and task handles live in that
  context.
- `Executor::tick(ctx, now)` returns the final `Schedule`, **after** performing
  its effects. Do not execute the returned schedule a second time.
- `Executor::halt(ctx)` cancels active actions exactly once and resets memory
  cursors. Call it before dropping a running executor; `Drop` cannot access
  the host's context.

Unresolved leaves provisionally return `Running`, so the circuit identifies
the next reachable leaf without executing any later one. The executor runs that
leaf, incorporates its status, and evaluates again from the beginning-of-tick
state. Each reached leaf runs once per tick; only the final evaluation commits
state. Post-order processing interleaves ticks and halts, ensuring an earlier
subtree cancels its abandoned action before a later sibling starts.

This preserves the regular engine's same-tick sequence/fallback advancement,
memory cursors, reactive preemption, and fresh activation after root completion.
Even conditions are evaluated only when reached, so they see effects from
earlier actions in the same tick. An action returning `Running` is polled on
the next host tick, not repeatedly during internal evaluation.

The number of circuit evaluations is bounded by `leaf_count + 1`, but depends
on the reached path. **The effectful executor does not promise fixed-cost
ticks or a speedup over the regular engine.** The fixed-shape, single-pass
benefit still belongs to pure evaluation.

There is no async runtime, clock, thread, or I/O dependency. Actions must return
promptly; the host starts work, delivers completions, and schedules later ticks.
`halt` requests cancellation and should invalidate activation tickets to reject
late completions. Circuit evaluation errors halt active actions and reset state;
panics in host methods are not caught.

See [`examples/inference.rs`](examples/inference.rs) for a fake inference
operation guarded by a reactive resource check, including cancellation,
resumption, and stale-result rejection:

```console
cargo run -p behavior-machines-circuit --example inference
```

## Supported nodes

Lowered: `Leaf`, `AlwaysSuccess`, `AlwaysFailure`, `Sequence`, `Fallback`,
`ReactiveSequence`, `ReactiveFallback`, `Inverter`, `ForceSuccess`,
`ForceFailure`.

Everything else returns `LowerError::Unsupported` with the node's name:
`Parallel`, `IfThenElse`, `WhileDoElse`, `Sleep`, `Timeout`, `Repeat`,
`RetryUntilSuccessful`, `KeepRunningUntilFailure`. `Node` is
`#[non_exhaustive]`, so a node added to the engine later lands there too
rather than compiling to something wrong by accident.

Both the pure evaluator and effectful executor enforce this node set.

The timed nodes need care beyond arithmetic: the engine's clock is `u64`
nanoseconds and `f32` holds integers exactly only below 2^24, so a deadline
cannot enter a circuit as an absolute time. They will take **relative**
deadlines, with `next_deadline` staying integer math on the host.

## How it works

Three passes, compiled by [`Lowered::new`]:

1. **Flatten** the definition in the engine's pre-order, so a node's index is
   its `NodeId` and every descendant sits at a higher index.
2. **Arbitrate** bottom-up: each node's *raw* status, meaning what it would
   return if it were ticked. This depends only on children's raw statuses and
   the node's own state, never on whether the node is reached.
3. **Reach** top-down: which nodes the tick visits, and which running nodes it
   halts.

Separating "what would it return" from "is it reached" is what removes the
recursion. A walk interleaves the two; split apart, each becomes a layer of
arithmetic.

Both ordered composites reduce to one identity — the walk passes over a child
or stops at it:

```text
before_i  = [i < cursor]                  the cursor already passed this child
blocked_i = NOT (before_i OR advance_i)   the walk cannot get past child i
clear_i   = [Σ_{j<i} blocked_j == 0]      every earlier child was passed
reach_i   = clear_i AND NOT before_i      so this child is ticked
stop_i    = clear_i AND blocked_i         and the walk stops here
```

`blocked` depends only on the cursor and each child's own status, never on the
other children, so the whole vector exists before any prefix test runs. `clear`
is then one `none_earlier` call shared by both answers, and `Σ blocked_j` is a
non-negative integer, so the test against zero is a **single** rectified gate,
`max(0, 1 - Σ)`. `stop` is at-most-one-hot by construction, so every collapse
over it is plain addition and costs no operations at all. A reactive composite
is the same lowering with the cursor pinned to zero, which constant-folds
`before_i` away.

The consequence is the measured cost, which `lower/tests.rs` pins:

| | depth | operations per child |
|---|---|---|
| reactive composite | 5, any width | 8 |
| memory composite | 7, any width | 12 |

**Depth does not grow with the number of children.** A prefix test reads a sum
of bits and a sum is free, so a wider composite widens a layer rather than
adding one — four children and sixty-four children cost the same number of
sequential steps. Depth grows only with nesting, about two layers per level.

The status-rewrite decorators are free: `Inverter` relabels two wires and the
`Force` pair merges two disjoint bits.

## Exactness

Every value in a lowered tree is an integer — a bit, a child index, or a count
of children — and all of them stay far below 2^24, where `f32` is exact. No
operation ever rounds, which is checked under `debug_assertions`.

This is a property of the lowering rather than a tolerance. Note what it rests
on: conditions are compared by the *host*, at full precision, and enter the
circuit already decided as bits. The circuit never compares a float, so there
is no threshold for a rounding error to land on.

## Validation

The October 28, 2026 checkpoint was validated with Rust 1.99.0:

- 64 circuit tests and one doctest, including effect-order parity against
  the regular engine over all status scripts/classifications in the fixtures.
- 61 core tests and one doctest with all features.
- Strict all-target circuit Clippy, formatting and warning-free Rustdoc.
- The inference example and bare-metal `thumbv7em-none-eabihf` builds of
  the circuit and all-feature core.

## License

BSD-3-Clause, as the rest of the repository.
