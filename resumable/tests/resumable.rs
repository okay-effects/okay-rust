//! Multi-shot continuations on stable Rust: `#[resumable(Ops)]` makes a function an explicit rest whose
//! rest is a Clone value — every path of a choice, a generator, a loop stopping at each guess, nested ifs
use okay::resumable::*;
use okay::Answer;
use okay_resumable::resumable;

okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
okay::effect! { pub enum Yield: Plain { Emit(v: i64) -> () = emit; } }
okay::effect! { pub enum Guess: Plain { Try(limit: i64) -> i64 = guess; } }
okay::ops! { pub enum ChooseOps { Choose } }
okay::ops! { pub enum GenOps { Yield } }
okay::ops! { pub enum SearchOps { Guess } }

#[resumable(ChooseOps)]
fn pick(base: usize) -> usize {
    let a: usize = ask!(Choose::Among(3));
    let b: usize = ask!(Choose::Among(2));
    base + a * 10 + b
}

fn all<K: Rest>(mut k: K, answer: Answer, out: &mut Vec<K::Out>)
where
    K::Op: OpOf<Choose>,
{
    match k.resume(answer) {
        Step::Done(a) => out.push(a),
        Step::Perform(op) => match op.op() {
            Ok(Choose::Among(n, r)) => {
                for i in 0..n {
                    all(k.clone(), r.give(i), out)
                }
            }
            Err(_) => unreachable!(),
        },
    }
}

#[test]
fn every_path_of_a_choice() {
    let mut out = vec![];
    all(pick(100), unit(), &mut out);
    assert_eq!(out, vec![100, 101, 110, 111, 120, 121]);
}

#[resumable(GenOps)]
fn squares(n: i64) {
    let mut i: i64 = 0;
    while i < n {
        if i % 2 == 0 {
            let _: () = ask!(Yield::Emit(i * i));
        }
        i += 1;
    }
}

#[resumable(SearchOps)]
fn search(target: i64) -> i64 {
    let mut tries: i64 = 0;
    loop {
        let g: i64 = ask!(Guess::Try(10));
        tries += 1;
        if g < 0 {
            continue;
        }
        if g == target {
            return tries;
        }
        if tries >= 5 {
            break;
        }
    }
    -1
}

#[test]
fn a_generator_pulled_and_a_loop_stopping_at_each_guess() {
    let mut got = vec![];
    let mut s = squares(7);
    let mut a = unit();
    while let Step::Perform(GenOps::Yield(Yield::Emit(v, r))) = s.resume(a) {
        got.push(v);
        a = r.give(());
    }
    assert_eq!(got, vec![0, 4, 16, 36]);

    let feed = [4, -1, 9, 7, 2];
    let mut at = 0;
    let mut t = search(7);
    let mut a = unit();
    let found = loop {
        match t.resume(a) {
            Step::Done(v) => break v,
            Step::Perform(SearchOps::Guess(Guess::Try(_, r))) => {
                a = r.give(feed[at]);
                at += 1;
            }
        }
    };
    assert_eq!(found, 4);
    let mut u = search(100);
    let mut a = unit();
    let gave_up = loop {
        match u.resume(a) {
            Step::Done(v) => break v,
            Step::Perform(SearchOps::Guess(Guess::Try(_, r))) => a = r.give(1),
        }
    };
    assert_eq!(gave_up, -1);
}

#[test]
fn a_rest_saved_and_resumed_twice_from_the_same_point() {
    // stopped at the second choice: the rest kept, resumed with 0 and with 1 — the first choice as it was
    let mut k = pick(0);
    let Step::Perform(ChooseOps::Choose(Choose::Among(_, r))) = k.resume(unit()) else {
        panic!()
    };
    let Step::Perform(ChooseOps::Choose(Choose::Among(_, r2))) = k.resume(r.give(2)) else {
        panic!()
    };
    let a = k.clone().resume(r2.give(0));
    let b = k.resume(r2.give(1));
    assert!(matches!((a, b), (Step::Done(20), Step::Done(21))));
}

#[resumable(ChooseOps)]
fn arms_and_loops(n: usize) -> usize {
    let mut total: usize = 0;
    let mut i: usize = 0;
    while i < n {
        i += 1;
        let c: usize = ask!(Choose::Among(3));
        match c {
            0 => {
                continue;
            }
            1 => {
                let d: usize = ask!(Choose::Among(2));
                total += 10 * (d + 1);
            }
            _ => {
                if total > 0 {
                    break;
                }
                total += 1;
            }
        }
    }
    total
}

#[test]
fn match_arms_continue_and_break_after_a_stop() {
    let mut got = vec![];
    all(arms_and_loops(2), unit(), &mut got);
    // depth first, each choice from 0: the first round's 0 (continue), 1 then d, 2; in each, the second round's
    assert_eq!(
        got,
        [0, 10, 20, 1, 10, 20, 30, 10, 20, 30, 40, 20, 1, 11, 21, 1]
    );
}
