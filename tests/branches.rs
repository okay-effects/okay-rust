//! Branches on one thread (okay::typed `join`, `both`, `race`): okay-effects' `all`, `par`, `race` on the one API —
//! the branches' waits overlap, their effects' handlers shared, a host answering them one at a time
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use okay::effect;
use okay::typed::*;

struct Timer {
    shared: Arc<Mutex<(bool, Option<Waker>)>>,
    after: Option<Duration>,
}

fn sleep(ms: u64) -> Timer {
    Timer { shared: Arc::new(Mutex::new((false, None))), after: Some(Duration::from_millis(ms)) }
}

impl Future for Timer {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<()> {
        let mut s = self.shared.lock().unwrap();
        if s.0 {
            return Poll::Ready(());
        }
        s.1 = Some(c.waker().clone());
        drop(s);
        if let Some(d) = self.after.take() {
            let shared = self.shared.clone();
            std::thread::spawn(move || {
                std::thread::sleep(d);
                let mut s = shared.lock().unwrap();
                s.0 = true;
                if let Some(w) = s.1.take() {
                    w.wake();
                }
            });
        }
        Poll::Pending
    }
}

effect! { enum Ask: Plain { Ask(n: i64) -> i64 = ask_n; } }
effect! { enum Tick: Plain { Tick() -> () = tick; } }
effect! { enum Slow: Plain, waits { Slow(n: i64) -> i64 = slow; } }

/// each ask answered by `f`, the asks logged
struct Answer<F>(F, RefCell<Vec<i64>>);
impl<F: Fn(i64) -> i64> Handles<Ask> for Answer<F> {
    fn handle(&self, op: Ask) -> Step<'_> {
        match op {
            Ask::Ask(n, r) => {
                self.1.borrow_mut().push(n);
                Step::Now(r.give((self.0)(n)))
            }
        }
    }
}
fn answer<F: Fn(i64) -> i64>(f: F) -> Answer<F> {
    Answer(f, RefCell::new(vec![]))
}

struct Ticks(Cell<u32>);
impl Handles<Tick> for Ticks {
    fn handle(&self, op: Tick) -> Step<'_> {
        match op {
            Tick::Tick(r) => {
                self.0.set(self.0.get() + 1);
                Step::Now(r.give(()))
            }
        }
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
fn join_waits_concurrently() {
    let t = Instant::now();
    let got = block_on(join([50u64, 50].map(|ms| async move {
        sleep(ms).await;
        ms
    })));
    assert_eq!(got, vec![50, 50]);
    assert!(t.elapsed() < Duration::from_millis(90), "{:?}", t.elapsed());
}

#[test]
fn join_values_in_order_whichever_ends_first() {
    let got = block_on(join([30u64, 0, 15].map(|ms| async move {
        sleep(ms).await;
        ms
    })));
    assert_eq!(got, vec![30, 0, 15]);
}

#[test]
fn branches_perform_through_a_shared_handler() {
    let cx = Cx(answer(|n| n * 10), End);
    let cx = &cx;
    let got = block_on(join((0..3i64).map(|i| async move {
        let mut s = 0;
        for k in 0..3 {
            s += ask_n(cx, i * 100 + k).await;
            sleep(5).await;
        }
        s
    })));
    assert_eq!(got, vec![30, 3030, 6030]);
    let mut seen = cx.0.1.borrow().clone();
    assert_eq!(&seen[..3], &[0, 100, 200]); // each branch's first, in turn, before any sleep ends
    seen.sort();
    assert_eq!(seen, vec![0, 1, 2, 100, 101, 102, 200, 201, 202]);
}

#[test]
fn each_branch_with_its_own_handlers() {
    // a capability by identity: the context a branch passes picks the handler — its own, or the parent's
    let parent = Cx(answer(|n| n + 1000), End);
    let parent = &parent;
    let got = block_on(join((1..=3i64).map(|i| async move {
        let mine = Cx(answer(move |n| n * i), End);
        let x = ask_n(&mine, 10).await;
        sleep(10).await;
        let y = ask_n(&mine, 1).await;
        x + y + ask_n(parent, i).await
    })));
    assert_eq!(got, vec![11 + 1001, 22 + 1002, 33 + 1003]);
}

/// a slow answer: a tick performed outside, then a wait, then the value
struct Translate<'o>(&'o Cx<Ticks, End>);
impl Handles<Slow> for Translate<'_> {
    fn handle(&self, op: Slow) -> Step<'_> {
        match op {
            Slow::Slow(n, r) => Step::Wait(Box::pin(async move {
                tick(self.0).await;
                sleep(5).await;
                r.give(n)
            })),
        }
    }
}

