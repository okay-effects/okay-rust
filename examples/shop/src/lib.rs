//! An application on the one API (`okay::typed`) — the same shop as in okay's Scala and Java examples, okay-go and
//! okay-python. The business code is one `needs!` function over its effects; what answers them is the context it is
//! given: production (a database, a payment gateway over HTTP), tests (a table in memory, a card), a host stepping it
//! (`okay::host`: a manager, middleware, a recording, okay's wire). Payments are SENSITIVE: they reach a program only
//! under an interceptor — here a limit, a business rule ending the whole checkout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use okay::effects::Throws;
use okay::typed::*;
use okay::{Data, IsData};

// ---- the services: effects, their operations as data (`wire`) so a host or the wire can answer them

okay::data!(
    pub struct Receipt {
        pub paid: bool,
        pub detail: String,
    }
);

okay::effect! { pub enum Inventory: Plain, wire { Reserve(item: String) -> bool = reserve; } }
okay::effect! { pub enum Payments: Sensitive, wire { Charge(user: String, cents: i64) -> Receipt = charge; } }
okay::effect! { pub enum Notify: Plain, wire { Email(to: String, text: String) -> () = email; } }

// ---- the business code: plain Rust over its effects

okay::needs! {
    pub async fn checkout(cx: [Inventory, Payments, Notify], user: &str, item: &str, cents: i64) -> Receipt {
        let mut r = Receipt { paid: false, detail: format!("{item}: out of stock") };
        if reserve(cx, item.into()).await {
            r = charge(cx, user.into(), cents).await;
        }
        if r.paid {
            email(cx, user.into(), format!("paid, receipt {}", r.detail)).await;
        } else {
            email(cx, user.into(), format!("not paid: {}", r.detail)).await;
        }
        r
    }
}

// ---- a database, as an effect: a statement, its rows affected

okay::effect! { pub enum Db: Plain, wire { Exec(sql: String, args: Vec<Data>) -> i64 = exec; } }

/// a database driver: whatever executes a statement
pub trait Connection {
    fn execute(&self, sql: &str, args: &[Data]) -> Result<i64, String>;
}

/// production: a Connection; a failed statement raised where it was performed
pub struct Sql(pub Rc<dyn Connection>);
impl Handles<Db> for Sql {
    fn handle(&self, op: Db) -> Step<'_> {
        match op {
            Db::Exec(sql, args, r) => match self.0.execute(&sql, &args) {
                Ok(n) => Step::Now(r.give(n)),
                Err(e) => std::panic::resume_unwind(Box::new(e)),
            },
        }
    }
}

pub type Stock = Rc<RefCell<HashMap<String, i64>>>;

pub const RESERVE_SQL: &str = "update stock set qty = qty - 1 where sku = ? and qty > 0";

/// for tests: the stock table in memory, answering the statements the shop makes
pub struct Table(pub Stock);
impl Handles<Db> for Table {
    fn handle(&self, op: Db) -> Step<'_> {
        match op {
            Db::Exec(sql, args, r) => {
                if sql != RESERVE_SQL {
                    std::panic::resume_unwind(Box::new(format!("table: no such statement: {sql}")));
                }
                let sku = String::from_data(&args[0]).unwrap_or_default();
                let mut st = self.0.borrow_mut();
                let n = match st.get_mut(&sku) {
                    Some(q) if *q > 0 => {
                        *q -= 1;
                        1
                    }
                    _ => 0,
                };
                Step::Now(r.give(n))
            }
        }
    }
}

/// a component built on another: the inventory over a database — Reserve one statement, answered by whatever handles
/// `Db` in the context it is given
pub struct OverDb<'o, C, I>(pub &'o C, PhantomData<I>);
impl<'o, C, I> OverDb<'o, C, I> {
    pub fn new(db: &'o C) -> Self {
        OverDb(db, PhantomData)
    }
}
impl<C: Has<Db, I>, I> Handles<Inventory> for OverDb<'_, C, I> {
    fn handle(&self, op: Inventory) -> Step<'_> {
        match op {
            Inventory::Reserve(item, r) => match now(exec(self.0, RESERVE_SQL.into(), vec![Data::Str(item)])) {
                Some(n) => Step::Now(r.give(n == 1)),
                None => Step::Later,
            },
        }
    }
}

pub type Log = Rc<RefCell<Vec<String>>>;

/// the mail sent, kept
pub struct Mailbox(pub Log);
impl Handles<Notify> for Mailbox {
    fn handle(&self, op: Notify) -> Step<'_> {
        match op {
            Notify::Email(to, text, r) => {
                self.0.borrow_mut().push(format!("{to}: {text}"));
                Step::Now(r.give(()))
            }
        }
    }
}

