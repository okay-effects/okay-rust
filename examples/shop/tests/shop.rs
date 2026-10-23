use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use okay::host::{
    Answerer, Asked, Computation, Entry, Routes, Step, drive, every_path, only, record, remote, replay, start, with,
};
use okay::typed::*;
use okay::{Data, IsData};
use okay_shop::*;

/// the checkout handed out: its effects answered by whoever steps it — payments still under their interceptor
fn handed_out(user: &'static str, item: &'static str, cents: i64) -> Computation<'static, Receipt> {
    start(move |h| async move {
        let cx = Cx(
            h.outside::<Inventory>(),
            Cx(Audited::new(h.outside::<Payments>(), Allow), Cx(h.outside::<Notify>(), End)),
        );
        checkout(&cx, user, item, cents).await
    })
}

/// a database driver for the production assembly: the stock table in memory, behind the Connection trait
struct StockDb {
    qty: RefCell<HashMap<String, i64>>,
    seen: RefCell<Vec<String>>,
}
impl Connection for StockDb {
    fn execute(&self, sql: &str, args: &[Data]) -> Result<i64, String> {
        self.seen.borrow_mut().push(sql.into());
        if sql != RESERVE_SQL {
            return Err(format!("syntax error at: {sql}"));
        }
        let sku = String::from_data(&args[0])?;
        let mut q = self.qty.borrow_mut();
        match q.get_mut(&sku) {
            Some(n) if *n > 0 => {
                *n -= 1;
                Ok(1)
            }
            _ => Ok(0),
        }
    }
}
struct Down;
impl Connection for Down {
    fn execute(&self, _: &str, _: &[Data]) -> Result<i64, String> {
        Err("database is down".into())
    }
}

fn db(qty: i64) -> Rc<StockDb> {
    Rc::new(StockDb { qty: RefCell::new(HashMap::from([("book".into(), qty)])), seen: RefCell::default() })
}

#[test]
fn production_assembly() {
    let (calls, sent): (Log, Log) = Default::default();
    let d = db(1);
    let got = production(d.clone(), calls.clone(), sent.clone(), "bob", "book", 2000);
    assert_eq!(got, Receipt { paid: true, detail: "ch_42".into() });
    assert_eq!(*calls.borrow(), vec!["POST https://api.stripe.com/v1/charges customer=bob&amount=2000".to_string()]);
    assert_eq!(*sent.borrow(), vec!["bob: paid, receipt ch_42".to_string()]);
    assert_eq!((d.qty.borrow()["book"], d.seen.borrow().len()), (0, 1));
    // out of stock now: never charged
    let again = production(d.clone(), calls.clone(), sent.clone(), "eve", "book", 2000);
    assert!(!again.paid && calls.borrow().len() == 1);
}

#[test]
fn test_assembly() {
    let sent: Log = Default::default();
    let items: Stock = Rc::new(RefCell::new(HashMap::from([("book".into(), 1)])));
    let got = test(items.clone(), false, sent.clone(), "ann", "book", 2000);
    assert_eq!(got, Receipt { paid: false, detail: "declined".into() });
    assert_eq!((sent.borrow()[0].as_str(), items.borrow()["book"]), ("ann: not paid: declined", 0));
}

#[test]
fn a_database_failure_is_raised_where_stock_performed_it() {
    let sent: Log = Default::default();
    let s2 = sent.clone();
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        production(Rc::new(Down), Default::default(), s2, "bob", "book", 2000)
    }));
    assert_eq!(p.unwrap_err().downcast_ref::<String>().map(|s| s.as_str()), Some("database is down"));
    assert!(sent.borrow().is_empty()); // stopped at Reserve: no mail
}

