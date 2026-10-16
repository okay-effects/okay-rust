//! ops/second of okay-effects, std only: `cargo bench -p okay-effects`.
//! Each lane runs `REPS` times; the best and the median are printed.

use std::hint::black_box;
use std::time::Instant;

use okay_effects::{answering, drive, handle, op, only, run, start, with, Answerer, Reply, Routes};

op!(pub struct Ask { pub n: i64 } -> i64);
op!(pub struct Ask2 { pub n: i64 } -> i64);
op!(pub struct Ask3 { pub n: i64 } -> i64);

const N: i64 = 1_000_000;
const REPS: usize = 7;

fn answering_lane() -> i64 {
    run(|r| async move {
        answering(&r, |a: Ask| a.n, |ask| async move {
            let mut s = 0;
            for i in 0..N {
                s += ask.perform(Ask { n: black_box(i) }).await;
            }
            s
        })
        .await
    })
}

/// the body performs Ask3; each level answers by performing on the next out; Ask answers
fn chain3_lane() -> i64 {
    run(|r| async move {
        let r = &r;
        answering(r, |a: Ask| a.n, |ask| {
            handle(r, move |a: Ask2| {
                let ask = ask.clone();
                async move { Reply::Resume(ask.perform(Ask { n: a.n }).await) }
            }, |ask2| {
                handle(r, move |a: Ask3| {
                    let ask2 = ask2.clone();
                    async move { Reply::Resume(ask2.perform(Ask2 { n: a.n }).await) }
                }, |ask3| async move {
                    let mut s = 0;
                    for i in 0..N {
                        s += ask3.perform(Ask3 { n: black_box(i) }).await;
                    }
                    s
                })
            })
        })
        .await
    })
}

/// a handed-out program, every operation as data through two middleware and Routes
fn drive_lane() -> i64 {
    let c = start(|h| async move {
        let ask = h.outside::<Ask>();
        let mut s = 0;
        for i in 0..N {
            s += ask.perform(Ask { n: black_box(i) }).await;
        }
        s
    });
    let mut count = 0u64;
    let answer: Answerer = with(
        Routes::new().on(|a: &Ask| Ok(a.n)).answerer(),
        vec![
            Box::new(|a, next| {
                count += 1;
                next(a)
            }),
            only(|_: &Ask, a, next| next(a)),
        ],
    );
    let v = drive(c, answer).unwrap();
    black_box(count);
    v
}

fn lane(name: &str, f: fn() -> i64) {
    let expected = N * (N - 1) / 2;
    let mut secs: Vec<f64> = (0..REPS)
        .map(|_| {
            let t = Instant::now();
            assert_eq!(black_box(f()), expected);
            t.elapsed().as_secs_f64()
        })
        .collect();
    secs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (best, median) = (secs[0], secs[REPS / 2]);
    println!(
        "{name:<12} {:>7.1} M ops/s best, {:>7.1} M ops/s median, {:>6.1} ns/op median",
        N as f64 / best / 1e6,
        N as f64 / median / 1e6,
        median * 1e9 / N as f64
    );
}

fn main() {
    lane("answering", answering_lane);
    lane("chain3", chain3_lane);
    lane("drive", drive_lane);
}
