# okay-rust

Algebraic effects for Rust, typed, on `async`/`.await`: the design of
[okay](https://github.com/sergey-scherbina/okay) (Scala 3),
[okay-go](https://github.com/sergey-scherbina/okay-go) and
[okay-python](https://github.com/sergey-scherbina/okay-python), in Rust.
Crate `okay-effects`; no dependencies; stable Rust.

- **A handler makes a capability and hands it to its body.** The body performs
  through it: `let ok = inventory.reserve(item).await` — plain Rust, typed by
  what the operation answers.
- **Programs are futures.** An `async fn` is a stackless coroutine: performing
  suspends the body, the handler that owns the capability answers, the body
  resumes. No executor needed; real async (timers, sockets) passes through.
- **By identity, not by type.** A handler answers only its own capability's
  operations; two instances of one effect coexist; an operation of another passes
  outward. A capability that escaped its handler is refused by name.
- **Components on components.** A handler's answer may perform through other
  capabilities: the inventory as a statement on a `Db` effect, payments over HTTP.
- **Abort and cancel drop the body**: its destructors run — Rust's `defer`.
- **The kernel's protocol.** A program handed out and stepped by a host that sees
  only operations and answers: `step`, `resume`, `fail` (a panic at the
  operation), `cancel`.
- **Middleware.** Every operation answered in one place, through audit, metrics,
  security, `record` and `replay` — all alike, or one kind its own way.
- **Every path.** Every outcome of a checkout in one test.
- **Over the wire, both ways.** An operation crosses as a name and a record — a
  Scala case class's shape. With okay's Rust worker, the shop runs in Rust while
  Scala handlers answer it, and a Rust handler answers okay's operations.

**Documentation:** [the guide](docs/guide.md), and `cargo doc --open`.

## The shop

```rust
op!(pub struct Reserve { pub item: String } -> bool);
op!(pub struct Charge { pub user: String, pub cents: i64 } -> Receipt);

impl Shop {
    pub async fn checkout(&self, user: &str, item: &str, cents: i64) -> Receipt {
        let mut r = Receipt { paid: false, detail: format!("{item}: out of stock") };
        if self.inventory.reserve(item).await {
            r = self.payments.charge(user, cents).await;
        }
        if r.paid {
            self.mail.email(user, &format!("paid, receipt {}", r.detail)).await;
        } else {
            self.mail.email(user, &format!("not paid: {}", r.detail)).await;
        }
        r
    }
}
```

A handler, the inventory over a database effect:

```rust
pub async fn stock<A, F: FnOnce(Inventory) -> Fut, Fut: Future<Output = A>>(r: &Run, db: Db, body: F) -> A {
    handle(r, move |op: Reserve| {
        let db = db.clone();
        async move { Reply::Resume(db.exec(RESERVE_SQL, vec![Data::Str(op.item)]).await == 1) }
    }, |c| body(Inventory(c)))
    .await
}
```

The system:

```rust
pub fn production<Fut: Future<Output = Receipt>>(conn: Rc<dyn Connection>, calls: Log, sent: Log,
                                                 use_: impl FnOnce(Shop) -> Fut) -> Receipt {
    run(|r| async move {
        let r = &r;
        mailbox(r, sent, move |mail| gateway(r, calls, move |http| stripe(r, http, move |pay|
            sql(r, conn, move |db| stock(r, db, move |inv| use_(Shop { inventory: inv, payments: pay, mail }))))))
        .await
    })
}

let receipt = production(conn, calls, sent, |shop| async move { shop.checkout("bob", "book", 2000).await });
```

`examples/shop`: `sql` over any `Connection`, `table` and `card` for tests,
`stripe` over `Http`, `limit` (a business rule that aborts the checkout).

## Tests

```
cargo test --workspace
cargo clippy --workspace --all-targets
```

`examples/okayhost/TestOkayRustWire.scala`: the same shop under Scala handlers
in okay, and a Rust handler answering okay, run against okay's JVM host.

## License

Apache 2.0, as okay.
