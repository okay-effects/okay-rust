# okay-machine

The SPECIFICATION of okay's one API (the `okay` crate, [../README.md](../README.md)): okay-core's kernel (`okay.one`) in
Rust: a program of effects is DATA (`Eff<R, A>`, a tree of nodes), its effects a ROW of types, its handlers VALUES of
three forms not tied to it, the rest `k` data too — resumed any number of times, each time as it was taken — and
INTERCEPTORS for control and audit: a sensitive effect handled only under its audit, the types refusing anything else;
an observer of every operation of every effect.

```rust
use okay_machine::*;
use okay_machine::effects::*;

type R = row![Reader<i64>, State<i64>];
let p: Eff<R, i64> = eff! { data;
    n <- ask::<i64, _, _>();
    _ <- modify::<i64, _, _>(move |s| s + n);
    s <- get::<i64, _, _>();
    yield s * 2
};
assert_eq!(p.handle(reader(5)).handle(state(1)).run(), (6, 12));
```

The specification is okay-core's kernel and its suites: each test here is one of okay-core's, named after it
(`tests/std.rs` = TestStd, `tests/acceptance.rs` = TestAcceptance, `tests/intercept.rs` = TestIntercept).

## Contents

1. [Rows](#rows)
2. [Effects and operations](#effects-and-operations)
3. [Programs and `eff!`](#programs-and-eff)
4. [Recursion in constant stack](#recursion-in-constant-stack)
5. [Handlers](#handlers)
6. [The machine](#the-machine)
7. [Control and audit: interceptors](#control-and-audit-interceptors)
8. [A program stepped by a host](#a-program-stepped-by-a-host)
9. [Over a wire, with okay](#over-a-wire-with-okay)
10. [Guards](#guards)
11. [Delimited control: answer-type modification](#delimited-control-answer-type-modification)
12. [Generators, control as an effect](#generators-control-as-an-effect)
13. [Checked against](#checked-against)
14. [What the types refuse](#what-the-types-refuse)
15. [What Rust costs, against Scala](#what-rust-costs-against-scala)
16. [Next](#next)

## Rows

A row is a list of types: `Cons<E, R>` ending in `Nil`, written `row![A, B, C]`. An effect's place in a row is found
by the compiler — `Member<E, Ix>`, its index `Here` or `There<I>` never written — and `Member::Rest` is the row
without it: what a handler of `E` leaves. An effect is once in a row (a second would make the index ambiguous).

## Effects and operations

An effect is an enum of its operations; each variant carries its `Reply<T>`, `T` the operation's value:

```rust
#[derive(Clone, Debug)]
pub enum State<S> {
    Get(Reply<S>),
    Put(S, Reply<()>),
}
impl<S: Clone + Debug + 'static> Effect for State<S> {
    type Kind = Plain;
}

pub fn get<S: Clone + Debug + 'static, R: Member<State<S>, Ix>, Ix>() -> Eff<R, S> { perform(State::Get) }
pub fn put<S: Clone + Debug + 'static, R: Member<State<S>, Ix>, Ix>(s: S) -> Eff<R, ()> {
    perform(|r| State::Put(s, r))
}
```

`perform(make)` gives `make` the reply and makes the operation; the reply is the ONLY way to its value — `r.give(v)`
for a handler answering in place, `r.resume(&k, v)` for one taking the rest — so the value is the operation's type.
An effect is `Clone + Debug` and an operation's value too: what crosses the boundary of effects can be told and kept
(the observer needs both). `Kind` is `Plain` or `Sensitive` (see the interceptors).

The standard effects (`okay_machine::effects`): `Reader<V>` (`ask`, `reader`), `State<S>` (`get`, `put`, `modify`, `state`),
`Writer<W>` (`tell`, `collect`), `Choose` (`among`, `flip`, `fail`, `all` — every path, multi-shot), `Throws<E>`
(`raise`, `either`); `okay_machine::audit::Audit` (`record`, `trail`).

## Programs and `eff!`

`Eff<R, A>`: a program of value `A` over the row `R`. `pure(a)`, `perform(…)`, `p.flat_map(f)`, `p.map(f)`,
`p.handle(h)`, `p.run()` (only for `Eff<Nil, A>`: every effect handled).

`eff! { data; … }` is a for over programs as data (`eff! { … }` without `data;` is the `async` form, see
[One API](#one-api-compiled-by-the-handlers-type-okaytyped)): `x <- p;` binds, `_ <- p;` or `p;` drops, `let x = v;` names, the last line a program
or `yield v`. The rests are `Fn` closures — a handler may resume them many times — so each takes copies (`clone`) of
the names bound before it: a name bound is `Clone`. `flat_map` is typed by a trait, `Bind<N>`: the answer met chosen
by the kind of the rest, so that answer-type modification (`Cont`, stage 2) binds with no specialization.

## Recursion in constant stack

Rust has no guaranteed tail calls: functions calling each other a million times overflow the thread's stack. A
program's recursion runs on the machine's own stack, on the heap, instead — two ways to write it:

- `call!(f(args))` — a TAIL CALL: `f(args)` made when the machine gets to it (`defer`), so `is_even` → `is_odd` → …
  is a loop of the machine, however deep. The arguments are evaluated at the call and cloned (a rest resumed twice
  calls again).
- `tail_rec(s0, step)` — a LOOP (cats' `tailRecM`): `step` from a state to a program of `Loop::Next(s)` or
  `Loop::Done(a)`. Mutual recursion is a loop whose state names the function and its arguments.

```rust
fn is_even(i: i64) -> Eff<Nil, bool> { if i == 0 { pure(true) } else { call!(is_odd(i - 1)) } }
fn is_odd(i: i64) -> Eff<Nil, bool> { if i == 0 { pure(false) } else { call!(is_even(i - 1)) } }
assert!(is_even(1_000_000).run());

// an effect at each step: a million operations of a state
fn even_counted(i: i64) -> Eff<row![State<i64>], bool> {
    if i == 0 { pure(true) } else { eff! { data; modify::<i64, _, _>(|s| s + 1); call!(odd_counted(i - 1)) } }
}

enum Call { Even(i64), Odd(i64) }
let p = tail_rec(Call::Even(1_000_000), |c| ask::<i64, R, _>().map(move |d| match c {
    Call::Even(0) => Loop::Done(true),
    Call::Odd(0) => Loop::Done(false),
    Call::Even(i) => Loop::Next(Call::Odd(i - d)),
    Call::Odd(i) => Loop::Next(Call::Even(i - d)),
}));
```

The one rule: the recursive call inside a closure — `call!`, `defer`, a rest of `flat_map` or `eff!` — never before
it (`is_odd(i - 1).flat_map(…)` builds the whole tree first, by Rust's own recursion). Checked on a million steps,
with and without an effect at each, and a loop under a choice, each path its own (`tests/recursion.rs`).

## Handlers

Three forms, each over the row `T` outside it, of the body's `A` to its own `B`:

- `Handler::answering(value, ret)` — each operation its value at once, in place; nothing taken;
- `Handler::stateful(s0, step, ret)` — a state `S`, each operation its value and the state moved, in place. Taken
  into a rest, the state as it was then: each resumption its own (a choice's paths, each its state);
- `Handler::deep(ret, apply)` — each operation its clause, given the rest `k: K<T, B>`, data: resumed none, once or
  many times, kept, by the operation's `Reply`;
- `Handler::aborting(ret, abort)` — the rest dropped, no `k` made (an error, an early answer);
- `Handler::translating(apply)` — each operation a program over the row outside, `r.give(x)` at its end; a failure
  in it raised where the operation was, so the body's own `recover` sees it.

`p.handle(h)` takes a PLAIN effect off wherever it is in the row.

## The machine

A loop over the nodes and a stack of levels, each a handler's (or the top) with the frames waiting inside it. An
operation goes to the nearest level of its effect (by `TypeId`; the row says there is one): answered in place, the
state moved in place, or — a handler taking the rest — the levels from it up taken off as `k` and its clause run
outside them. A `k` resumed: its levels pushed back, copies, each state as it was (a state is written in place when
no `k` holds it, copied when one does). No recursion: binds nested to any depth (100 000 in the tests) on a heap stack.

The machine is ERASED: values are `Box<dyn Any>`, each taken back at the type it was made at — the typed API allows
no other. Rust has no GADTs, so okay-core's loop typed by the answers is not portable; the casts are three, inside
(`take`, `take_op`, `state`), and their panics unreachable from a typed program.

## Control and audit: interceptors

THE POINT OF IT ALL (okay-core's `docs/intercept.md` is the full account; the same programs there):

- an effect is `Plain` or `Sensitive` (`Effect::Kind`). A sensitive one is handled only by `handle_audited`, which
  needs `Audited<S>` in the row — put there only by `audit` or `observe`, in the row outside them, where no program
  can perform `S` (`Audited<S>` is not an effect);
- `p.intercept(h)`: a plain effect handled again for a part of a program — its clauses outside it, their own
  operations of the effect reaching the handler further out;
- `p.audit::<S, _, _>(i)`: an `Interceptor` of a sensitive effect, its clauses given `Pass<S>`, the only way to
  perform `S` past the audit — each operation told, passed on as it is or changed, refused; its answer changed;
- `p.observe(o)`: an `Observer` of every operation of every effect on its way out — `AnyOp`: `effect()` (the type's
  name), `describe()` (its `Debug`), `proceed(&k)`, `proceed_then(&k, after)` (its answer told each time the rest
  gets it) — or refused; outside, every sensitive effect of the row audited (`AuditAll`);
- the trail is an effect, `audit::Audit` (`record`, handled by `trail()`), outside the interceptors.

```rust
// every operation and its answer told: a reader, a state, a sensitive payment
fn telling<T: Member<Audit, I> + 'static, I: 'static, A: 'static>() -> Observer<T, A, A> {
    Observer::new(pure, |op, k| {
        let line = format!("{} {}", op.effect(), op.describe());
        record::<T, I>(line).flat_map(move |_| op.clone().proceed_then(&k, |a| record::<T, I>(format!("  → {a}"))))
    })
}
p.observe(telling()).handle(reader(5)).handle(state(1)).handle_audited::<Pay, _, _>(bank(100)).handle(trail()).run()
// trail: ["Reader<i64> Ask(_)", "  → 5", "State<i64> Get(_)", "  → 1", "State<i64> Put(6, _)", "  → ()", …]
```

## A program stepped by a host

okay-kernel's protocol (okay-core's `Step`): `p.step()` runs a program until its value, a failure, or its first
operation of the row OUTSIDE it — one no handler of its own takes — and gives it back with the rest:

```rust
type R = row![Reader<i64>];
let Step::Yield(y) = ask::<i64, R, _>().map(|n| n * 2).step() else { unreachable!() };
let Some(Reader::Ask(r)) = y.op::<Reader<i64>>() else { unreachable!() };
assert!(matches!(y.resume(r.give(1)), Step::Done(2)));
assert!(matches!(y.resume(r.give(7)), Step::Done(14)));   // the same rest again: each resumption its own
```

`Step::{Done, Yield, Failed, Cancelled}`; a `Yield` answered by the operation's reply (`resume`), failed (`fail`:
raised where the operation was, in the rest's levels — a `recover` there given it), or cancelled (`cancel`: the
finalizers run; under `uncancelable` the same step given back, the cancel fired when the mask is left). User code's
panics are `Failed` at the run (`tests/step.rs`, `tests/raise.rs`, `tests/head.rs`).

## Over a wire, with okay

`okay_machine::wire`: an operation as a name and a record, its answer a plain value (`Data`). An effect crosses when it is
`Remote` — `effect! { pub enum Prices: Plain, wire { … } }` writes it (each operation by its function's name, its
arguments a record, lowerCamel: a Scala case class's shape); the standard effects are. `wire::program(p.step(),
&remotes)` is a program on the wire: `Wire::Perform { name, args, k }`, `k` called as many times as the other side
resumes it; `wire::serve` a Rust handler answering the wire. Against okay's JVM host, through okay's Rust worker
(`examples/okayhost/TestOkayRustWire.scala`): okay's `Choice` resumes the Rust rest for every branch of a Rust
program — `[11, 21, 12, 22]` — the kernel's effect answered by Scala callbacks under okay's `Reader`, and a kernel
handler answering okay (`tests/wire.rs`).

## Guards

A guard is a level of no effect (okay-core's `Guard`): `p.ensure(fin)` runs `fin` when `p` is left — with its value,
a failure, an abort past it, a cancel; under a `k` resumed twice, twice; `p.recover(f)` gives `f` a failure raised in
`p` (a panic, a host's `fail` — not an effect's `raise`, which is its handler's); `p.uncancelable()`;
`p.guard(Guard::new().finalizer(…).passing(…))`, its passing told of each operation going out and sending one in its
place. In the machine a panic is a `Raise` where it happened: the loop runs under `catch_unwind`, its stack is on the
heap and untouched, so the machine unwinds the levels itself, their guards asked (`tests/guard.rs`).

## Delimited control: answer-type modification

`okay_machine::cont` (okay-core's stage 2; Materzok–Biernacki's shift0 and `$`, Asai–Kameyama's typing): a program of
control `Cont<R, I, O, A>` under a `$` over `T` — its row `Delim<T>` (`Control<T>` first) — `I` the answer its rest
gives, `O` the answer it makes. `shift0` takes the rest up to the nearest `$` (`Nearest`, never past another), the
`$` with it, as `k`; `dollar(body, ret)` and `reset` close it. A program of effects meets one of control at any
answers (`Bind` for the four pairs); under a `$` only handlers leaving the rest (`Passing`: answering, stateful,
translating). printf, the answer `String` turned `i64 -> String`:

```rust
use okay_machine::cont::shift0;
type S = Rc<dyn Fn(i64) -> String>;
let body: Cont<Reset, Eff<Nil, String>, Eff<Nil, S>, i64> =
    shift0::<Reset, Nil, _, i64, Eff<Nil, String>, Eff<Nil, S>>(|k| pure(Rc::new(move |x| k(x).run()) as S));
let p: Eff<Nil, S> = dollar(body.map(|n| n.to_string()), |s| pure(format!("Hello {s}")));
assert_eq!(p.run()(42), "Hello 42");
```

`PState` (`pstate`, `state`): a state whose TYPE the steps move, carried by the answer type — a protocol (closed,
open, logged in: a query before the login does not compile, a session left open does not compile), a pipeline whose
rows' schema is the state's type (`tests/control.rs`, `tests/pstate.rs`, `tests/nodes.rs`, `src/cont.rs`'s refusals).

## Generators, control as an effect

`Emit` with `generate` (a body as a lazy generator, its `k` kept as the next step, pulled again and again) and
`collect_emitted`; `Shifts<S, T>`: `shift0`, `shift` (Danvy–Filinski's: its body under the reset) and `reset`, control
as an effect of the row, its answer fixed (`tests/data.rs`, `tests/stream.rs`, `tests/shift.rs`).

## Checked against

- **okay-core's suites**, each test one of theirs, named after it: TestStd, TestAcceptance, TestIntercept,
  TestRecursion, TestStep, TestRaise, TestGuard, TestGuardStep, TestLaws, TestAgain, TestCont, TestControl, TestShift,
  TestShiftThin, TestPState, TestAtmUses, TestNodes, TestData, TestStream, TestHead, TestStatefulRun, TestFused's
  behaviour, TestGuarantees (as `compile_fail` doc tests: `src/guarantees.rs`, `src/cont.rs`).
- **A reference** (`src/reference.rs`, okay-core's TestReference): each node by its rule, nothing clever — normal forms
  in continuation-passing style, a shift0 not yet at its `$` a form of its own. Random programs with handlers of every
  kind in them and around them, stepped ones, and programs of control (shift0 calling `k` none, once, twice, `$`s a
  row deeper, effects out past the `$`s, handlers under them): the machine as the meaning on thousands; their answers
  pinned by a digest. (Rows are types here: a random program's handlers are installed by `intercept`, the row kept.)
- **okay-kernel's TCK digest** (`tests/tck.rs`): its thousand random programs — logs, states, failures, choices nested
  in any order, each effect with two identities — give `-8623788747993822161`, the digest of okay-kernel's reference
  machine, the legacy kernel and okay-java.

## What the types refuse

Checked by `compile_fail` doc tests (`src/lib.rs`, `src/audit.rs`), each failing for its own reason:

| refused | the compiler's message |
|---|---|
| `run` with an effect not handled | no method `run` for `Eff<Cons<…>, _>` |
| an operation of an effect not in the row | the effect `State<i64>` is not in the row … |
| a sensitive effect by `handle`, or `intercept` | a sensitive effect is handled only under its audit — audit it first, then `handle_audited` |
| `handle_audited` with no audit before it | `Member<Audited<Pay>, _>` not satisfied |
| a sensitive operation outside its audit | the effect `Pay` is not in the row … a sensitive effect outside its audit is `Audited<Pay>` |

## What Rust costs, against Scala

- **The erased machine** (no GADTs): typed outside, three casts inside.
- **`Fn` rests**: `k` is multi-shot, so frames are `Fn`, and names bound in `eff!` are `Clone`; a program value is
  single-use (build it inside the closure that needs it).
- **Turbofish**: `ask::<i64, _, _>()` — the effect's type named, the row and the index inferred.
- **Known gap**: a `Reply` kept from one operation and given to another panics (a value taken at another type);
  closing it wants the reply branded by a lifetime.


## Speed

okay-core's kernel lanes (`KernelBenchmark`), the same programs and n, ported to `machine/benches/kernel.rs`; one machine (an
Apple Mac), JMH's AverageTime for Scala, the same warm-up and rounds for Rust (`cargo bench -p okay-machine --bench kernel`, run under
JMH's lock). µs per run of the program:

| lane | Scala (okay-core, JVM 26) | Rust, first port | Rust, now | now / Scala |
|---|---|---|---|---|
| `pure` — isEven/isOdd, 1000 binds | 3.08 | 20.34 | 8.50 | 2.8× |
| `answered` — an operation answered in place, each step | 5.58 | 37.21 | 10.81 | 1.9× |
| `forwarded` / `forwarded16` — past 1 / 16 handlers | 5.53 / 5.83 | 37.71 / 47.31 | 11.04 / 12.43 | 2.0× / 2.1× |
| `general` — a `deep` handler, `k` each step | 9.44 | 85.57 | 36.87 | 3.9× |
| `forwardedK` / `forwardedK16` | 20.28 / 139.80 | 88.10 / 190.64 | 38.73 / 78.91 | 1.9× / **0.56×** |
| `reads` / `writer` | 3.15 / 9.45 | 29.16 / 30.61 | 10.14 / 11.66 | 3.2× / 1.2× |
| `stateCell` / `state` / `stateDeep` | 9.34 / 12.47 / 12.02 | 71.66 / 80.03 / 74.24 | 35.48 / 42.63 / 42.92 | 3.4–3.8× |
| `leftMaps` / `rightMaps` | 4.92 / 7.75 | 29.57 / 35.91 | 20.12 / 26.41 | 4.1× / 3.4× |
| `throwing` — an aborting handler in the program, each raise | 20.23 | 141.50 | 88.39 | 4.4× |
| `choose` — every value of a choice among 1000 | 13.24 | 121.61 | 101.52 | 7.7× |

Rust's columns from interleaved A/B runs of the commits (`/tmp/okz/tools/rs-ab.sh`, two rounds, each run under JMH's
lock), 2026-10-14. What made it so,
each step measured, the ones that lost reverted:

1. **Small values in place**: a value up to 16 bytes with nothing to drop (`i64`, `bool`, `()`, an operation of
   such) kept in the node — no `Box<dyn Any>`; its `TypeId` checked as `downcast` does.
2. **Fused nodes**: `perform(op).flat_map(f)` one node (`Inject`) — answered in place, the value straight into `f`,
   no frame pushed; `pure(a).flat_map(f)` one node (`Then`).
3. **An operation's node small**: its effect's `TypeId`, its value, and one reference to a table made at compile time
   (its name, describe, copy) — not six fields: a smaller node moved at each step.
4. **Frames in place**: a bind's closure up to 24 bytes kept in the frame — no allocation; taken into a rest `k`,
   moved behind an `Rc` there, once — so a closure is never cloned, and need not be `Clone`.
5. **One-shot `k` moved back**: a rest resumed by its last holder (a handler resuming once — most) puts its levels
   back as they are, nothing copied; copies, frames shared, only when it is resumed again.
6. **`Handler::aborting`**: a handler dropping the rest (okay-core's `Aborting`; `either()` is one) truncates the
   levels, no `k` made.
7. **A pool of level vectors**: the vector of a rest resumed once goes back to a pool for the next rest taken —
   `choose` −11%, `general` −12%, `forwardedK` −14% (`answered`, `forwarded`, `writer` +7–8%: the loop's code laid out
   otherwise by LLVM; the better of three layouts tried).

Tried and not kept: the operation's two paths (in place, taking the rest) as one function, or the taking one out of
the loop — the captures faster still, the in-place lanes 10–17% slower; the frames of a bind as a list in the node (every lane but `leftMaps` 30–55% slower: a bigger node
moved each step); frames cloned with `k` (a choice among 1000 cloned its `Vec` at each resumption, 2× slower).

**Guards, control, a program stepped (2026-10-17).** What they cost the machine's lanes, found by bisecting the
commits and A/B under JMH's lock: a new variant of `Kind` with a payload of its own — even one boxed value — made the
loop 10–20% slower, and `catch_unwind`'s closure taking the stack's address another 15–20%. So a rare node (a raise,
a cancel, a host's fail, control's shift0) is a `Handle` of `Level::Rare`, `Kind` as it was; and the loop is two: a
fast one on its own locals while no guard has been entered in the thread, handing the stack to the guarded one (under
`catch_unwind`) when one is. Against the commit before the guards, µs per run:

| lane | before guards | now | |
|---|---|---|---|
| `pure` / `answered` / `forwarded` | 8.78 / 10.26 / 10.53 | 8.82 / 10.03 / 10.08 | +0.5% / −2.2% / −4.3% |
| `writer` / `throwing` / `throwingK` | 11.27 / 96.37 / 129.36 | 10.74 / 94.62 / 129.39 | −4.7% / −1.8% / 0% |
| `reads` / `state` / `stateDeep` / `choose` | 10.23 / 44.04 / 44.29 / 99.05 | 10.87 / 47.42 / 47.41 / 107.56 | +6–9% |
| `general` / `forwardedK` / `forwardedK16` / `forwarded16` | 37.89 / 39.97 / 81.89 / 12.72 | 43.60 / 45.17 / 89.51 / 14.21 | +9–15% |
| `stateCell` / `leftMaps` / `rightMaps` | 36.22 / 20.75 / 27.32 | 40.88 / 24.04 / 30.17 | +10–16% |

The lanes left slower run code the same as before (the loop's arms the commit before's): what is left is the loop's
layout, as in step 7 above. The typed form (`T:`) unchanged.

**The allocator (2026-10-23).** `--features mimalloc` makes mimalloc the global allocator of the binary okay is
linked into (optional; okay has no dependency without it). Allocations of one run (`--features count-allocs`):
`pure`/`answered`/`reads` 2–4, `general`/`forwardedK`/`state*` ~1 per operation, `rightMaps` 2, `choose` 6,
`throwing` 8, `throwingK` 10; typed `T:` none but `T:throwing`'s message (1) and `T:writer`'s vector. Alternating A/B,
three rounds under JMH's lock, medians (`machine/benches/history.tsv`), µs per run:

| lane | system | mimalloc | |
|---|---|---|---|
| `pure` / `answered` / `forwarded` / `reads` | 8.68 / 9.93 / 10.28 / 10.80 | 8.69 / 9.95 / 10.13 / 11.54 | noise |
| `state` / `stateCell` / `writer` | 47.71 / 40.41 / 10.73 | 46.95 / 39.47 / 10.36 | −2–3% |
| `general` / `forwardedK` / `forwardedK16` | 43.55 / 46.96 / 88.51 | 40.96 / 43.35 / 80.40 | −6–9% |
| `leftMaps` / `rightMaps` | 25.11 / 29.79 | 22.51 / 26.55 | −10–11% |
| `throwing` / `throwingK` / `choose` | 110.17 / 135.02 / 111.22 | 86.77 / 103.17 / 89.54 | −20–24% |
| `T:throwing` / `T:writer` | 7.94 / 1.14 | 5.94 / 0.94 | −18–25% |

So the allocator pays where there are several allocations an operation; a state's one does not — its time, and that
of the lanes that do not allocate, is the loop's own. What is left: the levels and frames of a rest copied when it is
resumed many times (`choose`); a nested `flat_map` still a boxed node (`leftMaps`, `state`).

**The node smaller (2026-10-23).** An operation's effect `TypeId` moved from the node into its table (`OpInfo`, made
at compile time): `Op` 64 → 48 bytes, `Kind` 96 → 80 — the node moved at each step. Alternating A/B, three rounds
under JMH's lock, µs per run: `state` 47.7 → 42.2, `stateCell` 41.3 → 35.3, `stateDeep` 47.5 → 42.3, `general` 43.4
→ 39.8, `forwardedK` 45.6 → 41.4, `choose` 114.9 → 102.5, `throwing` 104.7 → 96.1, `writer` 11.1 → 10.3; `pure` (no
operation in it) 8.7 → 9.6, the loop's layout. `eff.rs`'s `a_node_is_small` keeps it so. Tried and not kept: a `Val`
with no tag (32 bytes, a box marked by an id) — `answered`/`forwarded`/`writer` 10–12% slower, with the loops aligned
too, so not layout; the frames' vectors of a rest's copies pooled — `choose` unchanged, `answered`/`forwarded`/`writer`
+15% (layout). Profiles (`sample`): `choose` is malloc and free (42%) and the copies of its rest (11%); `state` is the
loop itself (56%) — what is left there is the loop's dispatch, not memory.

## Next

- A `Linear` handler (okay-core's: `k` once, at any answers, under a `$`): its clause generic over the answers needs
  a rank-2 type — a trait object of a generic method; not yet.
- Operations BY KEY (okay-core's `In.At`, okay's root): an effect's place here is its type in the row.
- The reference's random programs a row deeper at each step (okay-core's `reset` inside a body over `Delim[R]`): a
  recursion over types, one per step — written to a fixed depth if wanted.
