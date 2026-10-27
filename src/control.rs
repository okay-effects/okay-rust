//! CONTROL: a program ended early (`Abort`, `optional`), run again while it fails (`retry`, `backoff`), given a time
//! (`timeout`), and a SCOPE — tasks started in it run beside its body and are cancelled when it ends, its resources
//! released, the last first, however it ends: its value, an error raised past it, a panic, a host's cancel.
//!
//! ```
//! use okay::control::*;
//! use okay::services::*;
//! use okay::typed::*;
//!
//! // a program that fails twice, then answers
//! let tries = std::cell::Cell::new(0);
//! let cx = Cx(VirtualClock::at(0), End);
//! let got = run(backoff(&cx, 5, 100, 2, |e: &String| e == "busy", async |cx| {
//!     tries.set(tries.get() + 1);
//!     if tries.get() < 3 { raise(cx, "busy".to_string()).await } else { 42 }
//! }));
//! assert_eq!(got, Ok(Ok(42)));
//! assert_eq!(cx.0.now(), 300);   // waited 100, then 200 — virtual time, at once
//! ```

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::{pin, Pin};
use std::rc::Rc;
use std::task::Poll;

use crate::kinds::{Effect, Plain};
use crate::reply::{Never, Reply};
use crate::services::{now_ms, sleep_ms, Clock};
use crate::typed::{either, perform, Cx, Handles, Has, Raised, Step};

// ENDED EARLY

/// the program ended here, no value: `optional` gives `None`
#[derive(Clone, Debug)]
pub enum Abort {
    Exit(Reply<Never>),
}
impl Effect for Abort {
    type Kind = Plain;
}

/// the handler `optional` puts in the context: the rest taken, never given back
pub struct Aborted(Cell<bool>);
impl Handles<Abort> for Aborted {
    fn handle(&self, op: Abort) -> Step<'_> {
        match op {
            Abort::Exit(_) => {
                self.0.set(true);
                Step::Later
            }
        }
    }
}

/// the program ended: the body dropped where it is
pub async fn exit<A, C: Has<Abort, I>, I>(cx: &C) -> A {
    let never: Never = perform(cx, Abort::Exit).await;
    match never {}
}

/// `body`'s value, or `None` — it exited
pub async fn optional<'c, A, C>(cx: &'c C, body: impl AsyncFnOnce(&Cx<Aborted, &'c C>) -> A) -> Option<A> {
    let inner = Cx(Aborted(Cell::new(false)), cx);
    let mut fut = pin!(body(&inner));
    std::future::poll_fn(|c| match fut.as_mut().poll(c) {
        Poll::Ready(a) => Poll::Ready(Some(a)),
        Poll::Pending if inner.0.0.get() => Poll::Ready(None),
        Poll::Pending => Poll::Pending,
    })
    .await
}

// RUN AGAIN WHILE IT FAILS

/// `body` run until it gives a value or raises an error `retry_on` does not take — at most `times` runs: its value,
/// or the last error
pub async fn retry<'c, E, A, C>(
    cx: &'c C,
    times: u32,
    retry_on: impl Fn(&E) -> bool,
    body: impl AsyncFn(&Cx<Raised<E>, &'c C>) -> A,
) -> Result<A, E>
where
    E: Clone + std::fmt::Debug + 'static,
{
    let mut left = times.max(1);
    loop {
        left -= 1;
        match either(cx, async |cx| body(cx).await).await {
            Ok(a) => return Ok(a),
            Err(e) if left > 0 && retry_on(&e) => continue,
            Err(e) => return Err(e),
        }
    }
}

/// `retry`, waiting on the context's clock between runs: `first_ms`, then each wait `factor` times the last
pub async fn backoff<'c, E, A, C, I>(
    cx: &'c C,
    times: u32,
    first_ms: u64,
    factor: u64,
    retry_on: impl Fn(&E) -> bool,
    body: impl AsyncFn(&Cx<Raised<E>, &'c C>) -> A,
) -> Result<A, E>
where
    E: Clone + std::fmt::Debug + 'static,
    C: Has<Clock, I>,
{
    let mut left = times.max(1);
    let mut wait = first_ms;
    loop {
        left -= 1;
        match either(cx, async |cx| body(cx).await).await {
            Ok(a) => return Ok(a),
            Err(e) if left > 0 && retry_on(&e) => {
                sleep_ms(cx, wait).await;
                wait = wait.saturating_mul(factor);
            }
            Err(e) => return Err(e),
        }
    }
}

// A TIME GIVEN

