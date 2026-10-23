# okay-rust

[okay](https://github.com/okay-effects/okay)'s algebraic effects in Rust, no dependencies, two crates:

| crate | what |
|---|---|
| **`okay`** (the root, `src/`) | okay-core's kernel (`okay.one`) ported: a program of effects as DATA, its effects a ROW of types, handlers as values, the rest `k` resumed any number of times, INTERCEPTORS for control and audit; a program STEPPED by a host (okay-kernel's protocol) and over okay's WIRE — its `k` resumed by okay's Choice across processes; GUARDS (`ensure`, `recover`, `uncancelable`); delimited CONTROL with answer-type modification (`Cont`, `$`, `shift0`, `PState`); one API compiled by the handlers' types (`okay::typed`, async); direct style passing effects down (`okay::direct`). Checked by okay-core's own suites, a reference interpreter on random programs, okay-kernel's TCK digest; measured against Scala. |
| **`okay-effects`** (`effects/`) | SUPERSEDED by `okay::typed` + `okay::host` (2026-10-23; kept for existing users, not developed): effects in plain `async` Rust with capabilities, the kernel's protocol, middleware, every path, branches, streams, the wire — each now on the one API (below: [Handed out to a host, branches, streams](#handed-out-to-a-host-branches-streams)). Guide: [docs/guide.md](docs/guide.md). |

`examples/shop`: the shop of okay's examples on the one API (`okay::typed`, `okay::host`) — one `needs!` checkout over
its effects; a `Db` effect under the inventory, payments (sensitive) over HTTP under an interceptor, a limit ending
the checkout, production and test assemblies, the same checkout handed out to a host: a manager, middleware, a
recording and its replay, every path, the wire. `examples/okayhost/TestOkayRustWire.scala`: the same shop under Scala handlers in okay, and a Rust handler
answering okay, run against okay's JVM host.

```
cargo test --workspace
```

This repository was okay-rs (github.com/okay-effects/okay-rs, its history kept here) and okay-effects, together.

---

# The kernel: `okay`

okay-core's kernel (`okay.one`) in Rust: a program of effects is DATA (`Eff<R, A>`, a tree of nodes), its effects a
ROW of types, its handlers VALUES of three forms not tied to it, the rest `k` data too — resumed any number of times,
each time as it was taken — and INTERCEPTORS for control and audit: a sensitive effect handled only under its audit,
the types refusing anything else; an observer of every operation of every effect.

```rust
use okay::*;
use okay::effects::*;

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

The standard effects (`okay::effects`): `Reader<V>` (`ask`, `reader`), `State<S>` (`get`, `put`, `modify`, `state`),
`Writer<W>` (`tell`, `collect`), `Choose` (`among`, `flip`, `fail`, `all` — every path, multi-shot), `Throws<E>`
(`raise`, `either`); `okay::audit::Audit` (`record`, `trail`).

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

`okay::wire`: an operation as a name and a record, its answer a plain value (`Data`). An effect crosses when it is
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

`okay::cont` (okay-core's stage 2; Materzok–Biernacki's shift0 and `$`, Asai–Kameyama's typing): a program of
control `Cont<R, I, O, A>` under a `$` over `T` — its row `Delim<T>` (`Control<T>` first) — `I` the answer its rest
gives, `O` the answer it makes. `shift0` takes the rest up to the nearest `$` (`Nearest`, never past another), the
`$` with it, as `k`; `dollar(body, ret)` and `reset` close it. A program of effects meets one of control at any
answers (`Bind` for the four pairs); under a `$` only handlers leaving the rest (`Passing`: answering, stateful,
translating). printf, the answer `String` turned `i64 -> String`:

```rust
use okay::cont::shift0;
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

## One API, compiled by the handler's type (`okay::typed`)

`eff! { x <- op(cx); …; yield v }` — the default form of `eff!` — is an `async` block: a state machine the
compiler builds, generic over its context `cx` (the handlers in force, `Cx(h, rest)`, each effect found by its type as
in a row). Monomorphized for the handlers it runs under, an operation is compiled by what its handler is:

- answering in place (a reader, a state, a writer, an audit): `Ready` at once — inlined, a call of the handler;
- dropping the rest (`either`): the body's future dropped where it raised;
- taking the rest once (`generate`): the block suspended, resumed once — the state machine itself the `k`, nothing on
  the heap;
- taking it many times (`choose`): by replay — the body run again for each path, its past told it again (below).

