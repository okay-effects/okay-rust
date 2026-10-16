//! An application as capabilities — the same shop as in okay's Scala and Java
//! examples, okay-go and okay-python.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use okay_effects::{answering, data, handle, op, run, Cap, Data, Host, Reply, Run};

// ---- the services: operations, and their typed faces over a capability

data!(pub struct Receipt { pub paid: bool, pub detail: String });
op!(pub struct Reserve { pub item: String } -> bool);
op!(pub struct Charge { pub user: String, pub cents: i64 } -> Receipt);
op!(pub struct Email { pub to: String, pub text: String } -> ());

#[derive(Clone)]
pub struct Inventory(pub Cap<Reserve>);
impl Inventory {
    pub async fn reserve(&self, item: &str) -> bool {
        self.0.perform(Reserve { item: item.into() }).await
    }
}

#[derive(Clone)]
pub struct Payments(pub Cap<Charge>);
impl Payments {
    pub async fn charge(&self, user: &str, cents: i64) -> Receipt {
        self.0.perform(Charge { user: user.into(), cents }).await
    }
}

#[derive(Clone)]
pub struct Notify(pub Cap<Email>);
impl Notify {
    pub async fn email(&self, to: &str, text: &str) {
        self.0.perform(Email { to: to.into(), text: text.into() }).await
    }
}

// ---- the business code: a plain struct, its dependencies capabilities, plain Rust

#[derive(Clone)]
pub struct Shop {
    pub inventory: Inventory,
    pub payments: Payments,
    pub mail: Notify,
}

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

// ---- a database, as an effect: a statement, its rows affected

op!(pub struct Exec { pub sql: String, pub args: Vec<Data> } -> i64);

#[derive(Clone)]
pub struct Db(pub Cap<Exec>);
impl Db {
    pub async fn exec(&self, sql: &str, args: Vec<Data>) -> i64 {
        self.0.perform(Exec { sql: sql.into(), args }).await
    }
}

/// a database driver: whatever executes a statement
pub trait Connection {
    fn execute(&self, sql: &str, args: &[Data]) -> Result<i64, String>;
}

/// production: a Connection; a failed statement is raised where it was performed
pub async fn sql<A, F: FnOnce(Db) -> Fut, Fut: Future<Output = A>>(r: &Run, conn: Rc<dyn Connection>, body: F) -> A {
    handle(r, move |op: Exec| {
        let res = conn.execute(&op.sql, &op.args);
        async move {
            match res {
                Ok(n) => Reply::Resume(n),
                Err(e) => Reply::Raise(Box::new(e)),
            }
        }
    }, |c| body(Db(c)))
    .await
}

pub type Stock = Rc<RefCell<HashMap<String, i64>>>;

/// for tests: the stock table in memory, answering the statements the shop makes
pub async fn table<A, F: FnOnce(Db) -> Fut, Fut: Future<Output = A>>(r: &Run, stock: Stock, body: F) -> A {
    handle(r, move |op: Exec| {
        let reply = if op.sql != RESERVE_SQL {
            Reply::Raise(Box::new(format!("table: no such statement: {}", op.sql)))
        } else {
            let sku = match &op.args[0] { Data::Str(s) => s.clone(), other => format!("{other:?}") };
            let mut st = stock.borrow_mut();
            match st.get_mut(&sku) {
                Some(q) if *q > 0 => { *q -= 1; Reply::Resume(1) }
                _ => Reply::Resume(0),
            }
        };
        std::future::ready(reply)
    }, |c| body(Db(c)))
    .await
}

// ---- handlers: each makes a capability and hands it to its body

pub const RESERVE_SQL: &str = "update stock set qty = qty - 1 where sku = ? and qty > 0";

/// the inventory over a database: Reserve becomes one statement, answered by whatever Db is
pub async fn stock<A, F: FnOnce(Inventory) -> Fut, Fut: Future<Output = A>>(r: &Run, db: Db, body: F) -> A {
    handle(r, move |op: Reserve| {
        let db = db.clone();
        async move { Reply::Resume(db.exec(RESERVE_SQL, vec![Data::Str(op.item)]).await == 1) }
    }, |c| body(Inventory(c)))
    .await
}

