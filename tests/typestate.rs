//! A state whose type the steps move, on the one API: tests/pstate.rs's protocol and pipeline — by OWNERSHIP, not by
//! answer types. The state is a token: each step takes it and gives the next (`Sess<Open>` → `Sess<Authed>`), the
//! session must give back `Sess<Closed>`. A query before the login, a session left open, an old state used again
//! (moved) do not compile (src/guarantees.rs). The operations still go through the context: a watch sees them.
use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use okay::effects::{Reader, Writer};
use okay::typed::*;

mod db {
    use okay::effects::{Reader, Writer};
    use okay::typed::*;

    pub struct Closed;
    pub struct Open {
        pub host: String,
    }
    pub struct Authed {
        pub host: String,
        pub user: String,
    }

    /// the connection's state, a token: made here only, moved by each step
    pub struct Sess<S>(S);

    okay::needs! {
        /// a session: the body given the closed connection, owing it back closed
        pub async fn session(
            cx: [Reader<i64>, Writer<String>],
            body: impl AsyncFnOnce(Sess<Closed>, i64) -> (i64, Sess<Closed>),
        ) -> i64 {
            let port = ask(cx).await;
            body(Sess(Closed), port).await.0
        }
    }

    pub fn connect(_: Sess<Closed>, host: String) -> Sess<Open> {
        Sess(Open { host })
    }
    pub fn login(s: Sess<Open>, user: &str) -> Sess<Authed> {
        Sess(Authed { host: s.0.host, user: user.into() })
    }
    okay::needs! {
        pub async fn query(cx: [Writer<String>], s: Sess<Authed>, q: &str) -> (i64, Sess<Authed>) {
            tell(cx, format!("{}@{}: {q}", s.0.user, s.0.host)).await;
            (q.len() as i64, s)
        }
    }
    pub fn close(_: Sess<Authed>) -> Sess<Closed> {
        Sess(Closed)
    }
}

/// a watch keeping each operation
struct Seen(Rc<RefCell<Vec<String>>>);
impl Watch for Seen {
    fn after(&self, effect: &str, op: &dyn Debug, _: &dyn Debug) {
        self.0.borrow_mut().push(format!("{effect} {op:?}"));
    }
}

#[test]
fn a_protocol_connect_login_query_close() {
    let cx = Cx(AskWith(5432i64), Cx(Collect::<String>::new(), End));
    let seen = Rc::new(RefCell::new(vec![]));
    let n = run(observe(&cx, Seen(seen.clone()), async |cx| {
        db::session(cx, async |s, port| {
            let s = db::connect(s, format!("db:{port}"));
            let s = db::login(s, "alice");
            let (n, s) = db::query(cx, s, "select 1").await;
            (n, db::close(s))
        })
        .await
    }));
    assert_eq!(n, Ok(Ok(8)));
    assert_eq!(cx.1.0.take(), vec!["alice@db:5432: select 1".to_string()]);
    assert_eq!(seen.borrow().len(), 2, "the ask and the tell seen: {:?}", seen.borrow());
}

// A PIPELINE: the rows' schema the stage's type — a stage needing a column runs only after the one adding it

#[derive(Clone, Debug, PartialEq)]
struct Raw {
    id: i64,
    amount: i64,
}
#[derive(Clone, Debug, PartialEq)]
struct Enriched {
    id: i64,
    amount: i64,
    country: &'static str,
}
#[derive(Clone, Debug, PartialEq)]
struct Scored {
    id: i64,
    amount: i64,
    country: &'static str,
    risky: bool,
}

okay::needs! {
    async fn enrich(cx: [Writer<String>], rows: Vec<Raw>) -> Vec<Enriched> {
        tell(cx, format!("enriched {}", rows.len())).await;
        let country = |id: i64| if id % 2 == 0 { "PL" } else { "UA" };
        rows.into_iter().map(|x| Enriched { id: x.id, amount: x.amount, country: country(x.id) }).collect()
    }
}
okay::needs! {
    async fn score(cx: [Reader<i64>], rows: Vec<Enriched>) -> Vec<Scored> {
        let limit = ask(cx).await;
        rows.into_iter()
            .map(|x| {
                let risky = x.amount > limit && x.country != "PL";
                Scored { id: x.id, amount: x.amount, country: x.country, risky }
            })
            .collect()
    }
}

#[test]
fn a_pipeline_load_enrich_score() {
    let raw = vec![Raw { id: 1, amount: 50 }, Raw { id: 2, amount: 900 }, Raw { id: 3, amount: 700 }];
    let cx = Cx(AskWith(500i64), Cx(Collect::<String>::new(), End));
    let rows = run(async { score(&cx, enrich(&cx, raw).await).await }).unwrap();
    assert_eq!(cx.1.0.take(), vec!["enriched 3".to_string()]);
    assert_eq!(rows.iter().map(|r| r.risky).collect::<Vec<_>>(), vec![false, false, true]);
}
