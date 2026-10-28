//! Multi-shot continuations as coroutines (nightly, feature `coroutines`): the rest is the coroutine itself, cloned
//! to resume it again — every path of a choice, a generator, a loop stopping at each guess
#![cfg_attr(feature = "coroutines", feature(coroutines, coroutine_clone, stmt_expr_attributes))]
#![cfg(feature = "coroutines")]

use okay::ask;
use okay::resumable::*;

okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
okay::effect! { pub enum Yield: Plain { Emit(v: i64) -> () = emit; } }
okay::effect! { pub enum Guess: Plain { Try(limit: i64) -> i64 = guess; } }
okay::ops! { pub enum ChooseOps { Choose } }
okay::ops! { pub enum GenOps { Yield } }
okay::ops! { pub enum SearchOps { Guess } }

// every path's value: the rest resumed once for each alternative
fn all<K: Rest<Out = A>, A>(step: Step<K::Op, K, A>, out: &mut Vec<A>)
where
    K::Op: OpOf<Choose>,
{
    match step {
        Step::Done(a) => out.push(a),
        Step::Perform(op, k) => match op.op() {
            Ok(Choose::Among(n, r)) => {
                for i in 0..n {
                    all(k.clone().resume(r.give(i)), out)
                }
            }
            Err(_) => unreachable!(),
        },
    }
}

#[test]
fn every_path_of_a_choice() {
    let base = 100usize;
    let pick = okay::coroutine!(move -> usize {
        let a: usize = ask!(Choose::Among(3));
        let b: usize = ask!(Choose::Among(2));
        base + a * 10 + b
    });
    let mut out = vec![];
    all(co::<ChooseOps, usize, _>(pick), &mut out);
    assert_eq!(out, vec![100, 101, 110, 111, 120, 121]);
}

#[test]
fn a_generator_pulled_and_a_loop_stopping_at_each_guess() {
    let squares = okay::coroutine!(move {
        for i in 0..7i64 {
            if i % 2 == 0 {
                let _: () = ask!(Yield::Emit(i * i));
            }
        }
    });
    let mut got = vec![];
    let mut s = co::<GenOps, (), _>(squares);
    while let Step::Perform(op, k) = s {
        let GenOps::Yield(Yield::Emit(v, r)) = op;
        got.push(v);
        s = k.resume(r.give(()));
    }
    assert_eq!(got, vec![0, 4, 16, 36]);

    let search = okay::coroutine!(move -> i64 {
        let mut tries = 0;
        loop {
            let g: i64 = ask!(Guess::Try(10));
            tries += 1;
            if g < 0 { continue; }
            if g == 7 { return tries; }
            if tries >= 5 { break; }
        }
        -1
    });
    let feed = [4, -1, 9, 7, 2];
    let mut at = 0;
    let mut t = co::<SearchOps, i64, _>(search);
    let found = loop {
        match t {
            Step::Done(v) => break v,
            Step::Perform(SearchOps::Guess(Guess::Try(_, r)), k) => {
                t = k.resume(r.give(feed[at]));
                at += 1;
            }
        }
    };
    assert_eq!(found, 4);
}
