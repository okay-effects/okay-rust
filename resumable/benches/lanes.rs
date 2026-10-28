//! Multi-shot continuations, measured as okay-goose's lanes (n = 1000, µs per run, warm-up a second then five rounds
//! of a second): `#[resumable]` (stable, `RS:`) and coroutines (nightly, feature `coroutines`, `CO:`)
#![cfg_attr(
    feature = "coroutines",
    feature(coroutines, coroutine_clone, stmt_expr_attributes)
)]

use std::hint::black_box;
use std::time::{Duration, Instant};

use okay::resumable::*;
use okay_resumable::resumable;

okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
okay::effect! { pub enum Tick: Plain { T(v: i64) -> i64 = tick; } }
okay::effect! { pub enum Yield: Plain { Emit(v: i64) -> () = emit; } }
okay::ops! { pub enum ChooseOps { Choose } }
okay::ops! { pub enum TickOps { Tick } }
okay::ops! { pub enum GenOps { Yield } }

const N: i64 = 1000;

// THE PROGRAMS, stable

#[resumable(ChooseOps)]
fn pick(n: usize) -> usize {
    let i: usize = ask!(Choose::Among(n));
    i + 1
}

#[resumable(TickOps)]
fn ticks(n: i64) -> bool {
    let mut even: bool = true;
    let mut i: i64 = 0;
    while i < n {
        let v: i64 = ask!(Tick::T(i));
        black_box(v);
        even = !even;
        i += 1;
    }
    even
}

#[resumable(GenOps)]
fn numbers(n: i64) {
    let mut i: i64 = 0;
    while i < n {
        let _: () = ask!(Yield::Emit(i));
        i += 1;
    }
}

// THE HANDLERS: plain functions over the steps, the same for both ways; each answer and value through
// `black_box`, so the compiler cannot fold a whole run into a formula

fn count_all<K: Rest>(step: Step<K::Op, K, K::Out>, found: &mut i64)
where
    K::Op: OpOf<Choose>,
{
    match step {
        Step::Done(_) => *found += 1,
        Step::Perform(op, k) => match op.op() {
            Ok(Choose::Among(n, r)) => {
                for i in 0..n {
                    count_all(k.clone().resume(r.give(black_box(i))), found)
                }
            }
            Err(_) => unreachable!(),
        },
    }
}

fn answer<K: Rest>(mut step: Step<K::Op, K, K::Out>) -> K::Out
where
    K::Op: OpOf<Tick>,
{
    loop {
        match step {
            Step::Done(v) => return v,
            Step::Perform(op, k) => match op.op() {
                Ok(Tick::T(v, r)) => step = k.resume(r.give(black_box(v))),
                Err(_) => unreachable!(),
            },
        }
    }
}

fn sum<K: Rest>(mut step: Step<K::Op, K, K::Out>) -> i64
where
    K::Op: OpOf<Yield>,
{
    let mut total = 0;
    loop {
        match step {
            Step::Done(_) => return total,
            Step::Perform(op, k) => match op.op() {
                Ok(Yield::Emit(v, r)) => {
                    total += black_box(v);
                    step = k.resume(r.give(()))
                }
                Err(_) => unreachable!(),
            },
        }
    }
}

// THE PROGRAMS, coroutines

#[cfg(feature = "coroutines")]
mod co_lanes {
    use super::*;
    use okay::ask;

    pub fn choose() -> i64 {
        let n = black_box(N as usize);
        let p = okay::coroutine!(move -> usize {
            let i: usize = ask!(Choose::Among(n));
            i + 1
        });
        let mut found = 0;
        count_all(co::<ChooseOps, usize, _>(p), &mut found);
        found
    }
    pub fn general() -> i64 {
        let n = black_box(N);
        let p = okay::coroutine!(move -> bool {
            let mut even = true;
            for i in 0..n {
                let v: i64 = ask!(Tick::T(i));
                black_box(v);
                even = !even;
            }
            even
        });
        answer(co::<TickOps, bool, _>(p)) as i64
    }
    pub fn generate() -> i64 {
        let n = black_box(N);
        let p = okay::coroutine!(move {
            for i in 0..n {
                let _: () = ask!(Yield::Emit(i));
            }
        });
        sum(co::<GenOps, (), _>(p))
    }
}

fn lane(name: &str, f: &dyn Fn() -> i64) {
    let second = Duration::from_secs(1);
    let t = Instant::now();
    while t.elapsed() < second {
        black_box(f());
    }
    let mut rounds = vec![];
    for _ in 0..5 {
        let t = Instant::now();
        let mut n = 0u64;
        while t.elapsed() < second {
            black_box(f());
            n += 1;
        }
        rounds.push(t.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
    let mean = rounds.iter().sum::<f64>() / 5.0;
    let sd = (rounds.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / 4.0).sqrt();
    println!("{name:14} {mean:10.3} ± {sd:7.3} us/op");
}

fn main() {
    let only: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let run = |name: &str| only.is_empty() || only.iter().any(|o| o == name);
    // the answers, checked
    let mut found = 0;
    count_all(pick(N as usize), &mut found);
    assert_eq!(
        (found, answer(ticks(N)), sum(numbers(N))),
        (N, true, N * (N - 1) / 2)
    );
    if run("RS:choose") {
        lane("RS:choose", &|| {
            let mut f = 0;
            count_all(pick(black_box(N as usize)), &mut f);
            f
        });
    }
    if run("RS:general") {
        lane("RS:general", &|| answer(ticks(black_box(N))) as i64);
    }
    if run("RS:generate") {
        lane("RS:generate", &|| sum(numbers(black_box(N))));
    }
    #[cfg(feature = "coroutines")]
    {
        assert_eq!(
            (
                co_lanes::choose(),
                co_lanes::general(),
                co_lanes::generate()
            ),
            (N, 1, N * (N - 1) / 2)
        );
        if run("CO:choose") {
            lane("CO:choose", &co_lanes::choose);
        }
        if run("CO:general") {
            lane("CO:general", &co_lanes::general);
        }
        if run("CO:generate") {
            lane("CO:generate", &co_lanes::generate);
        }
    }
}
