# okay-rust

[okay](https://github.com/okay-effects/okay)'s algebraic effects in Rust, no dependencies (the allocator optional:
`--features mimalloc`). Three crates:

| crate | what |
|---|---|
| **`okay`** (the root) | THE ONE API — `okay::typed`, `okay::host`: what a program is written in |
| **`okay-resumable`** (`resumable/`) | the rest taken MANY times, compiled: `#[resumable(Ops)]` makes a function an explicit rest — an enum, `Clone` — on stable Rust; with `--features coroutines` (nightly) the same from a coroutine. [Multi-shot](#the-rest-taken-many-times-okayresumable) |
| **`okay-machine`** (`machine/`) | its SPECIFICATION — okay-core's kernel, a program as data; the rest taken many times, answer-type modification; okay-core's suites, a reference interpreter, okay-kernel's TCK. [machine/README.md](machine/README.md) |

ONE API: a program is written once — an `async` function over a context (`needs!`, `eff!`); what answers its effects
is the CONTEXT it is given, never the program. Handlers are values; INTERCEPTORS give control and audit — a sensitive
effect reaches a program only under its audit, the types refusing anything else.

```rust
okay::effect! { pub enum Payments: Sensitive, wire { Charge(user: String, cents: i64) -> Receipt = charge; } }

okay::needs! {
    pub async fn checkout(cx: [Inventory, Payments, Notify], user: &str, item: &str, cents: i64) -> Receipt { … }
}
let cx = Cx(OverDb::new(&db), Cx(Audited::new(Stripe::new(&http), Limit(1000)), Cx(Mailbox(sent), End)));
run(checkout(&cx, "bob", "book", 2000));
```

| the context | what it gives | µs per 1000 operations |
|---|---|---|
| `Cx(h, …)` (`okay::typed`) | handlers known at compile time: monomorphized, inlined | 0.28 (Scala 5.6) |
| `Audited::new(h, interceptor)` | control and audit: each operation passed on, changed, answered, refused | 0.28 |
| `Policies<E>` under `Audited` | an interceptor's rules set at run time (a configuration, a tenant) | 2.6, ~1 ns a rule |
| `observe(cx, watch, …)` | every operation of every effect seen, let through or refused | 12 |
| `okay::host` (`host.outside::<E>()`) | the program HANDED OUT: stepped by a host (resume, fail, cancel), middleware, a record and its replay, okay's wire | 31 a step answered typed, 59 through a route as data |

And in it: `either`/`raise`, `generate`, `stream` (an `Iterator`; `emit_all`, `merge` round robin), branches on one
thread (`join`, `both`, `race`), guards (`ensure`, `recover`, `uncancelable`), effects waiting on the world
(`block_on`, any executor), a state whose type the steps move (by ownership), `tail_rec`. The services as effects
(`okay::services`): `Clock` (virtual — waits at once — or the system's), `Random` (seeded), `Log`, `Env`, `Secrets`
(SENSITIVE: read only under an audit), `Fresh`, `Store`. Control (`okay::control`): `optional`/`exit`, `retry`,
`backoff`, `timeout`, `scope` (tasks beside its body, cancelled at its end; resources released, the last first). The one API: [One API](#one-api-compiled-by-the-handlers-type-okaytyped) and
[Handed out to a host, branches, streams](#handed-out-to-a-host-branches-streams); the plan it follows:
[specs/one-api.md](specs/one-api.md).

THE SPECIFICATION is okay-core's kernel ported as a machine (`okay-machine`: `Eff`, a program as data): okay-core's
suites, a reference interpreter on random programs, okay-kernel's TCK digest. The machine is also the backend of the rest
taken MANY times (a choice, every path: the rest as data) and of what an `async`
body cannot be: answer-type modification (`Cont`, `$`, `shift0`, `PState`).

`examples/shop`: the shop of okay's examples on the one API — one checkout, in process under production and test
handlers, and handed out: a manager, middleware, a recording and its replay, the wire.
`examples/okayhost/TestOkayRustWire.scala`: the same shop under Scala handlers in okay, run against okay's JVM host.

```
cargo test --workspace
```

This repository was okay-rs (github.com/okay-effects/okay-rs, its history kept here). okay-effects (capabilities on
async/await), `okay::direct` (effects passed down), the one API's replay and `okay::dynamic` were here until
2026-10-27: the one API took over each part, one-shot by design (git history keeps them).

## The rest taken many times (`okay::resumable`)

A handler resuming the rest many times (every path of a choice, a search, backtracking) needs the rest as a VALUE it
can copy. An `async` body's future cannot be cloned, so the one API is one-shot; a program whose rest is taken many
times is a RESUMABLE fn — its rest an explicit `Clone` value, compiled, no replay (the body is never run again):

```rust
use okay::resumable::*;
use okay::Answer;
use okay_resumable::resumable;

okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
okay::ops! { pub enum ChooseOps { Choose } }    // the operations it may stop at: an enum, `From` each effect

#[resumable(ChooseOps)]
fn pick(base: usize) -> usize {
    let a: usize = ask!(Choose::Among(3));      // a stop: the operation given to the handler
    let b: usize = ask!(Choose::Among(2));
    base + a * 10 + b
}

// a handler: a function over the steps — every path, the rest resumed on a clone for each alternative
fn all<K: Rest>(mut k: K, answer: Answer, out: &mut Vec<K::Out>) where K::Op: OpOf<Choose> {
    match k.resume(answer) {
        Step::Done(a) => out.push(a),
        Step::Perform(op) => match op.op() {
            Ok(Choose::Among(n, r)) => (0..n).for_each(|i| all(k.clone(), r.give(i), out)),
            Err(_) => unreachable!(),
        },
    }
}
let mut out = vec![];
all(pick(100), unit(), &mut out);              // a program not started is a rest: started with `unit()`
```

Two ways, one `Rest`/`Step` (`okay::resumable`), the same handlers:

- **stable** — `#[resumable(Ops)]` (crate `okay-resumable`): the function made an enum, one variant per stop holding
  the variables in scope there (typed: `let x: T = …` for one living across a stop); each variant's resumption is
  straight code to the next stop. Lowers `if`, `while`, `loop`, `match` arms, blocks, `break`/`continue`/`return`; a
  stop inside an expression, a `for` holding a stop and type parameters are errors saying so.
- **nightly** — `--features coroutines`: `co(okay::coroutine!(move -> T { … }))`, any Rust inside (`for` too), the
  coroutine `Clone` (`coroutine_clone`). `tests/resumable_co.rs`.

A rest is resumed IN PLACE (`resume(&mut self, answer)`): a step moves nothing but the operation; a clone is a copy
of the variables at the stop, no allocation.

µs per run, n = 1000, the handlers' answers through `black_box` (`resumable/benches/lanes.rs`, three alternating
rounds, 2026-10-28, medians):

| lane | `#[resumable]` | coroutines | okay-goose | Scala (okay-core's kernel) |
|---|---|---|---|---|
| every path of a choice among 1000 | 1.62 | **1.57** | 1.23 | 13.24 |
| a deep handler taking the rest at each of 1000 steps | 0.59 | **0.46** | 1.23 | 9.44 |
| a generator of 1000 values, summed | 0.30 | **0.29** | 1.21 | 22.6 |

A path of a choice is a clone, a resumption and a handler's call: ~1.6 ns. The single-resume lanes are at the floor
of the `black_box` around each answer: the loop is inlined whole.

## One API, compiled by the handler's type (`okay::typed`)

`eff! { x <- op(cx); …; yield v }` — the default form of `eff!` — is an `async` block: a state machine the
compiler builds, generic over its context `cx` (the handlers in force, `Cx(h, rest)`, each effect found by its type as
in a row). Monomorphized for the handlers it runs under, an operation is compiled by what its handler is:

- answering in place (a reader, a state, a writer, an audit): `Ready` at once — inlined, a call of the handler;
- dropping the rest (`either`): the body's future dropped where it raised;
- taking the rest once (`generate`): the block suspended, resumed once — the state machine itself the `k`, nothing on
  the heap;
- taken many times: not in an `async` body — ONE-SHOT by design; the rest taken many times is a resumable fn's
  ([below](#the-rest-taken-many-times-okayresumable)) or the machine's (the rest as data). Never a replay.

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


With the context visible, LLVM folds the loops of `reads` and `state` altogether (0.017).

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

The layers the same over a wait: a watch sees it when it comes, an interceptor waits for its handler (`pass.wait(…).await` in its own `Step::Wait`), `either`
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
is the context it is given. What capabilities did (okay-effects, now gone), the one API does with contexts:

- **A capability is a context**: the one a program passes picks the handler — two instances of one effect are two
  contexts (or one over the other, the index written: `ask_i::<_, Here>(&nested)`); a context cannot outlive its
  handlers (a borrow: `compile_fail`). `tests/capabilities.rs`.
- **Handed out (`okay::host`)**: `start(|host| async move { let cx = Cx(host.outside::<Inv>(), …); checkout(&cx, …).await })`
  — each operation of an effect left `outside` stops the program as `Asked` (typed: `get::<Inv>()`; data: `name`,
  `record`), answered by the host (`resume`, `resume_data`), failed (raised where performed), or the program
  cancelled (dropped). A sensitive effect is outside only under its interceptor, as any handler of it:
  `Audited::new(host.outside::<Pay>(), Limit(1000))` — a charge over the limit refused before the host sees it.
  Around the host: `drive`, middleware (`with`, `only::<E>`, `record`, `replay`), `Routes` (`on::<E>(|op| … r.give(v))`,
  or `handler::<E>(h)` — the same handler a context takes), `remote` (okay's wire). `tests/host.rs`.
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
  nested innermost first, a finalizer's panic replacing the one before. `recover(body, f)`
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


### Services and control

The world a program asks of — the time, randomness, a log, the environment, a store — is effects, so a test answers
at once what production answers for real: the same program.

```rust
okay::needs! {
    async fn roll(cx: [Clock, Random, Log]) -> u64 {
        let t = now_ms(cx).await;
        let n = random_below(cx, 6).await + 1;
        info(cx, format!("rolled {n} at {t}")).await;
        n
    }
}
let test = Cx(VirtualClock::at(1_000), Cx(SplitMix::seeded(7), Cx(LogCollect::at_least(Level::Info), End)));
let prod = Cx(SystemClock, Cx(SplitMix::from_time(), Cx(LogTo::new(Level::Info, |_, s| eprintln!("{s}")), End)));
```

`Secrets` is SENSITIVE: a secret is read only under an audit of it — `Audited::new(env, interceptor)` sees every
read and may refuse it; a `Secret` prints as `Secret(***)`. `tests/services.rs`.

`backoff(cx, times, first_ms, factor, retry_on, body)` waits on the context's clock between runs — on a virtual one,
at once; `timeout(cx, ms, body)` races a timer on the system's clock and compares deadlines on a virtual one; a
`scope(async |s| …)` runs `s.spawn(task)` beside its body (`join`, `cancel`), and at its end — a value, an error raised
past it, a panic, a host's cancel — cancels the tasks left and releases what it acquired, the last first.
`tests/control.rs`.

## Speed

The lanes of both crates (`machine/benches/kernel.rs`: the machine's, the one API's `T:` and the host's `H:`), their
history and what was tried: [machine/README.md](machine/README.md#speed), `machine/benches/history.tsv`.
