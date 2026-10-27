//! okay-core's TestShift and TestShiftThin: `Shifts<S, T>`, control as an effect — the shifts up to a `reset`, a deep
//! handler

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay_machine::effects::*;
use okay_machine::*;

type R<S> = row![Shifts<S, Nil>];
type K<A, T, S> = Rc<dyn Fn(A) -> Eff<T, S>>;

#[test]
fn a_body_dropping_k_aborts_the_reset_the_rest_never_runs() {
    let ran = Rc::new(Cell::new(false));
    let r2 = ran.clone();
    let p = shift::<i64, Nil, i64, R<i64>, _>(|_| pure(-1)).map(move |x| { r2.set(true); x });
    assert_eq!(p.handle(reset()).run(), -1);
    assert!(!ran.get());
}

#[test]
fn multi_shot_k_resumed_twice_the_answers_joined() {
    type S = Vec<i64>;
    let p = shift::<S, Nil, i64, R<S>, _>(|k| {
        let k2 = k.clone();
        k(1).flat_map(move |a| k2(2).map(move |b| [a.clone(), b].concat())).widen()
    })
    .map(|x| vec![x, x * 10]);
    assert_eq!(p.handle(reset()).run(), vec![1, 10, 2, 20]);
}

#[test]
fn a_shifts_body_runs_under_the_reset_a_shift_in_it_captures_up_to_the_same_reset() {
    let p = shift::<i64, Nil, i64, R<i64>, _>(|k: K<i64, Nil, i64>| {
        shift::<i64, Nil, i64, R<i64>, _>(|k2: K<i64, Nil, i64>| k2(1).map(|x| x * 100).widen())
            .flat_map(move |x| k(x).widen())
    })
    .map(|x| x + 1);
    assert_eq!(p.handle(reset()).run(), 200);
}

#[test]
fn a_shift_captures_up_to_the_nearest_reset_only() {
    let p = shift0::<i64, Nil, i64, R<i64>, _>(|k| k(1).map(|x| x * 100)).handle(reset()).flat_map(|a| {
        shift0::<i64, Nil, i64, R<i64>, _>(|k| k(2).map(|x| x + 7)).map(move |x| x + a).handle(reset())
    });
    assert_eq!(p.run(), 109);
}

#[test]
fn k_applied_runs_nothing_of_the_rest_the_rest_runs_when_stepped() {
    let ran = Rc::new(Cell::new(false));
    let (r1, r2) = (ran.clone(), ran.clone());
    let p = shift0::<i64, Nil, i64, R<i64>, _>(move |k| {
        let _ = k(1);
        pure(if r1.get() { 1 } else { 0 })
    })
    .flat_map(move |x| { r2.set(true); pure(x) });
    assert_eq!(p.handle(reset()).run(), 0);
}

#[test]
fn many_resets_and_shifts_in_constant_stack() {
    fn many(n: i64, acc: i64) -> Eff<Nil, i64> {
        if n == 0 {
            pure(acc)
        } else {
            shift0::<i64, Nil, i64, R<i64>, _>(|k| k(1)).map(|x| x + 1).handle(reset()).flat_map(move |x| defer(move || many(n - 1, acc + x)))
        }
    }
    assert_eq!(many(100_000, 0).run(), 200_000);
}

#[test]
fn a_shift_under_a_handler_the_reset_outside_k_holds_the_handler() {
    type T = row![Reader<i64>];
    // the inner reader of another type: an effect is once in a row here (its place found by its type)
    type Q = row![Reader<i32>, Shifts<i64, T>, Reader<i64>];
    // reset(reader(3) { x <- shift0(k => k(1) + ask + 100 ...); ask + x }): the ask in the body the outer reader's, 7
    let inner = shift0::<i64, T, i64, Q, _>(|k| {
        let k2 = k.clone();
        k(1).flat_map(move |a| { let k2 = k2.clone(); ask::<i64, T, _>().flat_map(move |e| k2(a + e)) }).map(|x| x + 100)
    })
    .flat_map(|x| ask::<i32, Q, _>().map(move |n| n as i64 + x));
    let p = inner.handle(reader(3i32)).handle(reset()).handle(reader(7i64));
    // k(y) = 3 + y under the inner reader: k(1) = 4, ask (outer) 7, k(11) = 14, + 100 = 114
    assert_eq!(p.run(), 114);
}

#[derive(Clone)]
enum Kept {
    Value(i64),
    Rest(K<i64, Nil, Kept>),
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kept::Value(v) => write!(f, "Value({v})"),
            Kept::Rest(_) => f.write_str("Rest(_)"),
        }
    }
}

#[test]
fn shift0_k_itself_the_answer_resumed_after_the_reset_has_answered() {
    let p = shift0::<Kept, Nil, i64, R<Kept>, _>(|k| pure(Kept::Rest(k))).map(|x| Kept::Value(x * 2));
    match p.handle(reset()).run() {
        Kept::Rest(k) => assert!(matches!(k(21).run(), Kept::Value(42))),
        v => panic!("no k: {v:?}"),
    }
}

// TestShiftThin: a shift's k in its body — its value, an operation of the row outside found past it, a shift inside
// it, a panic in it

type T = row![Reader<i64>];
type SR = row![Shifts<i64, T>, Reader<i64>];

fn ask_sr() -> Eff<SR, i64> {
    ask::<i64, SR, _>()
}

fn twice(k: K<i64, T, i64>, a: i64, b: i64) -> Eff<T, i64> {
    let k2 = k.clone();
    k(a).flat_map(move |x| k2(b).map(move |y| x + y))
}

#[test]
fn thin_k_twice_in_the_body_each_its_answer_summed() {
    let p = shift0::<i64, T, i64, SR, _>(|k| twice(k, 1, 2)).map(|x| x * 10);
    assert_eq!(p.handle(reset()).handle(reader(5i64)).run(), 30);
}

#[test]
fn thin_an_operation_of_the_row_outside_inside_k_found_past_the_level() {
    let p = shift0::<i64, T, i64, SR, _>(|k| twice(k, 1, 2)).flat_map(|x| ask_sr().map(move |n| x * n));
    assert_eq!(p.handle(reset()).handle(reader(5i64)).run(), 15);
}

#[test]
fn thin_a_shift_inside_k_the_level_itself_taken_into_a_k_put_back() {
    let p = shift0::<i64, T, i64, SR, _>(|k| twice(k, 1, 10))
        .flat_map(|x| shift0::<i64, T, i64, SR, _>(move |k2| twice(k2, x, x + 1)));
    assert_eq!(p.handle(reset()).handle(reader(5i64)).run(), 24);
}

#[test]
fn thin_a_panic_inside_k_out_of_the_run() {
    let p = shift0::<i64, T, i64, SR, _>(|k| twice(k, 1, 2)).map(|x| if x == 2 { panic!("two") } else { x });
    assert!(catch_unwind(AssertUnwindSafe(|| p.handle(reset()).handle(reader(5i64)).run())).is_err());
}

#[test]
fn thin_k_resumed_after_its_bodys_other_operations() {
    let p = shift0::<i64, T, i64, SR, _>(|k| ask::<i64, T, _>().flat_map(move |n| twice(k.clone(), n, n + 1)))
        .flat_map(|x| ask_sr().map(move |n| n + x));
    assert_eq!(p.handle(reset()).handle(reader(5i64)).run(), (5 + 5) + (5 + 6));
}