#[test]
fn a_translating_parent_handler_answers_branches() {
    let outer = Cx(Ticks(Cell::new(0)), End);
    let cx = Cx(Translate(&outer), End);
    let cx = &cx;
    let got = block_on(join((0..4i64).map(|i| async move { slow(cx, i).await + slow(cx, i).await })));
    assert_eq!((got, outer.0.0.get()), (vec![0, 2, 4, 6], 8));
}

#[test]
fn both_of_two_types() {
    let cx = Cx(answer(|n| n), End);
    let cx = &cx;
    let got = block_on(both(
        async {
            sleep(10).await;
            ask_n(cx, 7).await
        },
        async { "x".repeat(ask_n(cx, 3).await as usize) },
    ));
    assert_eq!(got, (7, "xxx".to_string()));
}

#[test]
fn race_first_wins_the_others_dropped() {
    let cleaned = Rc::new(Cell::new(false));
    let c = &cleaned;
    let t = Instant::now();
    let got = block_on(race([10u64, 500].map(|ms| async move {
        let _guard = (ms == 500).then(|| Cleanup(c.clone()));
        sleep(ms).await;
        ms
    })));
    assert_eq!(got, 10);
    assert!(cleaned.get(), "the loser's destructors ran");
    assert!(t.elapsed() < Duration::from_millis(300));
}

#[test]
fn a_failing_branch_fails_all_and_drops_the_others() {
    let cleaned = Rc::new(Cell::new(false));
    let c = &cleaned;
    let p = catch_unwind(AssertUnwindSafe(|| {
        block_on(join([0, 1].map(|i| async move {
            let _guard = Cleanup(c.clone());
            if i == 1 {
                sleep(10).await;
                panic!("branch 1 failed");
            }
            sleep(500).await;
            c.set(false);
            i
        })))
    }));
    assert_eq!(message(p.unwrap_err()), "branch 1 failed");
    assert!(cleaned.get());
}

#[test]
fn an_error_raised_in_a_branch_drops_every_branch() {
    let dropped = Rc::new(Cell::new(0));
    let d = &dropped;
    let got = block_on(either(&End, async |cx| {
        join((0..3i64).map(|i| async move {
            struct Count<'a>(&'a Cell<i32>);
            impl Drop for Count<'_> {
                fn drop(&mut self) {
                    self.0.set(self.0.get() + 1);
                }
            }
            let _c = Count(d);
            sleep(10 * i as u64).await;
            if i == 2 { raise::<String, i64, _, _>(cx, "two".into()).await } else { i }
        }))
        .await
    }));
    assert_eq!(got, Err("two".to_string()));
    assert_eq!(dropped.get(), 3);
}

#[test]
fn nested_branches() {
    let cx = Cx(answer(|n| n * 2), End);
    let cx = &cx;
    let got = block_on(join((0..2i64).map(|i| async move {
        join((0..2i64).map(|j| async move {
            sleep(5).await;
            ask_n(cx, i * 10 + j).await
        }))
        .await
    })));
    assert_eq!(got, vec![vec![0, 2], vec![20, 22]]);
}
