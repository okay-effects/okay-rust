# okay-rust guide

Every snippet is from a test or a doctest (`cargo test --workspace`).

1. [Operations, capabilities, handlers](#1-operations-capabilities-handlers)
2. [Handler shapes](#2-handler-shapes)
3. [The shop](#3-the-shop)
4. [The kernel's protocol](#4-the-kernels-protocol)
5. [Middleware](#5-middleware)
6. [Every path](#6-every-path)
7. [Over the wire, with okay — both ways](#7-over-the-wire-with-okay)
8. [Parallel branches](#8-parallel-branches)
9. [Streams](#9-streams)
10. [How it works](#10-how-it-works)
11. [Speed](#11-speed)
12. [Limits](#12-limits)

## 1. Operations, capabilities, handlers

An operation implements `Op`, its `Answer` the type it answers. `op!` writes the
struct, `Op`, and its form as data (`Operation`: a name and a record):

```rust
op!(pub struct Reserve { pub item: String } -> bool);
data!(pub struct Receipt { pub paid: bool, pub detail: String }); // an answer as data
```

A handler makes a `Cap<O>` and hands it to its body; the body performs with
`cap.perform(op).await`, typed `O::Answer`:

```rust
let got = run(|r| async move {
    let r = &r;
    answering(r, |_: Ask| 1, |outer| answering(r, |_: Ask| 2, move |inner| async move {
        outer.perform(Ask {}).await * 10 + inner.perform(Ask {}).await
    }))
    .await
});
assert_eq!(got, 12); // two instances of one effect, told apart by capability
```

`run` runs a program whose every operation is handled inside it; a real async
wait inside parks the thread until woken.

## 2. Handler shapes

`handle(r, answer, body)`: `answer` returns a future of a `Reply`:

- `Reply::Resume(x)` — the body resumes with `x` (`answering(r, f, body)` for a
  plain function);
- **translating** — the answer `.await`s outer capabilities (`stripe` over `Http`);
- `Reply::Abort(a)` — the body is dropped where it performed, its destructors run,
  the handler's value is `a`;
- `Reply::Raise(e)` — a panic with payload `e` at the operation.

A business rule as a handler — over the limit, the whole checkout ends, no mail:

```rust
handle(r, move |op: Charge| {
    let over = (op.cents > cents).then(|| stop(&op));
    let pay = pay.clone();
    async move {
        match over {
            Some(a) => Reply::Abort(a),
            None => Reply::Resume(pay.charge(&op.user, op.cents).await),
        }
    }
}, |c| body(Payments(c)))
```

## 3. The shop

`examples/shop/src/lib.rs`: `Shop::checkout` over `Inventory`, `Payments`,
`Notify`; `stock` over a `Db` effect answered by `sql` (any `Connection`; a
failed statement raised where it was performed) or `table` (in memory);
`stripe` over `Http`; `card`; `limit`; `production` and `test` assemblies.

## 4. The kernel's protocol

```rust
let mut run = start(|h| async move { remote(&h).checkout("bob", "book", 2000).await });
let Step::Yielded(a) = run.step() else { panic!() };      // a.get::<Reserve>()
let Step::Yielded(a) = run.resume(true) else { panic!() }; // a.get::<Charge>(), a.name() == "charge"
// later, from outside:
pending.resume(Receipt { paid: true, detail: "approved by ann".into() });
```

- `h.outside::<O>()`: a capability the host answers;
- `Step::Yielded(Asked)` — the operation typed (`get::<O>()`) and as data
  (`name()`, `record()`); `Done(a)`; `Failed(message)`; `Cancelled`;
- `resume(v)` typed, `resume_data(&Data)` decoded; `fail(msg)` — a panic at the
  operation; `cancel()` — the program is dropped, its destructors run.

## 5. Middleware

```rust
let mut answer = with(services, vec![
    auditing, metrics,          // every operation alike
    record(log.clone()),
    only(move |op: &Charge, a, next| {   // Charge alone
        if !allowed.contains(&op.user.as_str()) || op.cents > 5000 {
            return Err(format!("forbidden: charge {}", op.user));
        }
        next(a)
    }),
]);
drive(checkout("bob", "book", 2000), &mut answer)
```

`Routes::new().on(|op: &Reserve| ...)` answers by type; `drive` runs a handed-out
program through an answerer; an `Err` is a failure at the operation — a refused
`Charge` stops the checkout, no mail; `record` and `replay` (a diverging run
refused at its first difference).

## 6. Every path

A future cannot be cloned, so a continuation is one-shot; `every_path` starts the
program again with the answers so far replayed:

```rust
every_path(|a| match a.name() {
    "reserve" => vec![true.to_data(), false.to_data()],
    "charge" => vec![paid.to_data(), declined.to_data()],
    _ => vec![Data::Null],
}, || checkout("bob", "book", 2000))
// ch, declined, book: out of stock
```

## 7. Over the wire, with okay

`wire::remote(call)` answers every operation through `call(name, args)`: the
operation's name (snake_case of the struct) and one record (fields in
lowerCamel). `wire::serve::<O>(answer)` is the other way.

With okay's Rust worker (`okay` crate, `okay_call`), both ways:

```rust
// the Rust shop under Scala handlers
functions.insert("checkout".into(), function(|args| {
    let run = start(move |h| async move { remote(&h).checkout(&user, &item, cents).await });
    let r = drive(run, wire::remote(|name: &str, args: Vec<Data>| {
        okay_call(Op::<Value>::new(name, args.iter().map(value).collect())).map(|v| data(&v)).map_err(|e| e.to_string())
    }))?;
    Ok(Value::Str(format!("{} {}", r.paid, r.detail)))
}));
// a Rust handler answering okay
let (name, charge) = wire::serve(|op: Charge| Receipt { paid: op.cents <= 5000, detail: format!("rust-{}-{}", op.user, op.cents) });
functions.insert(name, function(move |args| charge(args.iter().map(data).collect()).map(|d| value(&d))));
```

`data`/`value` map okay's `Value` and `Data` one for one. In Scala the
operations are case classes, the handlers callbacks under Scala's own effects:
`examples/okayhost/TestOkayRustWire.scala`, run against okay's JVM host.

## 8. Parallel branches

`all(r, branches)` runs n branches of one type, their values in order;
`par(r, a, b)` two of different types; `race(r, branches)` the first value, the
others dropped where they are (their destructors run). The branches are polled
round-robin on the run's thread, so their real waits overlap — two 50 ms sleeps
take 50 ms:

```rust
let got = run(|r| async move { all(&r, [50u64, 50].map(|ms| async move { sleep(ms).await; ms })).await });
```

A branch performs through any capability it sees — a parent's, captured as any
value is (no lending: the borrow checker keeps a capability inside its handler's
scope), or its own handlers':

```rust
answering(r, |a: Ask| a.n + 1000, |parent| async move {
    let parent = &parent;
    all(r, (1..=3i64).map(|i| answering(r, move |a: Ask| a.n * i, move |mine| async move {
        let x = mine.perform(Ask { n: 10 }).await;
        sleep(10).await; // the others run meanwhile, their handlers installed
        let y = mine.perform(Ask { n: 1 }).await;
        x + y + parent.perform(Ask { n: i }).await
    })))
    .await
})
```

The parent's handler answers the branches one operation at a time; a handed-out
program's branches ask the host in turn (`all(&h.run(), …)`). A branch that
panics fails `all` and drops the others; a parent's `Abort` drops every branch.
`tests/par.rs`.

## 9. Streams

`stream(producer)`: the values a producer emits through its `Cap<Emit<T>>`
(`out.emit(x).await`), pulled as an `Iterator`, one step of the producer per
value; breaking out of the loop drops the producer, its destructors run:

```rust
let squares: Vec<i64> = stream(|_, out| async move {
    for i in 1..=5 {
        out.emit(i * i).await;
    }
})
.collect();
```

The producer gets its run too, for handlers of its own. Inside an effectful
program, `Emit<T>` is an operation like any other: `handle` it, each value to a
consumer that may perform effects itself and stop the producer with `Abort`
(`consumed_inside_an_effectful_program`, `tests/stream.rs`).

## 10. How it works

One operation is in flight per run, in a mailbox. `perform` puts the operation
there and returns `Pending`; the handler polling its body sees an operation of
its own capability, takes it, answers (`.await`ing outer capabilities if it
translates), puts the answer back and polls again; `perform` takes the answer
and returns `Ready`. An operation that is not the handler's — or a real async
wait — makes the handler return `Pending` itself, so it reaches the next handler
out, or the executor.

Branches (`all`, `par`, `race`) share the mailbox. An operation answered inside a
branch is taken and answered within the branch's own poll; one going past the
branches stays in the mailbox across polls, and while it does only the branch
that performed it is polled — so the parent answers one at a time.

## 11. Speed

`cargo bench -p okay-effects` (`effects/benches/ops.rs`, std only; a million
operations, the median of 7; Apple M4 Max, rustc 1.92):

| lane | M ops/s | ns/op |
|---|---|---|
| `answering`: a perform answered by a function | 43.0 | 23 |
| a translating chain of depth 3 (each level performs on the next out) | 11.4 | 88 |
| `drive`: a handed-out program, each operation as data through two middleware and `Routes` | 11.5 | 87 |

## 12. Limits

- Use the branches' combinators (`all`, `par`, `race`), not `join!`/`select!`:
  they know the run's mailbox; a foreign combinator polling two branches that
  both perform past it would clash.
- Continuations are one-shot; `every_path` replays.
- A failure at an operation is a panic (`Raise`, `fail`): it unwinds, destructors
  run; catch it with `catch_unwind` if the program must.
- Which operations a capability takes is typed (`Cap<O>`); which effects a program
  needs is its capabilities — its parameters.