#[test]
fn a_limit_ends_the_whole_checkout() {
    let sent: Log = Default::default();
    let items: Stock = Rc::new(RefCell::new(HashMap::from([("book".into(), 2)])));
    let got = limited(items.clone(), 1000, sent.clone(), "bob", "book", 2000);
    assert_eq!(got, Receipt { paid: false, detail: "needs approval".into() });
    assert!(sent.borrow().is_empty());
    let got = limited(items, 1000, sent.clone(), "bob", "book", 500);
    assert_eq!(got, Receipt { paid: true, detail: "test-bob".into() });
    assert_eq!(sent.borrow().len(), 1);
}

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn a_checkout_waits_for_a_manager() {
    let mut run = handed_out("bob", "book", 2000);
    let Step::Yielded(a) = run.step() else { panic!() };
    assert!(matches!(a.get::<Inventory>(), Some(Inventory::Reserve(item, _)) if item == "book"));
    let Step::Yielded(a) = run.resume(true) else { panic!() };
    assert!(matches!(a.get::<Payments>(), Some(Payments::Charge(user, 2000, _)) if user == "bob"));
    assert_eq!(
        (a.name(), a.record().clone()),
        ("charge", Data::Dict(vec![("user".into(), "bob".to_string().to_data()), ("cents".into(), 2000i64.to_data())]))
    );
    let mut pending = run; // kept as a value: its caller is not stuck inside a call
    let Step::Yielded(a) = pending.resume(Receipt { paid: true, detail: "approved by ann".into() }) else { panic!() };
    assert!(matches!(a.get::<Notify>(), Some(Notify::Email(_, text, _)) if text == "paid, receipt approved by ann"));
    let Step::Done(r) = pending.resume(()) else { panic!() };
    assert_eq!(r.detail, "approved by ann");
}

#[test]
fn fail_and_cancel() {
    let mut c = handed_out("bob", "book", 2000);
    c.step();
    let Step::Failed(why) = c.fail("no such sku".into()) else { panic!() };
    assert_eq!(why, "no such sku");

    let cleaned = Rc::new(Cell::new(false));
    let k = cleaned.clone();
    let mut c = start(move |h| async move {
        let _guard = Cleanup(k);
        reserve(&Cx(h.outside::<Inventory>(), End), "x".into()).await
    });
    c.step();
    assert!(matches!(c.cancel(), Step::Cancelled));
    assert!(cleaned.get()); // dropped: its destructors ran
}

fn services<'a>(db: &'a Cx<Table, End>, sent: Log) -> Answerer<'a> {
    Routes::new()
        .handler::<Inventory>(OverDb::new(db))
        .on::<Payments>(|op| match op {
            Payments::Charge(user, _, r) => Ok(r.give(Receipt { paid: true, detail: format!("ch_{user}") })),
        })
        .handler::<Notify>(Mailbox(sent))
        .answerer()
}

