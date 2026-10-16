//! EFFECTS WAITING ON THE WORLD (okay::typed): a handler answering when a future is done — `Step::Wait` — the program
//! waiting as any future does, woken by it; `block_on` runs it, `both` runs two at once. The replay, a watch, an
//! interceptor, either and generate the same over them

use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use okay::audit::Audit;
use okay::effects::State;
use okay::typed::*;
use okay::{eff, effect};

// THE WORLD: a timer — a thread sleeping, then waking the program

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

effect! {
    /// the world's answers, each after a while
    enum Net: Plain, waits {
        /// a value by its key, `ms` later
        Fetch(key: String, ms: u64) -> String = fetch;
    }
}

/// the network: each fetch answered when its timer is done; the fetches counted
struct Network {
    fetched: Cell<u32>,
}
impl Network {
    fn new() -> Self {
        Network { fetched: Cell::new(0) }
    }
}
impl Handles<Net> for Network {
    fn handle(&self, op: Net) -> Step<'_> {
        match op {
            Net::Fetch(key, ms, r) => Step::Wait(Box::pin(async move {
                sleep(ms).await;
                self.fetched.set(self.fetched.get() + 1);
                r.give(format!("<{key}>"))
            })),
        }
    }
}

/// "a program waiting on the world: block_on runs it; run, polling once, says it is stuck"
#[test]
fn a_program_waiting_on_the_world() {
    let cx = Cx(Network::new(), Cx(StateCell::new(0i64), End));
    okay::needs! {
        async fn twice(cx: [Net, State<i64>]) -> String {
            eff! {
                a <- fetch(cx, "a".into(), 20);
                _ <- modify(cx, |s| s + 1);
                b <- fetch(cx, "b".into(), 20);
                _ <- modify(cx, |s| s + 1);
                yield format!("{a}{b}")
            }
            .await
        }
    }
    assert_eq!(block_on(twice(&cx)), "<a><b>");
    assert_eq!(cx.0.fetched.get(), 2);
    assert_eq!(cx.1.0.take(), 2);
    let cx = Cx(Network::new(), Cx(StateCell::new(0i64), End));
    assert_eq!(run(twice(&cx)), Err(Stuck));
}

/// "two programs at once: each goes on while the other waits"
#[test]
fn two_programs_at_once() {
    let cx = Cx(Network::new(), End);
    let t = Instant::now();
    let (a, b) = block_on(both(fetch(&cx, "a".into(), 150), fetch(&cx, "b".into(), 150)));
    let took = t.elapsed();
    assert_eq!((a.as_str(), b.as_str()), ("<a>", "<b>"));
    assert!(took < Duration::from_millis(280), "not at once: {took:?}");
}

/// "the replay over an answer waited for: asked once, told again to every path"
#[test]
fn the_replay_over_an_answer_waited_for() {
    let cx = Cx(Network::new(), End);
    let r = block_on(all(&cx, async |cx| {
        let v = fetch(cx, "k".into(), 10).await;
        let x = among(cx, &[1, 2, 3]).await;
        format!("{v}{x}")
    }));
    assert_eq!(r, vec!["<k>1", "<k>2", "<k>3"]);
    assert_eq!(cx.0.fetched.get(), 1);
}

/// "a watch sees an answer waited for when it comes"
#[test]
fn a_watch_sees_an_answer_waited_for() {
    struct Lines(RefCell<Vec<String>>);
    impl Watch for &Lines {
        fn before(&self, effect: &str, op: &dyn Debug) -> Result<(), String> {
            self.0.borrow_mut().push(format!("{effect} {op:?}"));
            Ok(())
        }
        fn after(&self, _: &str, _: &dyn Debug, answer: &dyn Debug) {
            self.0.borrow_mut().push(format!("  → {answer:?}"));
        }
    }
    let lines = Lines(RefCell::new(vec![]));
    let cx = Cx(Network::new(), End);
    let r = block_on(observe(&cx, &lines, async |cx| fetch(cx, "k".into(), 10).await));
    assert_eq!(r, Ok("<k>".to_string()));
    assert_eq!(lines.0.into_inner(), vec!["Net Fetch(\"k\", 10, _)", "  → \"<k>\""]);
}

effect! {
    enum Pay: Sensitive, waits {
        /// money sent, a receipt when the bank has it
        Transfer(to: String, amount: i64) -> String = transfer;
    }
}

/// a bank far away: each transfer answered later
struct RemoteBank;
impl Handles<Pay> for RemoteBank {
    fn handle(&self, op: Pay) -> Step<'_> {
        match op {
            Pay::Transfer(to, amount, r) => Step::Wait(Box::pin(async move {
                sleep(10).await;
                r.give(format!("ok:{to}:{amount}"))
            })),
        }
    }
}

/// an audit waiting for the bank: the transfer passed on, its receipt recorded when it comes
struct Recorded;
impl<R: Has<Audit, I>, I> Interceptor<Pay, R, I> for Recorded {
    fn intercept<'a, H: Handles<Pay>>(&'a self, op: Pay, pass: Pass<'a, H>, cx: &'a R) -> Step<'a> {
        match op {
            Pay::Transfer(to, amount, r) => Step::Wait(Box::pin(async move {
                let account = to.clone();
                let receipt = pass.wait(|r2| Pay::Transfer(account, amount, r2)).await.expect("the bank answers");
                record(cx, format!("transfer {amount} to {to} → {receipt}")).await;
                r.give(receipt)
            })),
        }
    }
}

/// "an interceptor waiting for its handler: the receipt recorded when it comes"
#[test]
fn an_interceptor_waiting_for_its_handler() {
    let cx = Cx(Audited::new(RemoteBank, Recorded), Cx(Trail::new(), End));
    let receipt = block_on(transfer(&cx, "acct".into(), 5));
    assert_eq!(receipt, "ok:acct:5");
    assert_eq!(cx.1.0.take(), vec!["transfer 5 to acct → ok:acct:5"]);
}

/// "either over a wait: the error raised after it, the rest dropped"
#[test]
fn either_over_a_wait() {
    let cx = Cx(Network::new(), End);
    let r: Result<String, String> = block_on(either(&cx, async |cx| {
        let v = fetch(cx, "k".into(), 10).await;
        raise(cx, format!("no {v}")).await
    }));
    assert_eq!(r, Err("no <k>".to_string()));
}

/// "a generator waiting between its values"
#[test]
fn a_generator_waiting_between_its_values() {
    let cx = Cx(Network::new(), End);
    let seen = RefCell::new(vec![]);
    let n = block_on(generate(&cx, |v: String| seen.borrow_mut().push(v), async |cx| {
        for k in ["a", "b", "c"] {
            let v = fetch(cx, k.into(), 5).await;
            emit(cx, v).await;
        }
        3
    }));
    assert_eq!(n, 3);
    assert_eq!(seen.into_inner(), vec!["<a>", "<b>", "<c>"]);
}

effect! {
    enum Slow: Plain {
        /// not declared to wait
        Tick() -> u8 = tick;
    }
}

/// "a handler answering later an effect not declared to wait: said so"
#[test]
#[should_panic(expected = "is not an effect that waits")]
fn an_effect_not_declared_to_wait() {
    struct Late;
    impl Handles<Slow> for Late {
        fn handle(&self, op: Slow) -> Step<'_> {
            match op {
                Slow::Tick(r) => Step::Wait(Box::pin(async move { r.give(1) })),
            }
        }
    }
    let _ = block_on(tick(&Cx(Late, End)));
}
