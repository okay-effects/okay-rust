//! okay-core's TestGuard and TestGuardStep: guards — a level of no effect, its finalizer run when it is left, its
//! recover given a failure raised in it (a panic here), its passing told of each operation going out. The tests
//! under a `$` wait for stage 2's control.

use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay::effects::*;
use okay::*;

type Log = Rc<RefCell<Vec<String>>>;

/// a writer telling into a cell: what was told survives the level a failure drops
fn cell<T, A: 'static>(log: Log) -> Handler<Writer<String>, T, A, A> {
    Handler::answering(move |op: Writer<String>| match op {
        Writer::Tell(w, r) => {
            log.borrow_mut().push(w);
            r.give(())
        }
    }, |a| a)
}

fn failed<Q, A: std::fmt::Debug + 'static>(s: Step<Q, A>) -> String {
    match s {
        Step::Failed(e) => e.message(),
        other => panic!("not failed: {other:?}"),
    }
}

fn says<R: Member<Writer<String>, Ix> + 'static, Ix: 'static>(s: &'static str) -> impl Fn() -> Eff<R, ()> + 'static {
    move || tell::<String, R, Ix>(s.to_string())
}

#[test]
fn a_translations_failure_raised_where_the_operation_was_a_recover_in_the_body_sees_it() {
    type R = row![Writer<String>];
    let failing = Handler::translating(|_: Writer<String>| defer::<Nil, Answer>(|| panic!("boom")));
    let body: Eff<R, i64> = tell::<String, R, _>("x".into()).map(|_| 1).recover(|e| pure(if e.message() == "boom" { 7 } else { 0 }));
    assert_eq!(body.handle(failing).run(), 7);
}

#[test]
fn ensuring_the_finalizer_after_the_program_its_value_kept() {
    type R = row![Writer<String>];
    let p = tell::<String, R, _>("body".into()).map(|_| 1).ensure(says::<R, _>("fin"));
    assert_eq!(p.handle(collect()).run(), (vec!["body".to_string(), "fin".to_string()], 1));
}

#[test]
fn ensuring_and_a_throw_the_finalizer_then_the_error_out() {
    type R = row![Writer<String>];
    let p = || tell::<String, R, _>("body".into()).map(|_| -> i64 { panic!("boom") }).ensure(says::<R, _>("fin"));
    assert_eq!(failed(p().handle(collect()).step()), "boom");
    let log: Log = Rc::default();
    assert_eq!(failed(p().handle(cell(log.clone())).step()), "boom");
    assert_eq!(*log.borrow(), vec!["body", "fin"]);
}

#[test]
fn a_panic_in_an_answering_handlers_value_the_finalizer_runs_the_panic_reaches_the_top_as_itself() {
    type R = row![Writer<String>];
    let log: Log = Rc::default();
    let l = log.clone();
    let parking = Handler::answering(move |op: Writer<String>| match op {
        Writer::Tell(w, r) => {
            if w == "park" {
                std::panic::panic_any(String::from("cancelled"));
            }
            l.borrow_mut().push(w);
            r.give(())
        }
    }, |a| a);
    let p = tell::<String, R, _>("park".into()).map(|_| 1).ensure(says::<R, _>("fin"));
    let thrown = catch_unwind(AssertUnwindSafe(|| p.handle(parking).run())).unwrap_err();
    assert_eq!(Failure::from_panic(thrown).message(), "cancelled");
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn ensuring_under_attempt_a_raise_runs_the_finalizer_then_attempt_answers_err() {
    type R = row![Throws<String>, Writer<String>];
    let p = tell::<String, R, _>("body".into())
        .flat_map(|_| raise::<String, (), R, _>("no".into()))
        .map(|_| 1)
        .ensure(move || tell::<String, R, _>("fin".into()));
    assert_eq!(p.handle(either()).handle(collect()).run(), (vec!["body".to_string(), "fin".to_string()], Err("no".to_string())));
}

#[test]
fn a_finalizer_under_chooses_k_resumed_twice_runs_twice() {
    type R = row![Choose, Writer<String>];
    let p = flip::<R, _>().map(|b| if b { 1 } else { 2 }).ensure(move || tell::<String, R, _>("fin".into()));
    assert_eq!(p.handle(all()).handle(collect()).run(), (vec!["fin".to_string(), "fin".to_string()], vec![1, 2]));
}

#[test]
fn a_guard_told_of_an_operation_passing_out_another_sent_in_its_place_a_count_at_each_resumption() {
    type R = row![Choose];
    let (told, resumed) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(0)));
    let (t, rs) = (told.clone(), resumed.clone());
    let g = Guard::new().passing(move |op| match op.op::<Choose>() {
        Some(Choose::Among(n, r)) => {
            t.borrow_mut().push(n);
            let rs = rs.clone();
            // the same effect: a choice of one alternative less
            (op.with(Choose::Among(n - 1, r)), Some(Rc::new(move || rs.set(rs.get() + 1)) as Rc<dyn Fn()>))
        }
        _ => (op, None),
    });
    let p = among::<i64, R, _>(vec![1, 2, 3]).guard(g);
    assert_eq!(p.handle(all()).run(), vec![1, 2]);
    assert_eq!((told.borrow().clone(), resumed.get()), (vec![3], 2));
}

#[test]
fn a_guards_passing_that_panics_raised_where_the_operation_is_inside_the_guard() {
    type R = row![Choose];
    let fin = Rc::new(Cell::new(0));
    let f = fin.clone();
    let g = Guard::new()
        .finalizer(move || { f.set(f.get() + 1); pure(()) })
        .passing(|_| panic!("boom"));
    assert_eq!(failed(flip::<R, _>().map(|_| 1).guard(g).handle(all()).step()), "boom");
    assert_eq!(fin.get(), 1);
}

