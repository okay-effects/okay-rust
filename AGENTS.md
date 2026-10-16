# okay-rust — notes for agents

Two crates in one workspace: the root `okay` (okay-core's kernel ported — what follows down to "okay-effects") and
`effects/` (`okay-effects`: capabilities on async/await, okay-kernel's step protocol, middleware, okay's wire), with
`examples/shop` on the second. `cargo test --workspace` runs both. This repository was okay-rs; its history is here.

- Rust 2024 (rustc 1.92), no dependencies; `cargo test` runs the unit, integration and doc tests (the doc tests hold
  the `compile_fail` checks: a row not handled has no `run`; an effect not in the row cannot be performed).
- okay-core's kernel ported (`okay.one` in ~/work/my/okay-core is the specification; the okay-one repository is history): a program is DATA, `Eff<R, A>`, a tree of nodes; its
  effects a row `R` (`Cons<E, R>` … `Nil`, `row![…]`), an effect's place found by the compiler (`Member<E, Ix>`, the
  index `Here`/`There` never written); handlers values of three forms (`Handler::answering`, `stateful`, `deep`), not
  tied to the program; the rest `k` of a deep handler data (`K<T, B>`), resumed by the operation's `Reply`, as many
  times as wanted, each time with its levels as they were when taken — a state's included.
- An operation is an enum variant carrying its `Reply<T>`: `perform(Reader::Ask)`, `perform(|r| State::Put(s, r))`;
  a handler answers only through it (`r.give(v)`, `r.resume(&k, v)`), so the value's type is the operation's.
