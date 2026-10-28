# okay-rust — notes for agents

Two crates. `okay` (the root): THE ONE API — `okay::typed`, `okay::host`, the effects' enums, `Data`, `Remote`,
`Reply`/`Answer`/`Val`, the macros `effect!`, `needs!`, `eff!` (async), `data!`. `okay-machine` (`machine/`): its
SPECIFICATION — okay-core's kernel (`Eff`, the machine, rows, `Cont`, the reference interpreter, the TCK), its own
`effect!` (okay's, plus the `Eff` forms) and `eff! { data; … }`, `Resume` (`r.resume(&k, v)`); it re-exports `typed`,
`host`, `needs`, `data`; its tests in `machine/tests`, the bench `machine/benches/kernel.rs` (both crates' lanes) with
`count-allocs`. `examples/shop` on the one API. `cargo test --workspace` runs all. This repository was okay-rs.
Removed 2026-10-27 (the one API took over; git history keeps them): okay-effects, `okay::direct`, the one API's replay
(ONE-SHOT by design: the rest taken many times is a resumable fn's or the machine's) and `okay::dynamic`.
`okay-resumable` (`resumable/`, a proc macro, syn/quote): `#[resumable(Ops)]` — the rest taken many times, compiled,
stable; the core (`Rest`, `Step`, `OpOf`, `ops!`, `ask!`, `co`/`coroutine!` under `--features coroutines`, nightly) is
`okay::resumable`. NEVER a replay (the user's rule 2026-10-28): multi-shot is a rest that is a `Clone` value.
Its generator (`resumable/src/lib.rs`): `find` collects the stops (variables in scope, the blocks around each);
`gen_rest(k)` writes stop k's resumption — the rest of each block around it innermost first, a loop's remaining body
under labels `'okay_c{id}` (continue) / `'okay_b{id}` (break) then the loop written out again. Tests in
`resumable/tests`, the `compile_fail` errors in its crate docs, the bench `resumable/benches/lanes.rs`
(`cargo +nightly bench -p okay-resumable --features coroutines --bench lanes`).

- Rust 2024 (rustc 1.92), no dependencies but the optional `mimalloc` (feature `mimalloc`: the global allocator); `cargo test` runs the unit, integration and doc tests (the doc tests hold
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
  layer of the context (`Has::perform`/`resume`, typed by its value): ONE-SHOT: the rest taken many times is the machine's
  (no replay, the user's decision 2026-10-27), `Observed` (`observe`) shows each to a `Watch`. Interceptors: `Audited::new(handler, interceptor)`, an
  `Interceptor<E, R, Ix>` passing each operation on (`Pass`), changing or refusing it, its own operations to the
  context after the handler (`Has<E, Through<Ix>>`); a sensitive effect has no other way into a context (`Allowed`,
  sealed). `tests/typed_intercept.rs` = `tests/intercept.rs`'s programs and trails: keep the three in step. `tail_rec` for recursion in constant stack. `effect!` declares an
  effect and its operations for both forms; `needs!` an async function over a context, its effects listed (`cx:
  [Reader<i64>, State<i64>]`): the context's type and an index per effect made (a pool of 16 names, `I0`…`I15`). `tests/replay.rs` checks the replay against the machine's values.
- Waiting on the world in `okay::typed`: `Step::Wait(future)` from a handler, polled with the program's `Context`
  (`Has::perform` gives `Got::{Now, Taken, Wait}`). Only an effect declared `waits` (`effect!`) has `Waiting`
  operations, which keep that future; `Perform` must keep NO field with a destructor — one costs 2–7× on the `T:`
  lanes (measured: not unwinding, not size; `ManuallyDrop` restores it). `block_on`, `both`; `tests/async.rs`.
- Benchmarks: `machine/benches/kernel.rs` (`cargo bench -p okay-machine`), okay-core's KernelBenchmark lanes, the same programs and n; run it under JMH's
  lock (`scripts/withlock.py`) — other sessions measure on this machine — and compile under it too. A/B two bench
  binaries, alternating: `scripts/bench-ab.sh <binA> <binB> <rounds> [lanes]`; append the medians to
  `machine/benches/history.tsv` (never rewrite it). `--features count-allocs`: allocations per run per lane. The plan of the
  one API (typed as the API, contexts as backends, the machine as the specification): `specs/one-api.md`.
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
- Checks beyond the ported suites: `src/reference.rs` (cfg(test): normal forms, random programs of effects and of
  control, the machine as the meaning — run it after any change to the machine), `tests/tck.rs` (okay-kernel's
  digest; the TCK's state a cell, as its `Handlers.state`), A/B the lanes under the lock after machine changes.
  `Bind` impls for the four pairs of `Eff`/`Cont` (no overlap, so no specialization); `on_unimplemented` messages.
- No casts outside `take`/`take_op`/`state` (the erased machine's three); no panics but theirs and the machine's
  "a row not checked", which a typed program never reaches.
- Commits: no Co-Authored-By lines.

- `okay::host` (src/host.rs) — a typed program HANDED OUT: `start(|host| …)`, `host.outside::<E>()` (E a `wire`
  effect) a handler whose operations stop the program as `Asked` (`Slot`: the op out, its answer or failure);
  `Computation::{step, resume, resume_data, fail, cancel}`; `drive`, `with`, `only::<E>`, `Routes` (`on`, `handler`),
  `record`, `replay` (a recorded run answered again), `remote`. An interceptor passing an op to a handler that takes it (a host's)
  answers `Step::Later`; `Has<E, Through<Ix>>::resume` gives the handler's answer back as it is. `OUT` (thread-local)
  says an op is out with a host: `join`/`both`/`race` (typed.rs, `poll_branches`) lock to the branch that asked.
  `stream` (typed.rs): the body by value, `Emitter` an Rc slot. `Remote::out` (each op's answer as data) — the
  `effect!` macro writes it. Tests: tests/host.rs, branches.rs, typed_stream.rs, capabilities.rs; examples/shop.
- `Policies<E>` (typed.rs): an `Interceptor` of rules set at run time (`Policy<E>`: `for<'h> Fn(E, &dyn Fn(E) -> Step<'h>)`),
  the first outermost; `impl Watch for Vec<Box<dyn Watch>>`. tests/policies.rs; lanes `T:dyn0/1/4`.
- Guards on typed (typed.rs): `ensure` (a drop guard runs the finalizer with `now` when the body is dropped),
  `recover` (not `host::CancelFired`), `uncancelable` (the computation's `host::Mask`, set thread-locally while it is
  stepped; `Computation::cancel` inside gives the step back). tests/typed_guard.rs. Typestate by ownership:
  tests/typestate.rs, compile_fail in src/guarantees.rs.
- `okay::services` (Clock virtual/system + `timer`, Random/SplitMix, Log, Env, Secrets — SENSITIVE, Secret prints
  `***` —, Fresh, Store/InMemory) and `okay::control` (Abort/optional/exit, retry, backoff, timeout — a timer race on
  the system's clock, deadlines compared on a virtual one —, scope: spawn/join/cancel, acquire/defer released LIFO in
  Scope's Drop). Streams: `emit_all`, `merge` (round robin). tests/services.rs, tests/control.rs.
- crates.io: `cargo package -p okay-rust` verifies; okay-machine packages only once okay-rust is published (its
  dependency carries `version`). Publishing is the user's call (no cargo credentials here).
- THE ONE API (`specs/one-api.md`): new work goes to `okay::typed`/`okay::host`; the machine is the specification.
