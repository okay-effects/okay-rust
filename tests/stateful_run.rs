//! okay-core's TestStatefulRun: a run whose top is one stateful handler — the values those of the machine's (the
//! operations by key of okay's root have no counterpart here: an effect's place is its type in the row)

use std::panic::{catch_unwind, AssertUnwindSafe};

use okay::effects::*;
use okay::*;

type S = row![State<i64>];

fn get_s() -> Eff<S, i64> {
    get::<i64, S, _>()
}
fn put_s(n: i64) -> Eff<S, ()> {
    put::<i64, S, _>(n)
}

#[test]
fn binds_nested_to_the_left_and_to_the_right_100_000_operations_each_constant_stack() {
    let mut left: Eff<S, i64> = pure(0);
    for _ in 0..100_000 {
        left = left.flat_map(|_| get_s().flat_map(|s| put_s(s + 1)).map(|_| 0));
    }
    assert_eq!(left.handle(state(0)).run(), (100_000, 0));
    fn right(i: i64) -> Eff<S, i64> {
        if i == 0 { get_s() } else { get_s().flat_map(|s| put_s(s + 1)).flat_map(move |_| right(i - 1)) }
    }
    assert_eq!(right(100_000).handle(state(5)).run(), (100_005, 100_005));
}

#[test]
fn a_handler_inside_the_state_as_it_was() {
    type SL = row![Writer<String>, State<i64>];
    let p: Eff<S, i64> = get_s().flat_map(|s| put_s(s + 10)).flat_map(|_| {
        tell::<String, SL, _>("in".into())
            .flat_map(|_| get::<i64, SL, _>())
            .handle(collect())
            .flat_map(|(l, s): (Vec<String>, i64)| put_s(s * 2).map(move |_| l.len() as i64))
    });
    assert_eq!(p.handle(state(1)).run(), (22, 1));
}

#[test]
fn a_panic_in_a_user_function_out_of_the_run() {
    let p = || get_s().map(|s| if s > 0 { panic!("too big: {s}") } else { s });
    assert!(catch_unwind(AssertUnwindSafe(|| p().handle(state(1)).run())).is_err());
    assert_eq!(p().handle(state(0)).run(), (0, 0));
}

#[test]
fn a_handlers_ret_of_the_last_state() {
    let counter = Handler::stateful(4i64, |s: &mut i64, op| match op {
        State::Get(r) => r.give(*s),
        State::Put(n, r) => { *s = n; r.give(()) }
    }, |s, _: ()| s * 100);
    assert_eq!(get_s().flat_map(|s| put_s(s + 1)).handle(counter).run(), 500);
}

#[test]
fn a_choice_inside_the_states_program_every_path_the_state_threaded() {
    type CS = row![Choose, State<i64>];
    let p: Eff<CS, i64> = flip::<CS, _>().flat_map(|b| {
        get::<i64, CS, _>().flat_map(move |s| put::<i64, CS, _>(s + if b { 1 } else { 10 }).map(move |_| s))
    });
    assert_eq!(p.handle(all()).handle(state(0)).run(), (11, vec![0, 1]));
}
