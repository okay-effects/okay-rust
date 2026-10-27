//! Guards on the one API (okay::typed `ensure`, `recover`, `uncancelable`): tests/guard.rs's programs, as far as an
//! async body has them — a finalizer after a value, a panic, an error raised past it, a host's fail and cancel; a
//! failure recovered, not a raise; a host's cancel waiting for an uncancelable region
use std::cell::RefCell;
use std::rc::Rc;

use okay::effects::Writer;
use okay::host::{Step as HostStep, start};
use okay::typed::*;

okay::effect! { pub enum Ask: Plain, wire { Ask() -> i64 = ask_i; } }

type Log = Rc<RefCell<Vec<String>>>;

/// a writer telling into a log shared with the test
struct Told(Log);
impl Handles<Writer<String>> for Told {
    fn handle(&self, op: Writer<String>) -> Step<'_> {
        match op {
            Writer::Tell(s, r) => {
                self.0.borrow_mut().push(s);
                Step::Now(r.give(()))
            }
        }
    }
}

fn panicked<A>(f: impl FnOnce() -> A) -> String {
    let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).err().expect("a panic");
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default()
}

#[test]
fn ensuring_the_finalizer_after_the_program_its_value_kept() {
    let log: Log = Rc::default();
    let cx = Cx(Told(log.clone()), End);
    let got = run(ensure(
        async {
            tell(&cx, "body".to_string()).await;
            7
        },
        || tell(&cx, "fin".to_string()),
    ));
    assert_eq!(got, Ok(7));
    assert_eq!(*log.borrow(), vec!["body", "fin"]);
}

#[test]
fn ensuring_and_a_raise_the_finalizer_then_the_error_out() {
    let log: Log = Rc::default();
    let cx = Cx(Told(log.clone()), End);
    let got = run(either(&cx, async |cx| {
        ensure(async { raise::<String, i64, _, _>(cx, "no".into()).await }, || tell(cx, "fin".to_string())).await
    }));
    assert_eq!(got, Ok(Err("no".to_string())));
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn a_panic_the_finalizer_runs_the_panic_reaches_the_top_as_itself() {
    let log: Log = Rc::default();
    let cx = Cx(Told(log.clone()), End);
    let why = panicked(|| run(ensure(async { panic!("boom") }, || tell(&cx, "fin".to_string()))));
    assert_eq!(why, "boom");
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn recover_catches_a_panic_not_a_raise() {
    let thrown = run(recover(async { panic!("boom") }, |e| if e == "boom" { 7 } else { -1 }));
    assert_eq!(thrown, Ok(7));
    let raised = run(either(&End, async |cx| {
        recover(async { raise::<String, i64, _, _>(cx, "no".into()).await }, |_| -1).await
    }));
    assert_eq!(raised, Ok(Err("no".to_string())));
}

#[test]
fn guards_nested_unwound_innermost_first_a_finalizers_panic_replaces_the_error() {
    let log: Log = Rc::default();
    let cx = Cx(Told(log.clone()), End);
    let why = panicked(|| {
        run(ensure(ensure(async { panic!("boom") }, || tell(&cx, "inner".to_string())), || async {
            tell(&cx, "outer".to_string()).await;
            panic!("second")
        }))
    });
    assert_eq!(why, "second");
    assert_eq!(*log.borrow(), vec!["inner", "outer"]);
}

// a step failed, cancelled — the finalizers in the program run

#[test]
fn a_hosts_fail_the_finalizers_run_then_failed() {
    let log: Log = Rc::default();
    let l = log.clone();
    let mut c = start(move |host| async move {
        let cx = Cx(host.outside::<Ask>(), Cx(Told(l), End));
        ensure(async { ask_i(&cx).await + 1 }, || tell(&cx, "fin".to_string())).await
    });
    assert!(matches!(c.step(), HostStep::Yielded(_)));
    assert!(matches!(c.fail("boom".into()), HostStep::Failed(ref m) if m == "boom"));
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn a_hosts_fail_a_recover_in_the_program_given_the_error() {
    let mut c = start(|host| async move {
        let cx = Cx(host.outside::<Ask>(), End);
        recover(async { ask_i(&cx).await + 1 }, |e| if e == "boom" { 7 } else { -1 }).await
    });
    c.step();
    assert!(matches!(c.fail("boom".into()), HostStep::Done(7)));
}

#[test]
fn a_hosts_cancel_the_finalizers_run_no_recover_asked() {
    let log: Log = Rc::default();
    let l = log.clone();
    let mut c = start(move |host| async move {
        let cx = Cx(host.outside::<Ask>(), Cx(Told(l), End));
        ensure(recover(async { ask_i(&cx).await + 1 }, |_| -1), || tell(&cx, "fin".to_string())).await
    });
    c.step();
    assert!(matches!(c.cancel(), HostStep::Cancelled));
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn a_cancel_under_uncancelable_the_step_given_back_fired_when_the_region_is_left() {
    let log: Log = Rc::default();
    let l = log.clone();
    let mut c = start(move |host| async move {
        let cx = Cx(host.outside::<Ask>(), Cx(Told(l), End));
        let cx = &cx;
        ensure(
            async {
                let n = uncancelable(async {
                    let n = ask_i(cx).await;
                    tell(cx, format!("in {n}")).await;
                    n
                })
                .await;
                tell(cx, "after".to_string()).await;
                n
            },
            || tell(cx, "fin".to_string()),
        )
        .await
    });
    let HostStep::Yielded(first) = c.step() else { panic!("an ask") };
    let HostStep::Yielded(again) = c.cancel() else { panic!("the same step given back") };
    assert_eq!(format!("{again:?}"), format!("{first:?}"));
    assert!(matches!(c.resume(5i64), HostStep::Cancelled));
    assert_eq!(*log.borrow(), vec!["in 5", "fin"]);
}

#[test]
fn a_cancel_under_uncancelable_then_the_region_fails_cancelled_not_recovered() {
    let mut c = start(|host| async move {
        let cx = Cx(host.outside::<Ask>(), End);
        recover(
            uncancelable(async {
                ask_i(&cx).await;
                std::panic::resume_unwind(Box::new("boom"))
            }),
            |_| -1,
        )
        .await
    });
    c.step();
    assert!(matches!(c.cancel(), HostStep::Yielded(_)));
    // the region fails: its panic is the cancel's — not recovered
    let s = c.resume(5i64);
    assert!(matches!(s, HostStep::Cancelled), "{s:?}");
}
