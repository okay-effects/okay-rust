//! okay-core's TestRaise: what user code throws — in Rust, panics — caught at the run, `Failed`; `run` rethrows it

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay::effects::*;
use okay::*;

fn failed<Q, A: std::fmt::Debug + 'static>(s: Step<Q, A>) -> String {
    match s {
        Step::Failed(e) => e.message(),
        other => panic!("not failed: {other:?}"),
    }
}

/// "a throw in a map under a state: Failed, the state's level gone with it; run rethrows"
#[test]
fn a_throw_in_a_map_under_a_state_failed_run_rethrows() {
    type R = row![State<i64>];
    let p = || get::<i64, R, _>().map(|s| if s == 0 { panic!("boom") } else { s }).handle(state(0));
    assert_eq!(failed(p().step()), "boom");
    let e = catch_unwind(AssertUnwindSafe(|| p().run())).unwrap_err();
    assert_eq!(Failure::from_panic(e).message(), "boom");
    assert_eq!(get::<i64, R, _>().map(|s| s + 1).handle(state(0)).run(), (0, 1));
}

/// "a throw in each kind of user code: a bind, a value, a clause, a ret, a deferral"
#[test]
fn a_throw_in_each_kind_of_user_code() {
    type R = row![Reader<i64>];
    let in_bind = ask::<i64, R, _>().flat_map(|_| -> Eff<R, i64> { panic!("boom") }).handle(reader(1));
    let in_value = ask::<i64, R, _>().handle(Handler::answering(|_: Reader<i64>| -> Answer { panic!("boom") }, |a| a));
    let in_clause =
        ask::<i64, R, _>().handle(Handler::deep(|a| pure(a), |_: Reader<i64>, _| -> Eff<Nil, i64> { panic!("boom") }));
    let in_ret = ask::<i64, R, _>().handle(Handler::answering(|op: Reader<i64>| match op { Reader::Ask(r) => r.give(1) },
        |_: i64| -> i64 { panic!("boom") }));
    let in_defer = defer::<Nil, i64>(|| panic!("boom"));
    for p in [in_bind, in_value, in_clause, in_ret, in_defer] {
        assert_eq!(failed(p.step()), "boom");
    }
}

/// "a throw in k resumed twice by choose: the whole run Failed, the other path not run"
#[test]
fn a_throw_in_k_resumed_twice_by_choose() {
    type R = row![Choose];
    let ran = Rc::new(RefCell::new(Vec::new()));
    let r2 = ran.clone();
    let p = flip::<R, _>().map(move |b| { r2.borrow_mut().push(b); if b { panic!("boom") } else { 1 } }).handle(all());
    assert_eq!(failed(p.step()), "boom");
    assert_eq!(*ran.borrow(), vec![true]);
}

/// "a step failed by its driver: the error raised where the operation was"
#[test]
fn a_step_failed_by_its_driver() {
    type R = row![Reader<i64>];
    let p = ask::<i64, R, _>().map(|n| n + 1);
    let Step::Yield(y) = p.step() else { panic!("not a step") };
    assert_eq!(failed(y.fail(Failure::new("boom".to_string()))), "boom");
}

/// "stack safe: a throw under 100 000 handlers and binds, unwound"
#[test]
fn stack_safe_a_throw_under_many_binds_unwound() {
    type R = row![Reader<i64>];
    fn deep(i: i64) -> Eff<R, i64> {
        if i == 0 { pure(0).map(|_: i64| -> i64 { panic!("boom") }) } else { defer(move || deep(i - 1)).map(|n| n + 1) }
    }
    assert_eq!(failed(deep(100_000).handle(reader(1)).step()), "boom");
}
