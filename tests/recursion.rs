//! RECURSION IN CONSTANT STACK: functions calling each other in tail position (`call!`), a loop (`tail_rec`) — a
//! million steps, with and without an effect at each, on a thread's default 2 MiB stack (Rust has no guaranteed tail
//! calls: plain recursion this deep overflows it)

use okay::effects::*;
use okay::*;

const MILLION: i64 = 1_000_000;

// MUTUAL RECURSION BY call!

fn is_even(i: i64) -> Eff<Nil, bool> {
    if i == 0 { pure(true) } else { call!(is_odd(i - 1)) }
}
fn is_odd(i: i64) -> Eff<Nil, bool> {
    if i == 0 { pure(false) } else { call!(is_even(i - 1)) }
}

/// "mutual tail recursion by call!: a million steps, constant stack"
#[test]
fn mutual_tail_recursion_by_call_a_million_steps() {
    assert!(is_even(MILLION).run());
    assert!(!is_even(MILLION + 1).run());
}

// THE SAME, AN EFFECT AT EACH STEP: a state counting the steps

type Counted = row![State<i64>];
fn even_counted(i: i64) -> Eff<Counted, bool> {
    if i == 0 { pure(true) } else { eff! { data; modify::<i64, _, _>(|s| s + 1); call!(odd_counted(i - 1)) } }
}
fn odd_counted(i: i64) -> Eff<Counted, bool> {
    if i == 0 { pure(false) } else { eff! { data; modify::<i64, _, _>(|s| s + 1); call!(even_counted(i - 1)) } }
}

/// "mutual tail recursion with a state at each step: a million operations"
#[test]
fn mutual_tail_recursion_with_a_state_at_each_step() {
    assert_eq!(even_counted(MILLION).handle(state(0)).run(), (MILLION, true));
}

// MUTUAL RECURSION AS A LOOP: the state names the function

#[derive(Clone)]
enum Call {
    Even(i64),
    Odd(i64),
}

/// "mutual recursion by tail_rec: the state names the function, a reader asked at each step"
#[test]
fn mutual_recursion_by_tail_rec_a_reader_at_each_step() {
    type R = row![Reader<i64>];
    let p: Eff<R, bool> = tail_rec(Call::Even(MILLION), |c| {
        ask::<i64, R, _>().map(move |step| match c {
            Call::Even(0) => Loop::Done(true),
            Call::Odd(0) => Loop::Done(false),
            Call::Even(i) => Loop::Next(Call::Odd(i - step)),
            Call::Odd(i) => Loop::Next(Call::Even(i - step)),
        })
    });
    assert!(p.handle(reader(1)).run());
}

/// "a loop under a choice: each path its own loop, the rest resumed into it"
#[test]
fn a_loop_under_a_choice_each_path_its_own() {
    type R = row![Choose];
    let p: Eff<R, i64> = among(vec![10i64, 1000]).flat_map(|n| {
        tail_rec((0i64, 0i64), move |(i, acc)| {
            pure(if i == n { Loop::Done(acc) } else { Loop::Next((i + 1, acc + i)) })
        })
    });
    assert_eq!(p.handle(all()).run(), vec![45, 499_500]);
}