/// `body`'s value, or `None` — not done within `ms` on the context's clock: the body dropped where it is (its
/// finalizers run). On the system's clock a race with a timer; on a virtual one, whose waits take no real time, the
/// body run and its time compared: done later than the deadline, it timed out
pub async fn timeout<A, C: Has<Clock, I>, I>(cx: &C, ms: u64, body: impl Future<Output = A>) -> Option<A> {
    let deadline = now_ms(cx).await.saturating_add(ms);
    let mut body = pin!(body);
    let mut timer: Option<Pin<Box<dyn Future<Output = ()> + '_>>> = None;
    let done = std::future::poll_fn(|c| {
        if let Poll::Ready(a) = body.as_mut().poll(c) {
            return Poll::Ready(Some(a));
        }
        let t = timer.get_or_insert_with(|| Box::pin(sleep_ms(cx, ms)));
        match t.as_mut().poll(c) {
            Poll::Ready(()) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    })
    .await;
    match done {
        Some(a) if now_ms(cx).await <= deadline => Some(a),
        _ => None,
    }
}

// A SCOPE: tasks beside its body, resources released at its end

type Task<'s> = Option<Pin<Box<dyn Future<Output = ()> + 's>>>;

/// A SCOPE: tasks started in it (`spawn`) run beside its body, polled with it on one thread; resources (`acquire`,
/// `defer`) released when it ends, the last first
pub struct Scope<'s> {
    tasks: RefCell<Vec<Task<'s>>>,
    cancelled: RefCell<Vec<usize>>,
    releases: RefCell<Vec<Box<dyn FnOnce() + 's>>>,
}

/// A TASK started in a scope: its value when done (`join`), or cancelled (`cancel`: dropped where it is)
pub struct Handle<T> {
    index: usize,
    value: Rc<RefCell<Option<T>>>,
}

impl<'s> Scope<'s> {
    /// `task` started: it runs beside the scope's body until done, cancelled, or the scope ends
    pub fn spawn<T: 's>(&self, task: impl Future<Output = T> + 's) -> Handle<T> {
        let value = Rc::new(RefCell::new(None));
        let slot = value.clone();
        let mut tasks = self.tasks.borrow_mut();
        tasks.push(Some(Box::pin(async move {
            let v = task.await;
            *slot.borrow_mut() = Some(v);
        })));
        Handle { index: tasks.len() - 1, value }
    }

    /// the task stopped where it is: dropped (its destructors and finalizers run); `join` on it waits for ever
    pub fn cancel<T>(&self, h: &Handle<T>) {
        self.cancelled.borrow_mut().push(h.index);
    }

    /// `value` kept until the scope ends, then given to `release`
    pub fn acquire<T: 's>(&self, value: T, release: impl FnOnce(&T) + 's) -> Rc<T> {
        let v = Rc::new(value);
        let kept = v.clone();
        self.defer(move || release(&kept));
        v
    }

    /// `f` run when the scope ends
    pub fn defer(&self, f: impl FnOnce() + 's) {
        self.releases.borrow_mut().push(Box::new(f));
    }

    /// every task polled once, the cancelled dropped: whether one finished
    fn poll_tasks(&self, c: &mut std::task::Context<'_>) -> bool {
        for i in self.cancelled.borrow_mut().drain(..) {
            if let Some(t) = self.tasks.borrow_mut().get_mut(i) {
                *t = None;
            }
        }
        let mut finished = false;
        let n = self.tasks.borrow().len();
        for i in 0..n {
            // taken out while polled: a task may spawn another
            let Some(mut t) = self.tasks.borrow_mut()[i].take() else { continue };
            match t.as_mut().poll(c) {
                Poll::Ready(()) => finished = true,
                Poll::Pending => self.tasks.borrow_mut()[i] = Some(t),
            }
        }
        finished
    }
}

impl<T> Handle<T> {
    /// the task's value, when it is done
    pub async fn join(&self) -> T {
        std::future::poll_fn(|_| match self.value.borrow_mut().take() {
            Some(v) => Poll::Ready(v),
            None => Poll::Pending,
        })
        .await
    }
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        // the tasks left cancelled, then the resources released, the last first
        self.tasks.borrow_mut().clear();
        let releases = std::mem::take(&mut *self.releases.borrow_mut());
        for f in releases.into_iter().rev() {
            f();
        }
    }
}

/// `body` with a scope: its value when it is done — the tasks left cancelled, the resources released, the last
/// first; the same when it is dropped (an error raised past it, a race lost, a host's cancel) or panics
pub async fn scope<'s, A>(body: impl AsyncFnOnce(&Scope<'s>) -> A) -> A {
    let s = Scope {
        tasks: RefCell::new(Vec::new()),
        cancelled: RefCell::new(Vec::new()),
        releases: RefCell::new(Vec::new()),
    };
    let s = &s;
    let mut fut = pin!(body(s));
    std::future::poll_fn(|c| loop {
        if let Poll::Ready(a) = fut.as_mut().poll(c) {
            return Poll::Ready(a);
        }
        // a task done may be what the body waits for: polled again then
        if !s.poll_tasks(c) {
            return Poll::Pending;
        }
    })
    .await
}