- The machine (`machine.rs`) is erased: values `Box<dyn Any>`, taken back at the type they were made at — the typed
  API allows no other (Rust has no GADTs: okay-one's typed loop is not portable). An operation goes to the nearest
  level of its effect by `TypeId`; an effect is once in a row (a second would make `Member`'s index ambiguous).
- Frames are `Fn`, not `FnOnce`: a `k` is multi-shot. So `eff! { data; … }` clones the names bound before each rest — a name
  bound is `Clone`.
- Tests are okay-core's kernel's, named after them (`tests/std.rs` = TestStd, `tests/acceptance.rs` = TestAcceptance); port the
  next ones the same way.
- INTERCEPTORS, the point of it all — control and audit of effects (`tests/intercept.rs` is the specification, the same programs as okay-core's TestIntercept):
  - an effect is `Plain` or `Sensitive` (`Effect::Kind`). A sensitive one is handled only by `handle_audited`, which
    needs `Audited<S>` in the row; only `audit` (an `Interceptor` of `S`) or `observe` puts it there, in the row
    outside them — where no program can perform `S`. A plain `handle`/`intercept` of it does not compile.
  - `intercept`: a plain effect handled again for a part of a program, its clauses outside it (their own operations
    reach the handler further out). `audit`: the same for a sensitive one, its clauses given `Pass<S>`, the only way
    to perform `S` past the audit — each operation told, passed on changed, or refused.
  - `observe`: an `Observer` sees every operation of every effect on its way out (`AnyOp`: its effect, `Debug`
    description), passes it on (`proceed`, `proceed_then` telling its answer each time the rest gets it) or refuses
    it. So an operation's value is `Clone + Debug`, and an effect's enum too.
  - the trail is an effect, `audit::Audit` (`record`, handled by `trail()`), outside the interceptors.
- Recursion: `call!(f(args))` (a tail call, `defer` underneath) and `tail_rec(s0, step)` (`Loop::Next`/`Done`) run
  any depth on the machine's heap stack (`tests/recursion.rs`, a million steps). A recursive call before its
  `flat_map` is Rust's own recursion: keep it inside a closure.
- `okay::typed` — ONE API compiled by the handlers' types: `eff! { … }` is an `async` block over a context `cx`
  (`Cx(h, rest)`, `Has<E, Ix>`), a program as data the machine's `eff! { data; … }`. An operation goes through each
  layer of the context (`Has::perform`/`resume`, typed by its value): `Paths` (`all`/`first`) logs the answers from
  further out and replays them (many-shot `k` by replay — the body an `AsyncFn` deciding by its effects only),
  `Observed` (`observe`) shows each to a `Watch`. Interceptors: `Audited::new(handler, interceptor)`, an
  `Interceptor<E, R, Ix>` passing each operation on (`Pass`), changing or refusing it, its own operations to the
  context after the handler (`Has<E, Through<Ix>>`); a sensitive effect has no other way into a context (`Allowed`,
  sealed). `tests/typed_intercept.rs` = `tests/intercept.rs`'s programs and trails: keep the three in step. `tail_rec` for recursion in constant stack. `effect!` declares an
  effect and its operations for both forms; `needs!` an async function over a context, its effects listed (`cx:
  [Reader<i64>, State<i64>]`): the context's type and an index per effect made (a pool of 16 names, `I0`…`I15`). `tests/replay.rs` checks the replay against the machine's values.
- Waiting on the world in `okay::typed`: `Step::Wait(future)` from a handler, polled with the program's `Context`
  (`Has::perform` gives `Got::{Now, Taken, Wait}`). Only an effect declared `waits` (`effect!`) has `Waiting`
  operations, which keep that future; `Perform` must keep NO field with a destructor — one costs 2–7× on the `T:`
  lanes (measured: not unwinding, not size; `ManuallyDrop` restores it). `block_on`, `both`; `tests/async.rs`.
- Benchmarks: `benches/kernel.rs`, okay-core's KernelBenchmark lanes, the same programs and n; run it under JMH's
  lock (`/tmp/okz/tools/withlock.py`) — other sessions measure on this machine — and compile under it too.
- Stage 2 is in (`src/cont.rs`): `Cont<R, I, O, A>`, `Control<T>`/`Delim<T>`/`Reset` in the row, `Nearest` (a shift0
  to the nearest `$`, never past one — `Control` is not an `Effect`, which keeps the impls apart), `Bind` for the four
  pairs, `shift0` (in `okay::cont` only: `okay::effects::shift0` is the effect's), `dollar`, `reset`, `Passing`
  (answering, stateful, translating — under a `$` no handler takes the rest), `PState`. The machine: `Level::Dollar`,
  `Kind::Shift0` (the levels from the nearest `$` up, the `$` with them, as `k`).
- The machine's stops: `machine::Outcome::{Done, Asked, Failed, Cancelled}` — an operation no level takes is ASKED of
  the host with every level as `k` (`Eff::step`, `step.rs`); `run` only for `Nil`. A panic is a `Raise` where it
  happened (the loop under `catch_unwind`; the heap stack untouched), unwound by the machine: `Level::Guard`
  (finalizer, recover, passing), `Level::Mask` (uncancelable), `Kind::{Raise, Cancel, Leave, Fail, CancelK, Passed}`.
  A state's step that panics loses that state (it was moved out): keep steps total.
- `okay::wire` (`Remote`, `Remotes`, `program`, `serve`; `effect!`'s `wire` flag) and `okay::data` (`Data`, `IsData`)
  — okay-effects takes its `Data` from here.
- Checks beyond the ported suites: `src/reference.rs` (cfg(test): normal forms, random programs of effects and of
  control, the machine as the meaning — run it after any change to the machine), `tests/tck.rs` (okay-kernel's
  digest; the TCK's state a cell, as its `Handlers.state`), A/B the lanes under the lock after machine changes.
  `Bind` impls for the four pairs of `Eff`/`Cont` (no overlap, so no specialization); `on_unimplemented` messages.
- No casts outside `take`/`take_op`/`state` (the erased machine's three); no panics but theirs and the machine's
  "a row not checked", which a typed program never reaches.
- Commits: no Co-Authored-By lines.

## okay-effects (`effects/`)

- Edition 2021, no dependencies. An operation implements `Op` (`op!` writes it with its form as data, `Operation`:
  a name in snake_case and a record, fields in lowerCamel — a Scala case class's shape); a handler (`handle`,
  `answering`) makes a `Cap<O>` and answers its operations by identity; `perform(op).await` puts the operation in the
  run's mailbox (one in flight per run) and returns `Pending`; the handler polling its body answers it.
- `Reply::Abort` drops the body (destructors run); `Reply::Raise` and `Computation::fail` are panics at the operation.
- `start`/`Computation`: the kernel's protocol (`Step::{Done, Yielded(Asked), Failed, Cancelled}`); `drive` with an
  `Answerer`, `with`/`only`/`Routes`/`record`/`replay`; `every_path` by replay; `wire::{remote, serve}`.
- Branches (`par.rs`): `all`/`par`/`race` poll their branches round-robin in one run, one mailbox; an operation
  left in it past the branches locks them to the branch that performed it until answered (one at a time, no
  lending). A branch's panic drops the others. Not `join!`/`select!`: they don't know the mailbox. `Host::run` for
  branches in a handed-out program.
- `stream(producer)` (`stream.rs`): `Cap<Emit<T>>`, pulled as an `Iterator`; dropping it drops the producer.
- `effects/benches/ops.rs` (harness = false, std only): `cargo bench -p okay-effects` — only okay-effects', not
  the root's JMH-locked `benches/kernel.rs`; numbers in the guide's "Speed".
- `examples/okayhost/TestOkayRustWire.scala` is run from an okay scratch worktree (okay-py, a Live test), not here.
- Keep the guide (`docs/guide.md`) and the README's snippets in step with the tests.
