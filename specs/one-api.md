# One API: speed and control from the same program

Status: agreed 2026-10-23, in progress.

## The rule

A program is written ONCE, in `okay::typed`: `eff! { … }` / `needs!` — an `async` body over a context `cx`. What it
compiles to is chosen by the CONTEXT the caller gives, never by the program:

| context layer | what it gives | cost |
|---|---|---|
| static (`Cx(handler, rest)`, `Audited::new(h, i)`, `observe`) | handlers and interceptors known at compile time, monomorphized, inlined | `answered` 0.25 µs / 1000 ops, an interceptor free |
| `Policies<E>` under `Audited` | interceptors, audit policies installed at run time (config, a tenant) | ~1 ns a rule an operation (`T:dyn1` 2.64 µs / 1000) |
| `okay::host` (`Outside<E>`) | an operation no layer takes is ASKED of a host: okay-kernel's protocol (step, resume, fail, cancel), okay's wire | the suspended future is the one-shot `k` |

The guarantee holds in every one: a `Sensitive` effect enters a context only under its audit (`Allowed`, sealed) —
a `Dyn` layer too, since the context's TYPE stays static even when its interceptors are not.

## The machine (`Eff`)

Not a user API any more. It stays as:
- the SPECIFICATION — the reference interpreter, the TCK digest, okay-core's suites; typed programs are checked
  against the machine's answers (`tests/replay.rs` is the pattern);
- the backend of what an async body cannot be: answer-type modification (`Cont`, `$`, `shift0`, `PState`) — a
  `shift0` must hold the rest up to its `$` as a value, and an async body cannot be cut.

## A state whose type moves (`PState`'s purpose) in typed

Not by answer types: by ownership. The state is a token value; each operation takes `Sess<Open>` and gives
`Sess<Authed>`; the body must give back `Sess<Closed>`. A query before the login, a session left open, an old state
used again (moved) — each a compile error. The operations still go through `cx`: audit and observe see them.
Tests: `tests/pstate.rs`'s protocol and pipeline, with the compile_fail cases.

## Steps

0. Baseline: every lane, alternating A/B under JMH's lock, appended to `machine/benches/history.tsv`. DONE 2026-10-23.
1. Allocator: `--features mimalloc`; allocations per run per lane (`--features count-allocs`). DONE: −20–25% where
   several allocations an operation (throwing, choose, T:throwing), noise where none (README's Speed).
2. This spec. DONE.
3. `Dyn` layer — DONE 2026-10-23 as `Policies<E>` (an `Interceptor` of rules set at run time; a sensitive effect's
   audit too) and watches in a `Vec` under `observe`; lanes `T:dyn0/1/4`: 1.09 / 2.64 / 5.76 µs per 1000.
4. `Hosted`: the step protocol and the wire on typed — DONE as `okay::host` (2026-10-23). The TCK digest on typed —
   DONE 2026-10-23: `okay::dynamic` (a context built at run time: layers by `TypeId`, either, all/first by replay
   with a cloner carried by each operation; `Shared` for a handler every path sees), tests/tck_typed.rs = the
   machine's digest.
5. Guards on typed: `ensure`, `recover`, `uncancelable` — DONE 2026-10-23 (a drop guard; `catch_unwind` around a
   poll; a host's cancel waits on the computation's `Mask`). tests/typed_guard.rs.
6. Typestate on typed (above) — DONE 2026-10-23: tests/typestate.rs, compile_fail in src/guarantees.rs.
7. `okay::direct` folded into typed — DONE 2026-10-23: marked superseded; REMOVED 2026-10-27 with okay-effects (typed `answered` 0.28 vs 2.45; `D:stateDyn`
   1.21 vs `T:stateOpaque` 1.98 the one lane direct wins). okay-effects: DECIDED and DONE
   2026-10-23 — its capabilities, protocol, middleware, every path, branches (`join`/`both`/`race`), streams and
   wire on the one API (`okay::host`, `okay::typed`); `examples/shop` on it; the crate kept, marked superseded.
8. The machine's leftovers — DONE as far as measured 2026-10-23: the node smaller (an op's effect in its table,
   `Kind` 96 → 80: state lanes −11–15%, choose −11%), mimalloc (−20–25% where several allocations an operation).
   Refuted: a `Val` with no tag, the frames of a rest's copies pooled. Not tried, on purpose: fusing a left-nested
   `flat_map` over an operation into one frame — the frames composed call each other, so a million left-nested maps
   would overflow the stack the machine promises not to use. Left: `choose` is malloc/free (42%) and its rest's
   copies; `state` the loop's dispatch (56%) — the machine is the specification now, the one API is typed.

## 2026-10-27: simpler

The user's decisions: the one API is ONE-SHOT — replay removed (typed `all`/`first`/`among`/`flip`/`fail`, the host's
`every_path`); `okay::dynamic` removed (its reason was the TCK on typed, which needs a choice); the rest taken many
times is the machine's alone. The machine moves to its own crate (`okay-machine`): `okay` is the one API.

## 2026-10-28: the rest taken many times, compiled

The user's decision: no replay, ever; multi-shot is a rest that is a value. `okay::resumable` (`Rest`: `Clone`,
`resume(answer) -> Step`) with two ways to make one: `#[resumable(Ops)]` (crate `okay-resumable`, stable: the function
an enum of its stops, each resumption straight code) and a coroutine (`--features coroutines`, nightly,
`coroutine_clone`). The same handlers drive both. Measured (n = 1000, µs): choose 0.24, general 0.35, generate 0.29
(stable) against okay-goose's 1.23 / 1.23 / 1.21; the coroutines 2.75 / 6.33 / 6.25. Before each resumption was
straight code, the generated rest was a loop over states (`match` per hop): 1.25 / 4.89 / 3.68.

Later the same day: a rest is resumed IN PLACE — `Rest::resume(&mut self, answer) -> Step<Op, Out>` (was
`resume(self) -> Step<Op, Self, Out>`, the rest moved into each step and out). The coroutine paid for the move: its
body stores its state's byte, the caller then loads the whole coroutine to move it — a store-to-load forwarding
stall each step. Measured, three alternating rounds, medians (µs, choose / general / generate):

- coroutines: 2.76 / 6.55 / 6.45 → 1.57 / 0.46 / 0.29;
- `#[resumable]`: 0.24 / 0.36 / 0.30 → 1.62 / 0.59 / 0.30. The old choose was the compiler inlining a path that ends
  at once into the handler's loop (the resumption in the caller, `count_all(k.clone().resume(..))`); with the
  resumption in the handler (the in-place form's own shape) each path is a call, ~1.6 ns, for both ways. general is
  at the floor of its `black_box`es either way.

Refuted on the way: `#[inline(always)]` on `Co::resume` (6.10 → 6.13 general: the coroutine body stayed a call);
the coroutine boxed (`Co(Box<G>)`: general 2.28, generate 1.83, but choose 8.6 — a clone an allocation); the
choose handler resuming before its recursive call (2.46 for both: worse).
