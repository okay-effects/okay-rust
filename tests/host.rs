//! A typed program handed out to a host (`okay::host`): okay-effects' kernel protocol, middleware, record and replay,
//! the wire, every path — on the one API, `eff!` over a context.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use okay::host::{drive, every_path, only, record, remote, replay, start, with, Answerer, Entry, Routes, Step};
use okay::typed::*;
use okay::{Data, IsData};

okay::effect! { pub enum Ask: Plain, wire { Ask(n: i64) -> i64 = ask_n; } }
okay::effect! { pub enum Inv: Plain, wire { Reserve(item: String) -> bool = reserve; } }
okay::effect! { pub enum Pay: Sensitive, wire { Charge(user: String, cents: i64) -> String = charge; } }
okay::effect! { pub enum Coin: Plain, wire { Flip() -> bool = flip_coin; } }

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

okay::needs! {
    /// the business code: one program, whoever answers its effects
    async fn checkout(cx: [Inv, Pay], user: &str, item: &str, cents: i64) -> String {
        if reserve(cx, item.to_string()).await {
            charge(cx, user.to_string(), cents).await
        } else {
            format!("{item}: out of stock")
        }
    }
}

/// a payments' guard: over a limit refused, else passed on
struct Limit(i64);
impl<R> Interceptor<Pay, R, ()> for Limit {
    fn intercept<'a, H: Handles<Pay>>(&'a self, op: Pay, pass: Pass<'a, H>, _: &'a R) -> okay::typed::Step<'a> {
        match op {
            Pay::Charge(_, cents, r) if cents > self.0 => okay::typed::Step::Now(r.give(format!("refused {cents}"))),
            Pay::Charge(u, cents, r) => match pass.perform(|r2| Pay::Charge(u, cents, r2)) {
                Some(s) => okay::typed::Step::Now(r.give(s)),
                None => okay::typed::Step::Later,
            },
        }
    }
}

/// in process: the handlers of both effects in the context
struct Shelf(RefCell<Vec<String>>);
impl Handles<Inv> for Shelf {
    fn handle(&self, op: Inv) -> okay::typed::Step<'_> {
        match op {
            Inv::Reserve(item, r) => {
                let mut s = self.0.borrow_mut();
                let had = s.iter().position(|x| *x == item).map(|i| s.remove(i)).is_some();
                okay::typed::Step::Now(r.give(had))
            }
        }
    }
}
struct Bank;
impl Handles<Pay> for Bank {
    fn handle(&self, op: Pay) -> okay::typed::Step<'_> {
        match op {
            Pay::Charge(u, c, r) => okay::typed::Step::Now(r.give(format!("{u} paid {c}"))),
        }
    }
}

#[test]
fn one_program_in_process_and_handed_out() {
    let cx = Cx(Shelf(RefCell::new(vec!["book".into()])), Cx(Audited::new(Bank, Limit(1000)), End));
    assert_eq!(run(checkout(&cx, "ann", "book", 500)), Ok("ann paid 500".to_string()));
    assert_eq!(run(checkout(&cx, "ann", "book", 500)), Ok("book: out of stock".to_string()));

    // the same program, its effects answered by a host — the payments still under their guard
    let mut c = start(|host| async move {
        let cx = Cx(host.outside::<Inv>(), Cx(Audited::new(host.outside::<Pay>(), Limit(1000)), End));
        (checkout(&cx, "bob", "pen", 5000).await, checkout(&cx, "bob", "pen", 50).await)
    });
    let mut asked = vec![];
    let mut s = c.step();
    while let Step::Yielded(a) = s {
        asked.push(format!("{a:?}"));
        s = if a.get::<Inv>().is_some() { c.resume(true) } else { c.resume("ok".to_string()) };
    }
    assert!(matches!(s, Step::Done((ref x, ref y)) if x == "refused 5000" && y == "ok"), "{s:?}");
    assert_eq!(asked.len(), 3, "the refused charge never reached the host: {asked:?}");
    assert!(asked[2].starts_with("charge"), "{asked:?}");
}

#[test]
fn answered_as_data_through_middleware_recorded_and_replayed() {
    let program = || {
        start(|host| async move {
            let cx = Cx(host.outside::<Inv>(), Cx(host.outside::<Ask>(), End));
            let n = ask_n(&cx, 20).await;
            if reserve(&cx, "book".into()).await { n + 1 } else { n }
        })
    };
    let routes = || {
        Routes::new()
            .on::<Ask>(|op| match op {
                Ask::Ask(n, r) => Ok(r.give(n * 2)),
            })
            .on::<Inv>(|op| match op {
                Inv::Reserve(item, r) => Ok(r.give(item == "book")),
            })
    };
    let log: Rc<RefCell<Vec<Entry>>> = Rc::default();
    let seen: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = seen.clone();
    let answer = with(
        routes().answerer(),
        vec![
            record(log.clone()),
            only::<Inv>(move |op, a, next| {
                let Inv::Reserve(item, _) = op;
                s.borrow_mut().push(item.clone());
                next(a)
            }),
        ],
    );
    assert_eq!(drive(program(), answer), Ok(41));
    assert_eq!(*seen.borrow(), vec!["book".to_string()]);
    let entries = log.borrow().clone();
    assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["ask_n", "reserve"]);
    assert_eq!(entries[0].record, Data::Dict(vec![("n".into(), 20i64.to_data())]));

    // the log answers the run again, nothing reached
    assert_eq!(drive(program(), replay(entries.clone())), Ok(41));
    // a run that diverges is refused
    let other = start(|host| async move {
        let cx = Cx(host.outside::<Ask>(), End);
        ask_n(&cx, 21).await
    });
    let err = drive(other, replay(entries)).unwrap_err();
    assert!(err.contains("diverged at 0"), "{err}");
}

