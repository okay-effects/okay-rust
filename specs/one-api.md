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
| `okay::dynamic::Dyn` | handlers nested at run time, to any depth (scripts, the TCK) | 2.5 µs / 1000 a layer, ~2 ns each further |
| `Paths` (`all`, `first`) | the rest taken many times, by replay | a path's prefix answered again from the log |

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

0. Baseline: every lane, alternating A/B under JMH's lock, appended to `benches/history.tsv`. DONE 2026-10-23.
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
7. `okay::direct` folded into typed — DONE 2026-10-23: marked superseded (typed `answered` 0.28 vs 2.45; `D:stateDyn`
   1.21 vs `T:stateOpaque` 1.98 the one lane direct wins). okay-effects: DECIDED and DONE
   2026-10-23 — its capabilities, protocol, middleware, every path, branches (`join`/`both`/`race`), streams and
   wire on the one API (`okay::host`, `okay::typed`); `examples/shop` on it; the crate kept, marked superseded.
8. The machine's leftovers, only as far as it stays a working backend: `choose` copying, a nested `flat_map` boxed,
   the +9–16% since the guards.
