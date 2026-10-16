//! okay-core's TestStep: a program stepped — its operations of the row outside given back one by one, a driver
//! answering them

use okay::effects::*;
use okay::*;

type R = row![Reader<i64>, Writer<String>];

/// each `Ask` answered `n`, each `Tell` kept: the driver a loop
fn drive<A: 'static, Q>(mut s: Step<Q, A>, n: i64) -> (Vec<String>, A) {
    let mut log = Vec::new();
    loop {
        s = match s {
            Step::Done(a) => return (log, a),
            Step::Failed(e) => panic!("{}", e.message()),
            Step::Cancelled => panic!("cancelled"),
            Step::Yield(y) => {
                if let Some(Reader::Ask(r)) = y.op::<Reader<i64>>() {
                    y.resume(r.give(n))
                } else if let Some(Writer::Tell(m, r)) = y.op::<Writer<String>>() {
                    log.push(m);
                    y.resume(r.give(()))
                } else {
                    panic!("an operation of another effect: {}", y.effect())
                }
            }
        }
    }
}

/// "each operation given back, the rest resumed with its value"
#[test]
fn each_operation_given_back_the_rest_resumed_with_its_value() {
    let p: Eff<R, i64> = eff! { data;
        a <- ask::<i64, _, _>();
        _ <- tell::<String, _, _>(format!("a {a}"));
        b <- ask::<i64, _, _>();
        yield a + b
    };
    assert_eq!(drive(p.step(), 5), (vec!["a 5".to_string()], 10));
}

/// "handlers in the program: theirs answered in it, the others given back past them"
#[test]
fn handlers_in_the_program_theirs_answered_in_it_the_others_given_back_past_them() {
    type Q = row![Choose, State<i64>, Reader<i64>, Writer<String>];
    let p: Eff<Q, i64> = eff! { data;
        b <- flip();
        s <- get::<i64, _, _>();
        n <- ask::<i64, _, _>();
        _ <- tell::<String, _, _>(format!("{b} {s}"));
        _ <- put::<i64, _, _>(s + n);
        yield s
    };
    let handled = p.handle(all()).handle(state(1));
    assert_eq!(drive(handled.step(), 10), (vec!["true 1".to_string(), "false 11".to_string()], (21, vec![1, 11])));
}

/// "a step resumed twice: each its own rest"
#[test]
fn a_step_resumed_twice_each_its_own_rest() {
    let p: Eff<R, i64> = ask::<i64, R, _>().map(|n| n * 2);
    let Step::Yield(y) = p.step() else { panic!("not a step") };
    let Some(Reader::Ask(r)) = y.op::<Reader<i64>>() else { panic!("not an Ask") };
    assert_eq!(drive(y.resume(r.give(1)), 0).1, 2);
    assert_eq!(drive(y.resume(r.give(7)), 0).1, 14);
}

/// "stack safe: a loop of operations stepped"
#[test]
fn stack_safe_a_loop_of_operations_stepped() {
    fn lp(i: i64, acc: i64) -> Eff<R, i64> {
        if i == 0 { pure(acc) } else { ask::<i64, R, _>().flat_map(move |n| lp(i - 1, acc + n)) }
    }
    assert_eq!(drive(lp(100_000, 0).step(), 1).1, 100_000);
}

/// a host CANCELS: the rest dropped, never resumed
#[test]
fn a_host_cancels() {
    let p: Eff<R, i64> = ask::<i64, R, _>();
    let Step::Yield(y) = p.step() else { panic!() };
    assert!(matches!(y.cancel(), Step::Cancelled));
}