No turbofish: an operation's type is inferred from the context's bounds (`Has<Reader<i64>, _>`) or its argument.
No index parameters either: `needs!` writes them — `cx: [Reader<i64>, State<i64>]` lists the effects, as `row![…]`
does for a program as data; the function's own type parameters beside them (`async fn f<V: Clone>(cx: [Reader<V>])`).

```rust
okay::needs! {
    async fn counted(cx: [Reader<i64>, State<i64>, Writer<String>], n: i64) -> i64 {
        eff! {
            d <- ask(cx);
            _ <- modify(cx, |s| s + d * n);
            s <- get(cx);
            _ <- tell(cx, format!("s {s}"));
            yield s
        }.await
    }
}
let cx = Cx(AskWith(2i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
assert_eq!(run(counted(&cx, 10)), Ok(21));
```

The effects are the machine's own enums (`Reader`, `State`, `Writer`, `Throws`); a handler implements `Handles<E>`,
answering `Step::Now(reply.give(v))` or taking the rest `Step::Later`. A sensitive effect is in a context only
under an interceptor of it (below). `tests/typed.rs`: one program under two contexts, either, a raise in a loop each
under its own either, a generator with a state in it.

µs per 1000 operations, the same lanes:

| lane | Scala (okay-core) | the machine (`Eff`) | `eff! { … }` |
|---|---|---|---|
| `answered` | 5.58 | 10.3 | **0.25** |
| `reads` (the context opaque) | 3.15 | 9.8 | **0.27** |
| `state` (the context opaque; 2000 operations) | 12.47 | 42.0 | **1.77** |
| `writer` | 9.45 | 11.3 | **1.07** |
| `throwing` — each raise under its own either | 20.23 | 94.7 | **7.66** |
| a generator, 1000 values, the body suspended and resumed at each | 22.6 (okay-one's `generate`) | — | **2.14** |

| `choose` — every value of a choice among 1000 | 13.24 | 107.9 | **6.1** (replay) |

With the context visible, LLVM folds the loops of `reads` and `state` altogether (0.017).

### The rest taken many times: replay

`all(cx, body)` runs the body again for each path, and tells it its past again (Koppel, Scherer, Siek, "Capturing the
future by replaying the past", ICFP 2018): the choices made so far, and every answer a handler further out gave before
the last of them — told again, not asked again. So a state further out is threaded through the paths in order,
exactly as the machine's `all` under it; a state inside the body is each path's own; a writer further out hears what
was told before a choice once. `first(cx, body)` stops at the first path not failing. `flip`, `among`, `fail`.

```rust
let cx = Cx(StateCell::new(0i64), End);
let r = run(all(&cx, async |cx| eff! {
    s <- get(cx);              // read once: each path resumed with it, as the machine's k holds it
    b <- flip(cx);
    _ <- put(cx, s + 1);
    yield (b, s)
}.await));
assert_eq!(r, Ok(vec![(true, 0), (false, 0)]));
```

The cost: a path's prefix run again — the operations before its last choice answered from the log, not by their
handlers; n paths of depth d, n × d steps. The rule: the body decides by its effects only (an `AsyncFn`; a cell or the
clock read outside an effect breaks the replay, and it panics saying so). `tests/replay.rs`: the machine's suite's
choose programs, a read before the choice, a state in each path, the triples, eight queens (92), `all` inside `all`.

### Effects waiting on the world

A handler may answer when a future is done — a timer, the network, a channel: `Step::Wait(Box::pin(async move { …;
r.give(v) }))`. The operation polls it with the program's own `Context`, so the program waits as any future does,
woken by it, on any executor: `block_on` (one program, the thread parked until woken; no dependencies), or tokio's
`LocalSet`, or another. `both(a, b)` runs two at once, each going on while the other waits.

```rust
effect! {
    enum Net: Plain, waits {
        Fetch(key: String, ms: u64) -> String = fetch;
    }
}
impl Handles<Net> for Network {
    fn handle(&self, op: Net) -> Step<'_> {
        match op {
            Net::Fetch(key, ms, r) => Step::Wait(Box::pin(async move { sleep(ms).await; r.give(format!("<{key}>")) })),
        }
    }
}
let (a, b) = block_on(both(fetch(&cx, "a".into(), 150), fetch(&cx, "b".into(), 150)));   // 150 ms, not 300
```

The layers the same over a wait: the replay logs the answer when it comes (asked once, told again to every path), a
watch sees it then, an interceptor waits for its handler (`pass.wait(…).await` in its own `Step::Wait`), `either`
and `generate` go on around it. `run` polls once: a program waiting says `Stuck`.

An effect that may wait is DECLARED so (`waits`): its operations are `Waiting` futures, keeping the future they wait
on and dropping it if the program is dropped first. Every other operation is a `Perform`, keeping none — measured, a
future to drop in the operation costs 2–7× on `reads` and `state` even when it is never there (the async block
around it must be ready to drop it); so effects that never wait pay nothing. A handler answering later an effect not
declared to wait panics saying so. `tests/async.rs`.

### Recursion, every effect seen, an effect declared

- **`tail_rec(s0, async |s| … Loop::Next(s2) / Loop::Done(a))`**: a loop in constant stack — a recursion, a mutual
  one (its calls the states: `Call::Even(n)`, `Call::Odd(n)`), at any depth: a million calls, each a state's
  operation. An async recursion is Rust's own: each call a future boxed in its caller's, polled through them all.
- **Interceptors**: `Audited::new(handler, interceptor)` in the context — each operation of the effect goes through
  the `Interceptor<E, R, Ix>`, which passes it on (`pass.perform(…)`, as it is or changed), gives or changes its
  answer, or refuses it (an answer saying so, or an error raised further out); its own operations — a trail
  (`record`, `Trail`), a raise — go to the context after the handler. A sensitive effect is handled ONLY so: a bare
  bank, or a bank under something not an interceptor of `Pay`, is "no handler of `Pay` in the context"; the handler
  is a private field, not reached past its interceptor (the three `compile_fail` doc tests of `okay::typed`). An
  interceptor costs nothing: inlined, `answered` through one 0.26 µs per 1000 operations, as without.
  `tests/typed_intercept.rs` runs `tests/intercept.rs`'s programs, the same trails:

```rust
impl<R, I1, I2> Interceptor<Pay, R, (I1, I2)> for Guarded
where R: Has<Throws<String>, I1> + Has<Audit, I2> {
    fn intercept<H: Handles<Pay>>(&self, op: Pay, pass: Pass<'_, H>, cx: &R) -> Step<'_> {
        match op {
            Pay::Transfer(to, amount, r) => {
                let to = to.trim().to_uppercase();
                if amount > self.limit {
                    let why = format!("refused: {amount} to {} over the limit {}", masked(&to), self.limit);
                    now(record(cx, why.clone()));
                    let _: Option<()> = now(raise(cx, why));
                    return Step::Later;
                }
                let account = to.clone();
                let Some(receipt) = pass.perform(|r2| Pay::Transfer(account, amount, r2)) else { return Step::Later };
                now(record(cx, format!("transfer {amount} to {} → {receipt}", masked(&to))));
                Step::Now(r.give(receipt))
            }
            …
        }
    }
}
let cx = Cx(Audited::new(Bank(RefCell::new(1000)), Guarded { limit: 1000 }), Cx(Raised::new(), Cx(Trail::new(), End)));
let result = run(catch(&cx.1.0, payments(&cx)));
// "transfer 100 to ****1234 → ok:ACCT-1234:100", "balance → 900", "refused: 5000 to ****T-77 over the limit 1000"
```

- **`observe(cx, watch, body)`**: every operation of every effect on its way out seen by a `Watch` — its effect
  (`State<i64>`), the operation as written (`Put(6, _)`), its answer — let through or refused (the body dropped
  there: `Err(why)`); a layer with no handler of its own, each effect at the index it has further out. Outside `all`
  it sees an operation further out once; inside, each time the path runs. 11 ns an operation seen (`answered` under
  a watch counting them: 11.0 µs per 1000).
- **`effect!`**: the enum, its kind, each operation for both forms — `Pay::transfer(…)` an `Eff`, `transfer(cx, …)`
  a `Perform`:

```rust
okay::effect! {
    /// payments
    pub enum Pay: Sensitive {
        /// money sent to an account: a receipt, or why not
        Transfer(to: String, amount: i64) -> Result<String, String> = transfer;
        Balance() -> i64 = balance;
    }
}
```

### Handed out to a host, branches, streams

ONE API (`specs/one-api.md`): a program is written once, as `eff!`/`needs!` over a context; what answers its effects
is the context it is given. What okay-effects did with capabilities, the one API does with contexts:

- **A capability is a context**: the one a program passes picks the handler — two instances of one effect are two
  contexts (or one over the other, the index written: `ask_i::<_, Here>(&nested)`); a context cannot outlive its
  handlers (a borrow: `compile_fail`). `tests/capabilities.rs`.
- **Handed out (`okay::host`)**: `start(|host| async move { let cx = Cx(host.outside::<Inv>(), …); checkout(&cx, …).await })`
  — each operation of an effect left `outside` stops the program as `Asked` (typed: `get::<Inv>()`; data: `name`,
  `record`), answered by the host (`resume`, `resume_data`), failed (raised where performed), or the program
  cancelled (dropped). A sensitive effect is outside only under its interceptor, as any handler of it:
  `Audited::new(host.outside::<Pay>(), Limit(1000))` — a charge over the limit refused before the host sees it.
  Around the host: `drive`, middleware (`with`, `only::<E>`, `record`, `replay`), `Routes` (`on::<E>(|op| … r.give(v))`,
  or `handler::<E>(h)` — the same handler a context takes), `remote` (okay's wire), `every_path`. `tests/host.rs`.
- **Branches**: `join(branches)`, `both(a, b)`, `race(branches)` — polled on one thread, their waits overlapping,
  the handlers of their context shared; an operation out with a host locks the branches to the one that asked until
  it is answered; a branch's panic drops the others. `tests/branches.rs`.
- **Streams**: `stream(cx, async |cx| … emit(&cx, v).await …)` — an `Iterator`, the body run a step per value,
  dropped where it is when the iterator is. `tests/typed_stream.rs`.

- **Interceptors set at run time**: `Policies<E>` — an interceptor whose rules are closures added while programs run
  (from a configuration, per tenant): each given the operation and `next`, it passes it on (as it is or changed, the
  reply carried along), answers it, or refuses it. `Audited::new(handler, policies)`: a sensitive effect's audit
  too — the type still says "audited", the rules are data. And watches in a `Vec<Box<dyn Watch>>` under `observe`.
  µs per 1000 operations: a static interceptor 0.28; policies with no rule 1.09, one 2.64, four 5.76 (~1 ns a
  rule); `observe` 12.0. `tests/policies.rs`.

- **Guards**: `ensure(body, || fin)` — the finalizer after the value, after a panic (the panic goes on), and when
  the body is dropped where it is (an error raised past it, a race lost, a host's cancel: then answered in place);
  nested innermost first, a finalizer's panic replacing the one before, once a path under `all`. `recover(body, f)`
  — a panic, a host's `fail`; not a `raise`, not a cancel. `uncancelable(body)` — a host's cancel inside gives the
  same step back and fires when the region is left, over its value or its failure. `tests/typed_guard.rs`
  (tests/guard.rs's programs, as far as an async body has them).
- **A state whose type the steps move** (`PState`'s purpose) by OWNERSHIP: the state a token each step takes and gives
  (`Sess<Open>` → `Sess<Authed>`), the session owing `Sess<Closed>` back — a query before the login, a session left
  open, an old state used again do not compile (`src/guarantees.rs`); the operations still through the context, seen
  by a watch. `tests/typestate.rs`: tests/pstate.rs's protocol and pipeline. Answer-type modification itself (`Cont`,
  `$`, `shift0`) stays the machine's.

The shop (`examples/shop`) is the whole of it: one checkout, in process under production and test handlers, and
handed out.

## Level 1: effects passed down, direct style

> SUPERSEDED (2026-10-23) by `okay::typed`, the one API: the same evidence passing, and the rest taken too; as
> fast or faster. Kept, not developed.

The principled answer to the speed of level 2 (below): most effects — a reader, a state, a writer, a log, an audit —
answer and go on, they never need `k`; for them a program as data is pure cost. `okay::direct` (a prototype) passes
them DOWN as parameters, as Koka and Effekt do (evidence passing): an effect is a trait, its handler a value, a
program a generic function whose signature is its row, an operation a call of the handler's method — monomorphized,
inlined, no node, no allocation. The effects that take the rest (a choice resumed twice, a generator, async) stay on
the machine, level 2.

```rust
use okay::direct::*;

fn counting(st: &mut impl State<i64>, log: &mut impl Writer<String>, n: i64) -> i64 {
    for _ in 0..n { let x = st.get(); st.put(x + 1); }
    log.tell(format!("counted {n}"));
    st.get()
}
let (mut st, mut log) = (Cell(0), Log::new());
assert_eq!(counting(&mut st, &mut log, 1000), 1000);
```

- an interceptor is a wrapper of a handler (`InterceptWriter`);
- a SENSITIVE effect's trait asks `AuditedOnly` (`pub trait Pay: AuditedOnly`), sealed — implemented by okay's
  `Audited` and `Observed` alone: a bank's handler reaches a program only through `audit(&mut bank, policy)` or
  `observe(&mut bank, &observer)`; given bare, "`MyBank` is not under an audit";
- an observer (`Observed`, one `Trail` shared by many handlers) tells every operation and its answer;
- an error is `Result` and `?`: in direct style an abort costs nothing.

`tests/direct.rs`: tests/intercept.rs's programs — the payments audit (limit, normalized account, masked trail), a
plain intercept, the observer of a reader, a state and payments — the same trails.

The same lanes, µs per 1000 operations:

| lane | Scala (okay-core) | level 2 (the machine) | level 1, inlined | level 1, the handler opaque (`dyn`, a call each) |
|---|---|---|---|---|
| `answered` | 5.58 | 10.8 | 2.45 (a `black_box` each step) | — |
| `reads` | 3.15 | 10.2 | 0.017 (folded by LLVM) | 0.70 |
| `writer` | 9.45 | 11.5 | 0.85 | — |
| `state` / `stateDeep` | 12.47 / 12.02 | 42.9 / 43.1 | 0.017 (folded) | 1.21 (2000 calls) |

Inlined, LLVM sees the loop through the handler and folds it: the effect costs nothing. Behind `dyn`, each operation a
real call, still 10× faster than the JVM and 35× than the machine. Open: the bridge — a level-1 program calling an
effect of level 2 (it cannot capture `k` itself), and an `effect!` macro writing an effect's trait, its audited and
observed wrappers.

## Speed

okay-core's kernel lanes (`KernelBenchmark`), the same programs and n, ported to `benches/kernel.rs`; one machine (an
Apple Mac), JMH's AverageTime for Scala, the same warm-up and rounds for Rust (`cargo bench --bench kernel`, run under
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
layout, as in step 7 above. The typed form (`T:`) and level 1 (`D:`) unchanged.

**The allocator (2026-10-23).** `--features mimalloc` makes mimalloc the global allocator of the binary okay is
linked into (optional; okay has no dependency without it). Allocations of one run (`--features count-allocs`):
`pure`/`answered`/`reads` 2–4, `general`/`forwardedK`/`state*` ~1 per operation, `rightMaps` 2, `choose` 6,
`throwing` 8, `throwingK` 10; typed `T:` none but `T:throwing`'s message (1) and `T:writer`'s vector. Alternating A/B,
three rounds under JMH's lock, medians (`benches/history.tsv`), µs per run:

| lane | system | mimalloc | |
|---|---|---|---|
| `pure` / `answered` / `forwarded` / `reads` | 8.68 / 9.93 / 10.28 / 10.80 | 8.69 / 9.95 / 10.13 / 11.54 | noise |
| `state` / `stateCell` / `writer` | 47.71 / 40.41 / 10.73 | 46.95 / 39.47 / 10.36 | −2–3% |
| `general` / `forwardedK` / `forwardedK16` | 43.55 / 46.96 / 88.51 | 40.96 / 43.35 / 80.40 | −6–9% |
| `leftMaps` / `rightMaps` | 25.11 / 29.79 | 22.51 / 26.55 | −10–11% |
| `throwing` / `throwingK` / `choose` | 110.17 / 135.02 / 111.22 | 86.77 / 103.17 / 89.54 | −20–24% |
| `T:throwing` / `T:writer` / `D:writer` | 7.94 / 1.14 / 0.91 | 5.94 / 0.94 / 0.74 | −18–25% |

So the allocator pays where there are several allocations an operation; a state's one does not — its time, and that
of the lanes that do not allocate, is the loop's own. What is left: the levels and frames of a rest copied when it is
resumed many times (`choose`); a nested `flat_map` still a boxed node (`leftMaps`, `state`).

## Next

- A `Linear` handler (okay-core's: `k` once, at any answers, under a `$`): its clause generic over the answers needs
  a rank-2 type — a trait object of a generic method; not yet.
- Operations BY KEY (okay-core's `In.At`, okay's root): an effect's place here is its type in the row.
- The reference's random programs a row deeper at each step (okay-core's `reset` inside a body over `Delim[R]`): a
  recursion over types, one per step — written to a fixed depth if wanted.
