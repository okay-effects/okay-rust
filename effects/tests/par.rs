use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use okay_effects::{all, answering, handle, op, par, race, run, start, Reply, Step};

op!(pub struct Ask { pub n: i64 } -> i64);
op!(pub struct Tick {} -> ());

/// A timer: a thread sleeps, then wakes the task; std only.
struct Sleep {
    until: Instant,
    started: bool,
}

fn sleep(ms: u64) -> Sleep {
    Sleep { until: Instant::now() + Duration::from_millis(ms), started: false }
}

impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Poll::Ready(());
        }
        if !self.started {
            self.started = true;
            let w = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(left);
                w.wake();
            });
        }
        Poll::Pending
    }
}

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

fn message(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

#[test]
fn all_waits_concurrently() {
    let t = Instant::now();
    let got = run(|r| async move { all(&r, [50u64, 50].map(|ms| async move { sleep(ms).await; ms })).await });
    assert_eq!(got, vec![50, 50]);
    assert!(t.elapsed() < Duration::from_millis(90), "{:?}", t.elapsed());
}

#[test]
fn all_values_in_order_whichever_ends_first() {
    let got = run(|r| async move { all(&r, [30u64, 0, 15].map(|ms| async move { sleep(ms).await; ms })).await });
    assert_eq!(got, vec![30, 0, 15]);
}

#[test]
fn branches_perform_through_a_parent_one_at_a_time() {
    let log = Rc::new(RefCell::new(vec![]));
    let l = log.clone();
    let got = run(|r| async move {
        let r = &r;
        answering(r, move |a: Ask| { l.borrow_mut().push(a.n); a.n * 10 }, |ask| async move {
            let ask = &ask;
            all(r, (0..3i64).map(|i| async move {
                let mut s = 0;
                for k in 0..3 {
                    s += ask.perform(Ask { n: i * 100 + k }).await;
                    sleep(5).await;
                }
                s
            }))
            .await
        })
        .await
    });
    assert_eq!(got, vec![30, 3030, 6030]);
    let mut seen = log.borrow().clone();
    assert_eq!(seen.len(), 9);
    assert_eq!(&seen[..3], &[0, 100, 200]); // each branch's first operation, in turn, before any sleep ends
    seen.sort();
    assert_eq!(seen, vec![0, 1, 2, 100, 101, 102, 200, 201, 202]);
}

#[test]
fn each_branch_with_its_own_handlers() {
    let got = run(|r| async move {
        let r = &r;
        answering(r, |a: Ask| a.n + 1000, |parent| async move {
            let parent = &parent;
            all(r, (1..=3i64).map(|i| answering(r, move |a: Ask| a.n * i, move |mine| async move {
                let x = mine.perform(Ask { n: 10 }).await;
                sleep(10).await; // the others run meanwhile, their handlers installed
                let y = mine.perform(Ask { n: 1 }).await;
                x + y + parent.perform(Ask { n: i }).await
            })))
            .await
        })
        .await
    });
    assert_eq!(got, vec![11 + 1001, 22 + 1002, 33 + 1003]);
}

#[test]
fn a_translating_parent_handler_answers_branches() {
    let ticks = Rc::new(Cell::new(0));
    let t = ticks.clone();
    let got = run(|r| async move {
        let r = &r;
        answering(r, move |_: Tick| t.set(t.get() + 1), |tick| handle(r, move |a: Ask| {
            let tick = tick.clone();
            async move {
                tick.perform(Tick {}).await;
                sleep(5).await; // the answer itself waits; the branches wait for it
                Reply::Resume(a.n)
            }
        }, |ask| async move {
            let ask = &ask;
            all(r, (0..4i64).map(|i| async move { ask.perform(Ask { n: i }).await + ask.perform(Ask { n: i }).await })).await
        }))
        .await
    });
    assert_eq!((got, ticks.get()), (vec![0, 2, 4, 6], 8));
}

#[test]
fn par_of_two_types() {
    let got = run(|r| async move {
        let r = &r;
        answering(r, |a: Ask| a.n, |ask| async move {
            par(r, async { sleep(10).await; ask.perform(Ask { n: 7 }).await }, async {
                "x".repeat(ask.perform(Ask { n: 3 }).await as usize)
            })
            .await
        })
        .await
    });
    assert_eq!(got, (7, "xxx".to_string()));
}

#[test]
fn race_first_wins_the_others_dropped() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let t = Instant::now();
    let got = run(|r| async move {
        let c = &c;
        race(&r, [10u64, 500].map(|ms| async move {
            let _guard = (ms == 500).then(|| Cleanup(c.clone()));
            sleep(ms).await;
            ms
        }))
        .await
    });
    assert_eq!(got, 10);
    assert!(cleaned.get(), "the loser's destructors ran");
    assert!(t.elapsed() < Duration::from_millis(300));
}

#[test]
fn a_failing_branch_fails_all_and_drops_the_others() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let p = catch_unwind(AssertUnwindSafe(|| {
        run(|r| async move {
            let c = &c;
            all(&r, [0, 1].map(|i| async move {
                let _guard = Cleanup(c.clone());
                if i == 1 {
                    sleep(10).await;
                    panic!("branch 1 failed");
                }
                sleep(500).await;
                c.set(false); // never: dropped before
                i
            }))
            .await
        })
    }));
    assert_eq!(message(p.unwrap_err()), "branch 1 failed");
    assert!(cleaned.get());
}

#[test]
fn a_parent_abort_drops_every_branch() {
    let dropped = Rc::new(Cell::new(0));
    let d = dropped.clone();
    let got = run(|r| async move {
        let r = &r;
        let d = &d;
        handle(r, |a: Ask| std::future::ready(if a.n == 2 { Reply::Abort(vec![-1]) } else { Reply::Resume(a.n) }), |ask| async move {
            let ask = &ask;
            all(r, (0..3i64).map(|i| async move {
                struct Count<'a>(&'a Cell<i32>);
                impl Drop for Count<'_> {
                    fn drop(&mut self) {
                        self.0.set(self.0.get() + 1);
                    }
                }
                let _c = Count(d);
                sleep(10 * i as u64).await;
                ask.perform(Ask { n: i }).await
            }))
            .await
        })
        .await
    });
    assert_eq!(got, vec![-1]);
    assert_eq!(dropped.get(), 3);
}

#[test]
fn nested_branches() {
    let got = run(|r| async move {
        let r = &r;
        answering(r, |a: Ask| a.n * 2, |ask| async move {
            let ask = &ask;
            all(r, (0..2i64).map(|i| async move {
                all(r, (0..2i64).map(|j| async move { sleep(5).await; ask.perform(Ask { n: i * 10 + j }).await })).await
            }))
            .await
        })
        .await
    });
    assert_eq!(got, vec![vec![0, 2], vec![20, 22]]);
}

#[test]
fn branches_of_a_handed_out_program_ask_the_host_in_turn() {
    let mut c = start(|h| async move {
        let ask = h.outside::<Ask>();
        let ask = &ask;
        all(&h.run(), (1..=3i64).map(|i| async move { ask.perform(Ask { n: i }).await })).await
    });
    let mut asked = vec![];
    let mut s = c.step();
    while let Step::Yielded(a) = s {
        let n = a.get::<Ask>().unwrap().n;
        asked.push(n);
        s = c.resume(n * n);
    }
    assert!(matches!(s, Step::Done(ref v) if v == &vec![1, 4, 9]), "{s:?}");
    assert_eq!(asked, vec![1, 2, 3]);
}
