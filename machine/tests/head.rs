//! okay-core's TestHead on `step`: each operation paused, the rest the machine's own

use okay_machine::effects::*;
use okay_machine::*;

type R = row![Writer<i64>];

fn counting(n: i64) -> Eff<R, ()> {
    if n == 0 { pure(()) } else { tell::<i64, R, _>(n).flat_map(move |_| counting(n - 1)) }
}

fn drain(mut s: Step<R, ()>) -> Vec<i64> {
    let mut acc = Vec::new();
    loop {
        s = match s {
            Step::Done(()) => return acc,
            Step::Yield(y) => match y.op::<Writer<i64>>() {
                Some(Writer::Tell(w, r)) => {
                    acc.push(w);
                    y.resume(r.give(()))
                }
                None => panic!("not a tell"),
            },
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn each_operation_in_order_the_rest_resumed() {
    assert_eq!(drain(counting(5).step()), vec![5, 4, 3, 2, 1]);
}

#[test]
fn many_steps_in_constant_stack() {
    assert_eq!(drain(counting(100_000).step()).len(), 100_000);
}

#[test]
fn a_paused_step_resumed_twice_each_a_run_of_its_own() {
    let Step::Yield(y) = counting(2).step() else { panic!("not paused") };
    let Some(Writer::Tell(_, r)) = y.op::<Writer<i64>>() else { panic!() };
    assert_eq!(drain(y.resume(r.give(()))), vec![1]);
    assert_eq!(drain(y.resume(r.give(()))), vec![1]);
}

#[test]
fn failed_where_it_was_a_recover_in_the_rest_given_the_error() {
    let p = tell::<i64, R, _>(1).map(|_| 0).recover(|e| pure(e.message().len() as i64));
    let Step::Yield(y) = p.step() else { panic!("not paused") };
    assert!(matches!(y.fail(Failure::new(String::from("boom"))), Step::Done(4)));
}