/// for tests: payments without any HTTP
pub struct Card(pub bool);
impl Handles<Payments> for Card {
    fn handle(&self, op: Payments) -> Step<'_> {
        match op {
            Payments::Charge(user, _, r) => Step::Now(r.give(if self.0 {
                Receipt { paid: true, detail: format!("test-{user}") }
            } else {
                Receipt { paid: false, detail: "declined".into() }
            })),
        }
    }
}

// ---- payments over HTTP

okay::data!(
    pub struct Response {
        pub status: i64,
        pub body: String,
    }
);
okay::effect! { pub enum Http: Plain, wire { Post(url: String, body: String) -> Response = post; } }

/// the HTTP client: each call logged, answered 200
pub struct Gateway(pub Log);
impl Handles<Http> for Gateway {
    fn handle(&self, op: Http) -> Step<'_> {
        match op {
            Http::Post(url, body, r) => {
                self.0.borrow_mut().push(format!("POST {url} {body}"));
                Step::Now(r.give(Response { status: 200, body: "ch_42".into() }))
            }
        }
    }
}

/// payments over the HTTP of the context it is given
pub struct Stripe<'o, C, I>(pub &'o C, PhantomData<I>);
impl<'o, C, I> Stripe<'o, C, I> {
    pub fn new(http: &'o C) -> Self {
        Stripe(http, PhantomData)
    }
}
impl<C: Has<Http, I>, I> Handles<Payments> for Stripe<'_, C, I> {
    fn handle(&self, op: Payments) -> Step<'_> {
        match op {
            Payments::Charge(user, cents, r) => {
                let body = format!("customer={user}&amount={cents}");
                match now(post(self.0, "https://api.stripe.com/v1/charges".into(), body)) {
                    Some(res) if res.status == 200 => Step::Now(r.give(Receipt { paid: true, detail: res.body })),
                    Some(res) => Step::Now(r.give(Receipt {
                        paid: false,
                        detail: format!("card declined ({}): {}", res.status, res.body),
                    })),
                    None => Step::Later,
                }
            }
        }
    }
}

// ---- the interceptors of payments: the only way a handler of them reaches a program

/// every charge passed on as it is
pub struct Allow;
impl<R> Interceptor<Payments, R, ()> for Allow {
    fn intercept<'a, H: Handles<Payments>>(&'a self, op: Payments, pass: Pass<'a, H>, _: &'a R) -> Step<'a> {
        match op {
            Payments::Charge(u, c, r) => {
                pass.perform(|r2| Payments::Charge(u, c, r2)).map_or(Step::Later, |x| Step::Now(r.give(x)))
            }
        }
    }
}

/// a business rule: over the limit, the whole checkout ends (no mail; its destructors run) — an error raised to the
/// `Raised<Receipt>` further out
pub struct Limit(pub i64);
impl<R: Has<Throws<Receipt>, I>, I> Interceptor<Payments, R, I> for Limit {
    fn intercept<'a, H: Handles<Payments>>(&'a self, op: Payments, pass: Pass<'a, H>, cx: &'a R) -> Step<'a> {
        match op {
            Payments::Charge(_, cents, _) if cents > self.0 => {
                let _: Option<()> = now(raise(cx, Receipt { paid: false, detail: "needs approval".into() }));
                Step::Later
            }
            Payments::Charge(u, c, r) => {
                pass.perform(|r2| Payments::Charge(u, c, r2)).map_or(Step::Later, |x| Step::Now(r.give(x)))
            }
        }
    }
}

// ---- the system: every handler made, the checkout run in it

pub fn production(conn: Rc<dyn Connection>, calls: Log, sent: Log, user: &str, item: &str, cents: i64) -> Receipt {
    let db = Cx(Sql(conn), End);
    let http = Cx(Gateway(calls), End);
    let cx = Cx(OverDb::new(&db), Cx(Audited::new(Stripe::new(&http), Allow), Cx(Mailbox(sent), End)));
    run(checkout(&cx, user, item, cents)).expect("every effect answered in place")
}

pub fn test(items: Stock, approve: bool, sent: Log, user: &str, item: &str, cents: i64) -> Receipt {
    let db = Cx(Table(items), End);
    let cx = Cx(OverDb::new(&db), Cx(Audited::new(Card(approve), Allow), Cx(Mailbox(sent), End)));
    run(checkout(&cx, user, item, cents)).expect("every effect answered in place")
}

/// the test assembly under a limit: the receipt, or the one the limit ended the checkout with
pub fn limited(items: Stock, cents_limit: i64, sent: Log, user: &str, item: &str, cents: i64) -> Receipt {
    let db = Cx(Table(items), End);
    let cx = Cx(
        OverDb::new(&db),
        Cx(Audited::new(Card(true), Limit(cents_limit)), Cx(Mailbox(sent), Cx(Raised::new(), End))),
    );
    let r = run(catch(&cx.1.1.1.0, checkout(&cx, user, item, cents))).expect("every effect answered in place");
    r.unwrap_or_else(|stopped| stopped)
}