pub type Log = Rc<RefCell<Vec<String>>>;

pub async fn mailbox<A, F: FnOnce(Notify) -> Fut, Fut: Future<Output = A>>(r: &Run, sent: Log, body: F) -> A {
    answering(r, move |op: Email| sent.borrow_mut().push(format!("{}: {}", op.to, op.text)), |c| body(Notify(c))).await
}

/// for tests: payments without any HTTP
pub async fn card<A, F: FnOnce(Payments) -> Fut, Fut: Future<Output = A>>(r: &Run, approve: bool, body: F) -> A {
    answering(r, move |op: Charge| {
        if approve { Receipt { paid: true, detail: format!("test-{}", op.user) } } else { Receipt { paid: false, detail: "declined".into() } }
    }, |c| body(Payments(c)))
    .await
}

/// a business rule as a handler: over the limit, the whole body ends (no mail; its destructors run)
pub async fn limit<A, F: FnOnce(Payments) -> Fut, Fut: Future<Output = A>>(
    r: &Run, cents: i64, pay: Payments, stop: impl Fn(&Charge) -> A, body: F,
) -> A {
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
    .await
}

// a component built on another: payments over HTTP

data!(pub struct Response { pub status: i64, pub body: String });
op!(pub struct Post { pub url: String, pub body: String } -> Response);

#[derive(Clone)]
pub struct Http(pub Cap<Post>);
impl Http {
    pub async fn post(&self, url: &str, body: &str) -> Response {
        self.0.perform(Post { url: url.into(), body: body.into() }).await
    }
}

pub async fn gateway<A, F: FnOnce(Http) -> Fut, Fut: Future<Output = A>>(r: &Run, calls: Log, body: F) -> A {
    answering(r, move |op: Post| {
        calls.borrow_mut().push(format!("POST {} {}", op.url, op.body));
        Response { status: 200, body: "ch_42".into() }
    }, |c| body(Http(c)))
    .await
}

pub async fn stripe<A, F: FnOnce(Payments) -> Fut, Fut: Future<Output = A>>(r: &Run, http: Http, body: F) -> A {
    handle(r, move |op: Charge| {
        let http = http.clone();
        async move {
            let res = http.post("https://api.stripe.com/v1/charges", &format!("customer={}&amount={}", op.user, op.cents)).await;
            Reply::Resume(if res.status == 200 {
                Receipt { paid: true, detail: res.body }
            } else {
                Receipt { paid: false, detail: format!("card declined ({}): {}", res.status, res.body) }
            })
        }
    }, |c| body(Payments(c)))
    .await
}

// ---- the system: every capability made, a Shop handed to what uses it

pub fn production<Fut: Future<Output = Receipt>>(conn: Rc<dyn Connection>, calls: Log, sent: Log, use_: impl FnOnce(Shop) -> Fut) -> Receipt {
    run(|r| async move {
        let r = &r;
        mailbox(r, sent, move |mail| gateway(r, calls, move |http| stripe(r, http, move |pay| sql(r, conn, move |db| stock(r, db, move |inv| use_(Shop { inventory: inv, payments: pay, mail })))))).await
    })
}

pub fn test<Fut: Future<Output = Receipt>>(items: Stock, approve: bool, sent: Log, use_: impl FnOnce(Shop) -> Fut) -> Receipt {
    run(|r| async move {
        let r = &r;
        mailbox(r, sent, move |mail| card(r, approve, move |pay| table(r, items, move |db| stock(r, db, move |inv| use_(Shop { inventory: inv, payments: pay, mail }))))).await
    })
}

// ---- the shop handed out: its capabilities answered by whoever steps it (okay_effects::start)

pub fn remote(h: &Host) -> Shop {
    Shop { inventory: Inventory(h.outside()), payments: Payments(h.outside()), mail: Notify(h.outside()) }
}