#[test]
fn every_operation_through_the_same_middleware() {
    let sent: Log = Default::default();
    let audit: Log = Default::default();
    let counts: Rc<RefCell<HashMap<String, i32>>> = Default::default();
    let log: Rc<RefCell<Vec<Entry>>> = Default::default();
    let allowed = ["bob"];

    let (a2, c2) = (audit.clone(), counts.clone());
    let db = Cx(Table(Rc::new(RefCell::new(HashMap::from([("book".into(), 5)])))), End);
    let mut answer = with(
        services(&db, sent.clone()),
        vec![
            // all of them alike: an audit trail, a metric, a recording
            Box::new(move |a: &Asked, next: &mut dyn FnMut(&Asked) -> Result<Data, String>| {
                let v = next(a);
                a2.borrow_mut().push(format!("{a:?} -> {v:?}"));
                v
            }),
            Box::new(move |a: &Asked, next: &mut dyn FnMut(&Asked) -> Result<Data, String>| {
                *c2.borrow_mut().entry(a.name().into()).or_default() += 1;
                next(a)
            }),
            record(log.clone()),
            // one of them its own way: only these users may be charged, above 5000 nobody
            only::<Payments>(move |op, a, next| {
                let Payments::Charge(user, cents, _) = op;
                if !allowed.contains(&user.as_str()) || *cents > 5000 {
                    return Err(format!("forbidden: charge {user}"));
                }
                next(a)
            }),
        ],
    );

    assert_eq!(
        drive(handed_out("bob", "book", 2000), &mut answer),
        Ok(Receipt { paid: true, detail: "ch_bob".into() })
    );
    assert!(audit.borrow()[1].starts_with("charge Dict([(\"user\", Str(\"bob\")), (\"cents\", Int(2000))]) -> Ok("));
    // refused by security: raised where Charge was performed, so no mail goes out
    assert_eq!(drive(handed_out("eve", "book", 100), &mut answer), Err("forbidden: charge eve".into()));
    assert_eq!((sent.borrow().len(), counts.borrow()["charge"], counts.borrow()["email"]), (1, 2, 1));

    // replay: bob's run again from the recording — the same receipt, no service reached
    let bob: Vec<Entry> = log.borrow()[..3].to_vec();
    assert_eq!(
        drive(handed_out("bob", "book", 2000), replay(bob.clone())),
        Ok(Receipt { paid: true, detail: "ch_bob".into() })
    );
    // a run that does something else is caught at the first difference
    assert!(drive(handed_out("bob", "book", 3000), replay(bob)).unwrap_err().contains("diverged at 1"));
}

#[test]
fn every_path_of_a_checkout() {
    let got = every_path(
        |a| match a.name() {
            "reserve" => vec![true.to_data(), false.to_data()],
            "charge" => vec![
                Receipt { paid: true, detail: "ch".into() }.to_data(),
                Receipt { paid: false, detail: "declined".into() }.to_data(),
            ],
            _ => vec![Data::Null],
        },
        || handed_out("bob", "book", 2000),
    )
    .unwrap();
    let details: Vec<String> = got.into_iter().map(|r| r.detail).collect();
    assert_eq!(details, vec!["ch", "declined", "book: out of stock"]);
}

#[test]
fn the_shop_steps_over_the_wire() {
    let log: Log = Default::default();
    let l = log.clone();
    let far = move |name: &str, args: Vec<Data>| -> Result<Data, String> {
        let rec = &args[0];
        match name {
            "reserve" => Ok(Data::Bool(rec.field("item")? == &Data::Str("book".into()))),
            "charge" => Ok(Data::Dict(vec![
                ("paid".into(), Data::Bool(true)),
                ("detail".into(), Data::Str(format!("ch_{}", String::from_data(rec.field("user")?)?))),
            ])),
            "email" => {
                l.borrow_mut().push(format!(
                    "{}: {}",
                    String::from_data(rec.field("to")?)?,
                    String::from_data(rec.field("text")?)?
                ));
                Ok(Data::Null)
            }
            other => Err(format!("no {other}")),
        }
    };
    let mut answer = remote(far);
    assert_eq!(drive(handed_out("bob", "book", 2000), &mut answer).unwrap().detail, "ch_bob");
    assert_eq!(drive(handed_out("eve", "pen", 100), &mut answer).unwrap().detail, "pen: out of stock");
    assert_eq!(
        *log.borrow(),
        vec!["bob: paid, receipt ch_bob".to_string(), "eve: not paid: pen: out of stock".to_string()]
    );
    assert_eq!(
        drive(handed_out("x", "book", 1), remote(|n: &str, _| Err(format!("no {n}")))),
        Err("no reserve".into())
    );
}

#[test]
fn serve_a_rust_handler() {
    let f = okay::wire::serve(|op: Payments| match op {
        Payments::Charge(user, cents, r) => r.give(Receipt { paid: cents <= 5000, detail: user }),
    });
    let rec = Data::Dict(vec![("user".into(), "ann".to_string().to_data()), ("cents".into(), 300i64.to_data())]);
    assert_eq!(f("charge", &rec), Some(Ok(Receipt { paid: true, detail: "ann".into() }.to_data())));
}