#[test]
fn a_handler_of_the_context_answers_at_the_host() {
    let shelf = Shelf(RefCell::new(vec!["pen".into()]));
    let c = start(|host| async move {
        let cx = Cx(host.outside::<Inv>(), End);
        (reserve(&cx, "pen".into()).await, reserve(&cx, "pen".into()).await)
    });
    assert_eq!(drive(c, Routes::new().handler::<Inv>(&shelf).answerer()), Ok((true, false)));
}

#[test]
fn a_failure_raised_where_the_operation_was_performed() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let mut p = start(|host| async move {
        let _guard = Cleanup(c);
        let cx = Cx(host.outside::<Ask>(), End);
        ask_n(&cx, 1).await
    });
    assert!(matches!(p.step(), Step::Yielded(_)));
    assert!(!cleaned.get());
    assert!(matches!(p.fail("no".into()), Step::Failed(ref m) if m == "no"));
    assert!(cleaned.get(), "the program's destructors ran");
}

#[test]
fn cancelled_where_it_is() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let mut p = start(|host| async move {
        let _guard = Cleanup(c);
        let cx = Cx(host.outside::<Ask>(), End);
        ask_n(&cx, 1).await
    });
    assert!(matches!(p.step(), Step::Yielded(_)));
    assert!(matches!(p.cancel(), Step::Cancelled));
    assert!(cleaned.get());
}

#[test]
fn an_answer_as_data_decoded_or_refused() {
    let program = || {
        start(|host| async move {
            let cx = Cx(host.outside::<Ask>(), End);
            ask_n(&cx, 1).await
        })
    };
    let mut p = program();
    assert!(matches!(p.step(), Step::Yielded(_)));
    assert!(matches!(p.resume_data(&7i64.to_data()), Step::Done(7)));
    let mut p = program();
    p.step();
    assert!(matches!(p.resume_data(&Data::Str("x".into())), Step::Failed(ref m) if m.contains("did not decode")));
}

#[test]
fn over_the_wire_by_name_and_record() {
    let calls = Rc::new(RefCell::new(vec![]));
    let k = calls.clone();
    let answer: Answerer = remote(move |name, args| {
        k.borrow_mut().push((name.to_string(), args.clone()));
        match name {
            "ask_n" => Ok((i64::from_data(args[0].field("n")?)? + 1).to_data()),
            "flip_coin" => Ok(true.to_data()),
            _ => Err(format!("no {name}")),
        }
    });
    let c = start(|host| async move {
        let cx = Cx(host.outside::<Ask>(), Cx(host.outside::<Coin>(), End));
        (ask_n(&cx, 41).await, flip_coin(&cx).await)
    });
    assert_eq!(drive(c, answer), Ok((42, true)));
    assert_eq!(calls.borrow()[1], ("flip_coin".to_string(), vec![]));
}

#[test]
fn every_path_of_a_handed_out_program() {
    let got = every_path(
        |_| vec![true.to_data(), false.to_data()],
        || {
            start(|host| async move {
                let cx = Cx(host.outside::<Coin>(), End);
                let a = flip_coin(&cx).await;
                let b = flip_coin(&cx).await;
                (a, b)
            })
        },
    );
    assert_eq!(got, Ok(vec![(true, true), (true, false), (false, true), (false, false)]));
}

#[test]
fn branches_of_a_handed_out_program_ask_the_host_in_turn() {
    let mut c = start(|host| async move {
        let cx = Cx(host.outside::<Ask>(), End);
        let cx = &cx;
        join((1..=3i64).map(|i| async move { ask_n(cx, i).await + ask_n(cx, i * 10).await })).await
    });
    let mut asked = vec![];
    let mut s = c.step();
    while let Step::Yielded(a) = s {
        let Ask::Ask(n, _) = a.get::<Ask>().unwrap();
        asked.push(*n);
        s = c.resume(n * n);
    }
    assert!(matches!(s, Step::Done(ref v) if v == &vec![101, 404, 909]), "{s:?}");
    // each branch's operation answered before another branch asks: one out at a time
    assert_eq!(asked.len(), 6);
}