#[test]
fn recover_catches_a_panic_not_a_raise() {
    type R = row![Throws<String>];
    let thrown = pure::<R, i64>(0).map(|_| -> i64 { panic!("boom") }).recover(|e| pure(if e.message() == "boom" { 7 } else { -1 }));
    assert_eq!(thrown.handle(either()).run(), Ok(7));
    let raised = raise::<String, (), R, _>("no".into()).map(|_| 0).recover(|_| pure(-1));
    assert_eq!(raised.handle(either()).run(), Err("no".to_string()));
}

#[test]
fn guards_nested_unwound_innermost_first_a_finalizers_panic_replaces_the_error() {
    type R = row![Writer<String>];
    let log: Log = Rc::default();
    let p = pure::<R, i64>(0)
        .map(|_| -> i64 { panic!("boom") })
        .ensure(says::<R, _>("inner"))
        .ensure(|| tell::<String, R, _>("outer".into()).map(|_| -> () { panic!("second") }));
    assert_eq!(failed(p.handle(cell(log.clone())).step()), "second");
    assert_eq!(*log.borrow(), vec!["inner", "outer"]);
}

#[test]
fn an_abort_whose_finalizers_operation_takes_the_aborting_level_into_a_k_the_abort_on_each_path() {
    type R = row![Throws<String>, Choose];
    let p = raise::<String, (), R, _>("no".into()).map(|_| 1).ensure(|| among::<i64, R, _>(vec![1, 2]).map(|_| ()));
    assert_eq!(p.handle(either()).handle(all()).run(), vec![Err("no".to_string()), Err("no".to_string())]);
}

#[test]
fn an_abort_past_guards_and_states_each_finalizer_the_states_dropped() {
    type R = row![Throws<String>, State<i64>, Writer<String>];
    let p = put::<i64, R, _>(5)
        .flat_map(|_| raise::<String, (), R, _>("no".into()))
        .map(|_| 1)
        .ensure(|| get::<i64, R, _>().flat_map(|s| tell::<String, R, _>(format!("fin {s}"))));
    assert_eq!(p.handle(either()).handle(state(0)).handle(collect()).run(), (vec!["fin 5".to_string()], (5, Err("no".to_string()))));
}

// TestGuardStep: a step failed, cancelled — the error raised inside the rest's levels, their guards asked

type S = row![Reader<i64>, Writer<String>];

fn first<A: 'static>(s: Step<row![Reader<i64>], A>) -> Yield<row![Reader<i64>], A> {
    match s {
        Step::Yield(y) => y,
        other => panic!("not a step: {}", matches!(other, Step::Done(_))),
    }
}

fn give<A: 'static>(y: &Yield<row![Reader<i64>], A>, n: i64) -> Step<row![Reader<i64>], A> {
    match y.op::<Reader<i64>>() {
        Some(Reader::Ask(r)) => y.resume(r.give(n)),
        None => panic!("not an ask"),
    }
}

#[test]
fn resume_fail_the_finalizers_in_the_rest_run_then_failed() {
    let log: Log = Rc::default();
    let p = ask::<i64, S, _>().map(|n| n + 1).ensure(says::<S, _>("fin")).handle(cell(log.clone()));
    assert_eq!(failed(first(p.step()).fail(Failure::new(String::from("boom")))), "boom");
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn resume_fail_a_recover_in_the_rest_given_the_error() {
    let log: Log = Rc::default();
    let p = ask::<i64, S, _>().map(|n| n + 1).recover(|e| pure(if e.message() == "boom" { 7 } else { -1 })).handle(cell(log));
    assert!(matches!(first(p.step()).fail(Failure::new(String::from("boom"))), Step::Done(7)));
}

#[test]
fn cancel_the_finalizers_run_no_recover_asked_cancelled() {
    let log: Log = Rc::default();
    let p = ask::<i64, S, _>().map(|n| n + 1).recover(|_| pure(-1)).ensure(says::<S, _>("fin")).handle(cell(log.clone()));
    assert!(matches!(first(p.step()).cancel(), Step::Cancelled));
    assert_eq!(*log.borrow(), vec!["fin"]);
}

#[test]
fn cancel_under_uncancelable_the_step_given_back_the_cancel_fired_when_the_mask_is_left() {
    let log: Log = Rc::default();
    let p = ask::<i64, S, _>()
        .flat_map(|n| tell::<String, S, _>(format!("in {n}")).map(move |_| n))
        .uncancelable()
        .flat_map(|n| tell::<String, S, _>("after".into()).map(move |_| n))
        .ensure(says::<S, _>("fin"))
        .handle(cell(log.clone()));
    let y = first(p.step());
    let again = match y.clone().cancel() {
        Step::Yield(y) => y,
        _ => panic!("the same step given back"),
    };
    assert_eq!(again.describe(), y.describe());
    assert!(matches!(give(&again, 5), Step::Cancelled));
    assert_eq!(*log.borrow(), vec!["in 5", "fin"]);
}

#[test]
fn cancel_under_uncancelable_then_the_region_fails_cancelled_not_recovered() {
    let log: Log = Rc::default();
    let p = ask::<i64, S, _>()
        .flat_map(|_| pure::<S, i64>(1).map(|_| -> i64 { panic!("boom") }))
        .uncancelable()
        .recover(|_| pure(-1))
        .handle(cell(log));
    let again = match first(p.step()).cancel() {
        Step::Yield(y) => y,
        _ => panic!("the same step given back"),
    };
    assert!(matches!(give(&again, 5), Step::Cancelled));
}
