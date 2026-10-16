//! ONE API, COMPILED BY THE HANDLER'S TYPE: a program written with `eff! { … }` is an `async` block — a state
//! machine the compiler builds, generic over its context `cx`, the handlers in force. Monomorphized for the handlers it
//! runs under: an operation of a handler answering in place is `Ready` at once — inlined, a call of the handler; one
//! dropping the rest ends the block (`either`); one taking the rest once suspends it — the state machine itself the
//! `k`, no node on the heap (`generate`); one taking it many times runs it again for each path, its past told it
//! again (`all`, `first`: replay). An effect's handler under an `Interceptor` (`Audited::new(handler, interceptor)`):
//! each operation passed on, changed, refused — a sensitive effect is in a context only so; every operation of every
//! effect seen by a `Watch` under `observe`.
//!
//! ```
//! use okay::eff;
//! use okay::effects::{Reader, State, Writer};
//! use okay::typed::*;
//!
//! okay::needs! {
//!     async fn counted(cx: [Reader<i64>, State<i64>, Writer<String>], n: i64) -> i64 {
//!         eff! {
//!             d <- ask(cx);
//!             _ <- modify(cx, |s| s + d * n);
//!             s <- get(cx);
//!             _ <- tell(cx, format!("s {s}"));
//!             yield s
//!         }.await
//!     }
//! }
//!
//! let cx = Cx(AskWith(2i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
//! assert_eq!(run(counted(&cx, 10)), Ok(21));
//! assert_eq!(cx.1.1.0.take(), vec!["s 21".to_string()]);
//! ```
//!
//! A sensitive effect's bare handler is not a context's:
//!
//! ```compile_fail
//! use okay::typed::*;
//! okay::effect! { enum Pay: Sensitive { Transfer(n: i64) -> String = transfer; } }
//! struct Bank;
//! impl Handles<Pay> for Bank {
//!     fn handle(&self, op: Pay) -> Step<'_> { match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("ok {n}"))) } }
//! }
//! let cx = Cx(Bank, End);
//! let _ = run(transfer(&cx, 5));
//! ```
//!
//! nor one under something not its interceptor:
//!
//! ```compile_fail
//! use okay::typed::*;
//! okay::effect! { enum Pay: Sensitive { Transfer(n: i64) -> String = transfer; } }
//! struct Bank;
//! impl Handles<Pay> for Bank {
//!     fn handle(&self, op: Pay) -> Step<'_> { match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("ok {n}"))) } }
//! }
//! struct NotAnInterceptor;
//! let cx = Cx(Audited::new(Bank, NotAnInterceptor), End);
//! let _ = run(transfer(&cx, 5));
//! ```
//!
//! and under its interceptor the handler is not reached past it:
//!
//! ```compile_fail
//! use okay::typed::*;
//! okay::effect! { enum Pay: Sensitive { Transfer(n: i64) -> String = transfer; } }
//! struct Bank;
//! impl Handles<Pay> for Bank {
//!     fn handle(&self, op: Pay) -> Step<'_> { match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("ok {n}"))) } }
//! }
//! struct Guard;
//! impl<R> Interceptor<Pay, R, ()> for Guard {
//!     fn intercept<H: Handles<Pay>>(&self, op: Pay, pass: Pass<'_, H>, _: &R) -> Step<'_> {
//!         match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("refused {n}"))) }
//!     }
//! }
//! let cx = Cx(Audited::new(Bank, Guard), End);
//! let _ = cx.0.handler.handle(Pay::Transfer(5, todo!()));
//! ```
//!
//! The one way:
//!
//! ```
//! use okay::typed::*;
//! okay::effect! { enum Pay: Sensitive { Transfer(n: i64) -> String = transfer; } }
//! struct Bank;
//! impl Handles<Pay> for Bank {
//!     fn handle(&self, op: Pay) -> Step<'_> { match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("ok {n}"))) } }
//! }
//! struct Limit(i64);
//! impl<R> Interceptor<Pay, R, ()> for Limit {
//!     fn intercept<H: Handles<Pay>>(&self, op: Pay, pass: Pass<'_, H>, _: &R) -> Step<'_> {
//!         match op {
//!             Pay::Transfer(n, r) if n > self.0 => Step::Now(r.give(format!("refused {n}"))),
//!             Pay::Transfer(n, r) => pass.perform(|r2| Pay::Transfer(n, r2)).map_or(Step::Later, |s| Step::Now(r.give(s))),
//!         }
//!     }
//! }
//! let cx = Cx(Audited::new(Bank, Limit(100)), End);
//! assert_eq!(run(transfer(&cx, 5)), Ok("ok 5".to_string()));
//! assert_eq!(run(transfer(&cx, 500)), Ok("refused 500".to_string()));
//! ```
use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::{pin, Pin};
use std::task::{Context, Poll, Waker};

use crate::eff::{short, Answer, Loop, Never, Reply};
use crate::audit::Audit;
use crate::effects::{Choose, Reader, State, Throws, Writer};
use crate::val::Val;
use crate::row::{Effect, Here, Plain, There};

// HANDLERS

/// what a handler does with an operation: answers it now, takes the rest (the program suspended until the handler
/// gives it back), or answers it when a future is done — the world's: a timer, the network, a channel; polled with
/// the program's own `Context`, so the program waits as any future does, on any executor
pub enum Step<'h> {
    Now(Answer),
    Later,
    Wait(Pin<Box<dyn Future<Output = Answer> + 'h>>),
}

/// A HANDLER OF `E` in a context: `handle` each operation; an operation it took (`Later`) answered by `resume` when it
/// gives the rest back — once
pub trait Handles<E: Effect> {
    fn handle(&self, op: E) -> Step<'_>;
    fn resume(&self) -> Option<Answer> {
        None
    }
}

/// a handler borrowed: a part of a program run with its own context, the handler shared with the whole
impl<E: Effect, H: Handles<E>> Handles<E> for &H {
    #[inline(always)]
    fn handle(&self, op: E) -> Step<'_> {
        (**self).handle(op)
    }
    #[inline(always)]
    fn resume(&self) -> Option<Answer> {
        (**self).resume()
    }
}

mod sealed {
    /// a type no one outside okay names: what `Allowed`'s method asks
    pub struct Token;
}

/// A HANDLER ALLOWED FOR AN EFFECT OF THE KIND `K` in a context by itself: any for a plain effect, none for a sensitive
/// one — that is handled only under an interceptor (`Audited::new(handler, interceptor)`). Sealed: its method asks a
/// type no one outside okay can name
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not under an audit: a sensitive effect is handled only through one",
    label = "not an audit",
    note = "put `Audited::new(handler, interceptor)` in the context, not the bare handler"
)]
pub trait Allowed<K> {
    #[doc(hidden)]
    fn allowed(&self, _: sealed::Token);
}

impl<H> Allowed<Plain> for H {
    fn allowed(&self, _: sealed::Token) {}
}

// INTERCEPTORS: an effect's operations seen on their way to its handler — passed on as they are or changed, their
// answers changed, refused

/// AN INTERCEPTOR OF `E` around a handler: each operation on its way to it — passed on (`pass.perform(…)`, as it is
/// or changed), its answer given or changed (`r.give(…)`), refused (an answer saying so, or an error raised further
/// out: `Step::Later`, the rest dropped). Its own operations go to `cx`, the context after its handler — a trail, a
/// `Throws` — found at `Ix`
pub trait Interceptor<E: Effect, R, Ix> {
    fn intercept<'a, H: Handles<E>>(&'a self, op: E, pass: Pass<'a, H>, cx: &'a R) -> Step<'a>;
}

/// the handler an interceptor passes an operation on to — the only way to it
pub struct Pass<'h, H>(&'h H);

impl<'h, H> Pass<'h, H> {
    /// the operation `make` given a reply, handled: its value now, or none — the handler took the rest, or answers
    /// later (`wait` for it)
    pub fn perform<E: Effect, X: 'static>(&self, make: impl FnOnce(Reply<X>) -> E) -> Option<X>
    where
        H: Handles<E>,
    {
        match self.0.handle(make(Reply::new())) {
            Step::Now(a) => Some(a.0.take::<X>()),
            Step::Later | Step::Wait(_) => None,
        }
    }

    /// the operation `make` given a reply, handled — its value when the handler has it (an interceptor answering
    /// `Step::Wait(Box::pin(async move { … pass.wait(…).await … }))`), or none: the handler took the rest
    pub fn wait<E: Effect, X: 'static>(&self, make: impl FnOnce(Reply<X>) -> E) -> impl Future<Output = Option<X>> + 'h
    where
        H: Handles<E>,
    {
        let step = self.0.handle(make(Reply::new()));
        async move {
            match step {
                Step::Now(a) => Some(a.0.take::<X>()),
                Step::Later => None,
                Step::Wait(f) => Some(f.await.0.take::<X>()),
            }
        }
    }
}

/// A HANDLER UNDER ITS INTERCEPTOR: each operation of the effect goes through the interceptor, the handler reached
/// only by it — the only way a sensitive effect is handled. Its handler not reachable from outside it either
pub struct Audited<H, I> {
    handler: H,
    interceptor: I,
}

impl<H, I> Audited<H, I> {
    pub fn new(handler: H, interceptor: I) -> Self {
        Audited { handler, interceptor }
    }
    /// the interceptor, its state seen
    pub fn interceptor(&self) -> &I {
        &self.interceptor
    }
    /// the two, the run over
    pub fn into_parts(self) -> (H, I) {
        (self.handler, self.interceptor)
    }
}

/// the index of an effect handled under an interceptor: the interceptor's own (`Ix`, where its operations go)
pub struct Through<Ix>(PhantomData<Ix>);

// THE CONTEXT: the handlers in force, a list — `Cx(h, rest)`, `End`

pub struct Cx<H, R>(pub H, pub R);
pub struct End;

/// `E` HANDLED IN THE CONTEXT `Self`, at `Ix` — found by the compiler, as `Member`: an operation given to its handler
/// — on the way through each layer between (a replay, an observer)
#[diagnostic::on_unimplemented(
    message = "no handler of `{E}` in the context `{Self}`",
    note = "put a handler of `{E}` in the context; a sensitive effect's only under an interceptor of it: `Audited::new(handler, interceptor)`"
)]
pub trait Has<E: Effect, Ix> {
    /// the operation's value now, none yet (taken, the program suspended), or a future of it
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T>;
    /// the operation taken given back: its value, or none — taken still
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T>;
}

/// what an operation got from its context: its value now, none yet (taken: the program suspended until given back),
/// or a future of it
pub enum Got<'a, T> {
    Now(T),
    Taken,
    Wait(Pin<Box<dyn Future<Output = T> + 'a>>),
}

impl<'a, T: 'static> Got<'a, T> {
    #[inline(always)]
    fn of(step: Step<'a>) -> Self {
        match step {
            Step::Now(a) => Got::Now(a.0.take::<T>()),
            Step::Later => Got::Taken,
            Step::Wait(f) => Got::Wait(Box::pin(async move { f.await.0.take::<T>() })),
        }
    }
}

impl<E: Effect, H: Handles<E> + Allowed<E::Kind>, R> Has<E, Here> for Cx<H, R> {
    #[inline(always)]
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        Got::of(self.0.handle(op))
    }
    #[inline(always)]
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        self.0.resume().map(|a| a.0.take::<T>())
    }
}

impl<E: Effect, H, R: Has<E, Ix>, Ix> Has<E, There<Ix>> for Cx<H, R> {
    #[inline(always)]
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        self.1.perform(op)
    }
    #[inline(always)]
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        self.1.resume()
    }
}

impl<E: Effect, Ix, C: Has<E, Ix>> Has<E, Ix> for &C {
    #[inline(always)]
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        (**self).perform(op)
    }
    #[inline(always)]
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        (**self).resume()
    }
}

impl<E: Effect, H: Handles<E>, I: Interceptor<E, R, Ix>, R, Ix> Has<E, Through<Ix>> for Cx<Audited<H, I>, R> {
    #[inline]
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        Got::of(self.0.interceptor.intercept(op, Pass(&self.0.handler), &self.1))
    }
    /// the rest taken by an interceptor (an error raised further out) is not given back
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        None
    }
}

// AN OPERATION: a future — answered at once by a handler in place, else pending until its handler resumes it. An
// operation of an effect that waits on the world (`effect!`'s `waits`) is a `Waiting`: it keeps the future its
// handler answered with, and drops it if the program is dropped first. A `Perform` keeps none — nothing to drop, so
// a program's loop over it compiles to the handler's code alone (a field to drop costs 2–7× on `reads`, `state`)

pub struct Perform<'c, C, E, Ix, T> {
    cx: &'c C,
    op: Option<E>,
    _t: PhantomData<fn() -> (Ix, T)>,
}

impl<'c, C, E, Ix, T> Perform<'c, C, E, Ix, T> {
    #[inline(always)]
    pub fn new(cx: &'c C, op: E) -> Self {
        Perform { cx, op: Some(op), _t: PhantomData }
    }
}

// not pinned by anything in it: no field refers to another
impl<C, E, Ix, T> Unpin for Perform<'_, C, E, Ix, T> {}

impl<C: Has<E, Ix>, E: Effect, Ix, T: Clone + Debug + 'static> Future for Perform<'_, C, E, Ix, T> {
    type Output = T;
    #[inline(always)]
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        let me = self.get_mut();
        let got = match me.op.take() {
            Some(op) => me.cx.perform::<T>(op),
            None => {
                return match me.cx.resume::<T>() {
                    Some(t) => Poll::Ready(t),
                    None => Poll::Pending,
                };
            }
        };
        match got {
            Got::Now(t) => Poll::Ready(t),
            Got::Taken => Poll::Pending,
            Got::Wait(_) => not_waiting(std::any::type_name::<E>()),
        }
    }
}

#[cold]
fn not_waiting(effect: &str) -> ! {
    panic!(
        "okay: a handler of `{effect}` answered later (`Step::Wait`), but `{effect}` is not an effect that waits — \
         declare it so: `effect! {{ enum … : Plain, waits {{ … }} }}`, its operations `waiting(cx, …)`"
    )
}

/// AN OPERATION OF AN EFFECT THAT WAITS: as `Perform`, and the future its handler answered with kept until it is done
pub struct Waiting<'c, C, E, Ix, T> {
    cx: &'c C,
    op: Option<E>,
    waiting: Option<Pin<Box<dyn Future<Output = T> + 'c>>>,
    _t: PhantomData<fn() -> Ix>,
}

impl<C, E, Ix, T> Unpin for Waiting<'_, C, E, Ix, T> {}

impl<C: Has<E, Ix>, E: Effect, Ix, T: Clone + Debug + 'static> Future for Waiting<'_, C, E, Ix, T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<T> {
        let me = self.get_mut();
        if let Some(w) = me.waiting.as_mut() {
            let p = w.as_mut().poll(c);
            if p.is_ready() {
                me.waiting = None;
            }
            return p;
        }
        let got = match me.op.take() {
            Some(op) => me.cx.perform::<T>(op),
            None => {
                return match me.cx.resume::<T>() {
                    Some(t) => Poll::Ready(t),
                    None => Poll::Pending,
                };
            }
        };
        match got {
            Got::Now(t) => Poll::Ready(t),
            Got::Taken => Poll::Pending,
            Got::Wait(mut f) => {
                let p = f.as_mut().poll(c);
                if p.is_pending() {
                    me.waiting = Some(f);
                }
                p
            }
        }
    }
}

/// an operation of an effect that waits on the world: `make` its variant given its reply
#[inline(always)]
pub fn waiting<'c, C: Has<E, Ix>, E: Effect, Ix, T: Clone + Debug + 'static>(
    cx: &'c C,
    make: impl FnOnce(Reply<T>) -> E,
) -> Waiting<'c, C, E, Ix, T> {
    Waiting { cx, op: Some(make(Reply::new())), waiting: None, _t: PhantomData }
}

/// an operation of `E` in the context: `make` its variant given its reply
#[inline(always)]
pub fn perform<'c, C: Has<E, Ix>, E: Effect, Ix, T: Clone + Debug + 'static>(
    cx: &'c C,
    make: impl FnOnce(Reply<T>) -> E,
) -> Perform<'c, C, E, Ix, T> {
    Perform::new(cx, make(Reply::new()))
}

// THE STANDARD EFFECTS' OPERATIONS — the machine's effects, their enums the same

#[inline(always)]
pub fn ask<V: Clone + std::fmt::Debug + 'static, C: Has<Reader<V>, I>, I>(cx: &C) -> Perform<'_, C, Reader<V>, I, V> {
    perform(cx, Reader::Ask)
}
#[inline(always)]
pub fn get<S: Clone + std::fmt::Debug + 'static, C: Has<State<S>, I>, I>(cx: &C) -> Perform<'_, C, State<S>, I, S> {
    perform(cx, State::Get)
}
#[inline(always)]
pub fn put<S: Clone + std::fmt::Debug + 'static, C: Has<State<S>, I>, I>(cx: &C, s: S) -> Perform<'_, C, State<S>, I, ()> {
    perform(cx, |r| State::Put(s, r))
}
/// the state moved by `f`
pub async fn modify<S: Clone + std::fmt::Debug + 'static, C: Has<State<S>, I>, I>(cx: &C, f: impl FnOnce(S) -> S) {
    let s = get::<S, C, I>(cx).await;
    put::<S, C, I>(cx, f(s)).await
}
#[inline(always)]
pub fn tell<W: Clone + std::fmt::Debug + 'static, C: Has<Writer<W>, I>, I>(cx: &C, w: W) -> Perform<'_, C, Writer<W>, I, ()> {
    perform(cx, |r| Writer::Tell(w, r))
}
/// an error raised: the rest dropped by the handler of `Throws<E>`
pub async fn raise<E: Clone + std::fmt::Debug + 'static, A, C: Has<Throws<E>, I>, I>(cx: &C, e: E) -> A {
    let never: crate::Never = perform(cx, |r| Throws::Raise(e, r)).await;
    match never {}
}

// THEIR HANDLERS

/// each ask answered the same value
pub struct AskWith<V>(pub V);
impl<V: Clone + std::fmt::Debug + 'static> Handles<Reader<V>> for AskWith<V> {
    #[inline(always)]
    fn handle(&self, op: Reader<V>) -> Step<'_> {
        match op {
            Reader::Ask(r) => Step::Now(r.give(self.0.clone())),
        }
    }
}

/// a state in place
pub struct StateCell<S>(pub RefCell<S>);
impl<S> StateCell<S> {
    pub fn new(s: S) -> Self {
        StateCell(RefCell::new(s))
    }
    pub fn take(self) -> S {
        self.0.into_inner()
    }
}
impl<S: Clone + std::fmt::Debug + 'static> Handles<State<S>> for StateCell<S> {
    #[inline(always)]
    fn handle(&self, op: State<S>) -> Step<'_> {
        match op {
            State::Get(r) => Step::Now(r.give(self.0.borrow().clone())),
            State::Put(s, r) => {
                *self.0.borrow_mut() = s;
                Step::Now(r.give(()))
            }
        }
    }
}

/// the told, in order
pub struct Collect<W>(pub RefCell<Vec<W>>);
impl<W> Collect<W> {
    pub fn new() -> Self {
        Collect(RefCell::new(Vec::new()))
    }
    pub fn take(&self) -> Vec<W> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}
impl<W> Default for Collect<W> {
    fn default() -> Self {
        Self::new()
    }
}
impl<W: Clone + std::fmt::Debug + 'static> Handles<Writer<W>> for Collect<W> {
    #[inline(always)]
    fn handle(&self, op: Writer<W>) -> Step<'_> {
        match op {
            Writer::Tell(w, r) => {
                self.0.borrow_mut().push(w);
                Step::Now(r.give(()))
            }
        }
    }
}

// DROPPING THE REST: either — the body's value, or the error raised in it, the body dropped there

/// the handler of `Throws<E>` that `either` puts in the context: the error kept, the rest taken (never given back). In
/// a context itself (`Raised::new()`), past an interceptor raising, with `catch`
pub struct Raised<E>(RefCell<Option<E>>);
impl<E> Raised<E> {
    pub fn new() -> Self {
        Raised(RefCell::new(None))
    }
}
impl<E> Default for Raised<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// a program run under the `Raised` of its context: its value, or the error raised — the program dropped there
pub async fn catch<E, A>(raised: &Raised<E>, program: impl Future<Output = A>) -> Result<A, E> {
    let mut fut = pin!(program);
    std::future::poll_fn(|c| match fut.as_mut().poll(c) {
        Poll::Ready(a) => Poll::Ready(Ok(a)),
        Poll::Pending => match raised.0.borrow_mut().take() {
            Some(e) => Poll::Ready(Err(e)),
            None => Poll::Pending,
        },
    })
    .await
}
impl<E: Clone + std::fmt::Debug + 'static> Handles<Throws<E>> for Raised<E> {
    #[inline(always)]
    fn handle(&self, op: Throws<E>) -> Step<'_> {
        match op {
            Throws::Raise(e, _) => {
                *self.0.borrow_mut() = Some(e);
                Step::Later
            }
        }
    }
}

/// `body` under a handler of `Throws<E>`: its value, or the error raised — the body dropped where it raised
pub async fn either<'c, E, A, C>(cx: &'c C, body: impl AsyncFnOnce(&Cx<Raised<E>, &'c C>) -> A) -> Result<A, E>
where
    E: Clone + std::fmt::Debug + 'static,
{
    let inner = Cx(Raised::new(), cx);
    let mut fut = pin!(body(&inner));
    std::future::poll_fn(|c| match fut.as_mut().poll(c) {
        Poll::Ready(a) => Poll::Ready(Ok(a)),
        Poll::Pending => match inner.0.0.borrow_mut().take() {
            Some(e) => Poll::Ready(Err(e)),
            None => Poll::Pending,
        },
    })
    .await
}

// TAKING THE REST ONCE: a generator — each value emitted to the consumer, the body resumed after it

/// a value emitted
#[derive(Clone, Debug)]
pub enum Emit<V> {
    Value(V, Reply<()>),
}
impl<V: Clone + std::fmt::Debug + 'static> Effect for Emit<V> {
    type Kind = Plain;
}

#[inline(always)]
pub fn emit<V: Clone + std::fmt::Debug + 'static, C: Has<Emit<V>, I>, I>(cx: &C, v: V) -> Perform<'_, C, Emit<V>, I, ()> {
    perform(cx, |r| Emit::Value(v, r))
}

/// the handler `generate` puts in the context: the value emitted kept, the body suspended until it is taken
pub struct Emitted<V> {
    slot: RefCell<Option<V>>,
    taken: Cell<bool>,
}
impl<V: Clone + std::fmt::Debug + 'static> Handles<Emit<V>> for Emitted<V> {
    #[inline(always)]
    fn handle(&self, op: Emit<V>) -> Step<'_> {
        match op {
            Emit::Value(v, _) => {
                *self.slot.borrow_mut() = Some(v);
                self.taken.set(false);
                Step::Later
            }
        }
    }
    #[inline(always)]
    fn resume(&self) -> Option<Answer> {
        if self.taken.get() { Some(Reply::<()>::new().give(())) } else { None }
    }
}

/// every value the body emits, given to `each` as it comes — the body suspended at each emit, resumed after it: its
/// rest taken once, the state machine itself
pub async fn generate<'c, V, A, C>(
    cx: &'c C,
    mut each: impl FnMut(V),
    body: impl AsyncFnOnce(&Cx<Emitted<V>, &'c C>) -> A,
) -> A
where
    V: Clone + std::fmt::Debug + 'static,
{
    let inner = Cx(Emitted { slot: RefCell::new(None), taken: Cell::new(false) }, cx);
    let mut fut = pin!(body(&inner));
    std::future::poll_fn(|c| loop {
        match fut.as_mut().poll(c) {
            Poll::Ready(a) => return Poll::Ready(a),
            Poll::Pending => match inner.0.slot.borrow_mut().take() {
                Some(v) => {
                    each(v);
                    inner.0.taken.set(true);
                }
                None => return Poll::Pending,
            },
        }
    })
    .await
}

// TAKING THE REST MANY TIMES: by replay — the body run again for each path, its past told it again

/// what a path was told, in order: a choice (its index, `among` alternatives), or an answer from further out (`among`
/// 0)
struct Told {
    val: Val,
    among: usize,
}

/// the layer `all` and `first` put in the context: the choices of the path now, and the answers from further out told
/// before the last of them — told again, not asked again, when the body runs again (Koppel et al., "Capturing the
/// future by replaying the past", ICFP 2018)
pub struct Paths<'c, C> {
    told: RefCell<Vec<Told>>,
    at: Cell<usize>,
    failed: Cell<bool>,
    outer: &'c C,
}

impl<'c, C> Paths<'c, C> {
    fn new(outer: &'c C) -> Self {
        Paths { told: RefCell::new(Vec::new()), at: Cell::new(0), failed: Cell::new(false), outer }
    }

    /// the value told here before, if the body is replaying its past
    #[inline]
    fn replayed<T: Clone + 'static>(&self) -> Option<T> {
        let at = self.at.get();
        let told = self.told.borrow();
        let t = told.get(at)?;
        self.at.set(at + 1);
        Some(t.val.peek::<T>().cloned().unwrap_or_else(|| not_replayed()))
    }

    #[inline]
    fn tell<T: Clone + 'static>(&self, t: &T, among: usize) {
        self.told.borrow_mut().push(Told { val: Val::new(t.clone()), among });
        self.at.set(self.at.get() + 1);
    }

    /// the next path: the last choice with an alternative left moved to it, all told after it forgotten; none left —
    /// false
    fn next(&self) -> bool {
        let mut told = self.told.borrow_mut();
        while let Some(t) = told.pop() {
            if t.among > 0 {
                let i = t.val.take::<usize>() + 1;
                if i < t.among {
                    told.push(Told { val: Val::new(i), among: t.among });
                    return true;
                }
            }
        }
        false
    }

    /// the body run along the path told: its value, or none — the path failed
    async fn path<A>(&self, body: &impl AsyncFn(&Self) -> A) -> Option<A> {
        self.at.set(0);
        self.failed.set(false);
        let mut fut = pin!(body(self));
        std::future::poll_fn(|c| match fut.as_mut().poll(c) {
            Poll::Ready(a) => Poll::Ready(Some(a)),
            Poll::Pending if self.failed.get() => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        })
        .await
    }
}

#[cold]
fn not_replayed() -> ! {
    panic!("okay: the body did not replay its past — it decides by something not an effect (a cell, the time)")
}

impl<C> Has<Choose, Here> for Paths<'_, C> {
    #[inline]
    fn perform<T: Clone + Debug + 'static>(&self, op: Choose) -> Got<'_, T> {
        match op {
            Choose::Among(n, r) => {
                let i = match self.replayed::<usize>() {
                    Some(i) => i,
                    None if n == 0 => {
                        self.failed.set(true);
                        return Got::Taken;
                    }
                    None => {
                        self.tell(&0usize, n);
                        0
                    }
                };
                Got::Now(r.give(i).0.take::<T>())
            }
            Choose::Fail(_) => {
                self.failed.set(true);
                Got::Taken
            }
        }
    }
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        None
    }
}

impl<E: Effect, C: Has<E, Ix>, Ix> Has<E, There<Ix>> for Paths<'_, C> {
    #[inline]
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        if let Some(t) = self.replayed::<T>() {
            return Got::Now(t);
        }
        match self.outer.perform::<T>(op) {
            Got::Now(t) => {
                self.tell(&t, 0);
                Got::Now(t)
            }
            Got::Taken => Got::Taken,
            Got::Wait(f) => Got::Wait(Box::pin(async move {
                let t = f.await;
                self.tell(&t, 0);
                t
            })),
        }
    }
    #[inline]
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        let t = self.outer.resume::<T>()?;
        self.tell(&t, 0);
        Some(t)
    }
}

/// one of the values, each in turn
pub async fn among<A: Clone, C: Has<Choose, I>, I>(cx: &C, xs: &[A]) -> A {
    let i: usize = perform(cx, |r| Choose::Among(xs.len(), r)).await;
    xs[i].clone()
}

/// true, then false
pub async fn flip<C: Has<Choose, I>, I>(cx: &C) -> bool {
    let i: usize = perform(cx, |r| Choose::Among(2, r)).await;
    i == 0
}

/// no value: this path dropped
pub async fn fail<A, C: Has<Choose, I>, I>(cx: &C) -> A {
    let never: Never = perform(cx, Choose::Fail).await;
    match never {}
}

/// every value of the body, one per path, in order — the body run again for each, its past told it again: the
/// choices made, the answers from further out (each asked once: a state further out threaded through the paths in
/// order, as the machine's `all` under it). So the body decides by its effects only, and is an `AsyncFn`
pub async fn all<'c, A, C>(cx: &'c C, body: impl AsyncFn(&Paths<'c, C>) -> A) -> Vec<A> {
    let paths = Paths::new(cx);
    let mut out = Vec::new();
    loop {
        if let Some(a) = paths.path(&body).await {
            out.push(a);
        }
        if !paths.next() {
            return out;
        }
    }
}

/// the value of the first path not failing, the rest not run
pub async fn first<'c, A, C>(cx: &'c C, body: impl AsyncFn(&Paths<'c, C>) -> A) -> Option<A> {
    let paths = Paths::new(cx);
    loop {
        if let Some(a) = paths.path(&body).await {
            return Some(a);
        }
        if !paths.next() {
            return None;
        }
    }
}

// RECURSION

/// A LOOP IN CONSTANT STACK: `step` from a state to the next — `Loop::Next(s)` on, `Loop::Done(a)` its value; a
/// recursion, a mutual one (its calls the states: `Call::Even(n)`, `Call::Odd(n)`), at any depth. An async recursion
/// is Rust's own: each call a future inside its caller's (boxed), polled through all of them — deep, it overflows
pub async fn tail_rec<S, A>(s0: S, step: impl AsyncFn(S) -> Loop<S, A>) -> A {
    let mut s = s0;
    loop {
        match step(s).await {
            Loop::Next(next) => s = next,
            Loop::Done(a) => return a,
        }
    }
}

// EVERY OPERATION OF EVERY EFFECT: a watch between the body and its handlers

/// sees each operation on its way out — its effect (`Reader<i64>`), the operation as written (`Put(6, _)`) — lets it
/// through or refuses it (the body dropped there), then sees its answer
pub trait Watch {
    fn before(&self, effect: &str, op: &dyn Debug) -> Result<(), String> {
        let _ = (effect, op);
        Ok(())
    }
    fn after(&self, effect: &str, op: &dyn Debug, answer: &dyn Debug) {
        let _ = (effect, op, answer);
    }
}

impl<W: Watch> Watch for &W {
    fn before(&self, effect: &str, op: &dyn Debug) -> Result<(), String> {
        (**self).before(effect, op)
    }
    fn after(&self, effect: &str, op: &dyn Debug, answer: &dyn Debug) {
        (**self).after(effect, op, answer)
    }
}

/// an effect's type name, its paths dropped: a slice of it when its arguments have none (`Pay`, `Reader<i64>`), else
/// made (`Writer<String>` of `okay::effects::Writer<alloc::string::String>`)
#[inline]
fn effect_name(full: &'static str) -> std::borrow::Cow<'static, str> {
    let generic = full.find('<').unwrap_or(full.len());
    if full[generic..].contains("::") {
        return std::borrow::Cow::Owned(short(full));
    }
    let start = full[..generic].rfind("::").map_or(0, |i| i + 2);
    std::borrow::Cow::Borrowed(&full[start..])
}

/// the layer `observe` puts in the context: no handler of its own, each effect at the index it has further out
pub struct Observed<'c, W, C> {
    watch: W,
    refused: RefCell<Option<String>>,
    waiting: RefCell<Option<(String, Box<dyn Debug>)>>,
    outer: &'c C,
}

impl<E: Effect, Ix, W: Watch, C: Has<E, Ix>> Has<E, Ix> for Observed<'_, W, C> {
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        let effect = effect_name(std::any::type_name::<E>());
        if let Err(why) = self.watch.before(&effect, &op) {
            *self.refused.borrow_mut() = Some(why);
            return Got::Taken;
        }
        let seen = op.clone();
        match self.outer.perform::<T>(op) {
            Got::Now(t) => {
                self.watch.after(&effect, &seen, &t);
                Got::Now(t)
            }
            Got::Taken => {
                *self.waiting.borrow_mut() = Some((effect.into_owned(), Box::new(seen)));
                Got::Taken
            }
            Got::Wait(f) => Got::Wait(Box::pin(async move {
                let t = f.await;
                self.watch.after(&effect, &seen, &t);
                t
            })),
        }
    }
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        let t = self.outer.resume::<T>()?;
        if let Some((effect, seen)) = self.waiting.borrow_mut().take() {
            self.watch.after(&effect, &*seen, &t);
        }
        Some(t)
    }
}

/// `body` with every operation of every effect seen by `watch` on its way out: its value, or why the watch refused
/// an operation — the body dropped there
pub async fn observe<'c, W: Watch, A, C>(
    cx: &'c C,
    watch: W,
    body: impl AsyncFnOnce(&Observed<'c, W, C>) -> A,
) -> Result<A, String> {
    let inner = Observed { watch, refused: RefCell::new(None), waiting: RefCell::new(None), outer: cx };
    let mut fut = pin!(body(&inner));
    std::future::poll_fn(|c| match fut.as_mut().poll(c) {
        Poll::Ready(a) => Poll::Ready(Ok(a)),
        Poll::Pending => match inner.refused.borrow_mut().take() {
            Some(why) => Poll::Ready(Err(why)),
            None => Poll::Pending,
        },
    })
    .await
}

// THE TRAIL: `Audit`'s records kept, in order — what interceptors and watches write

/// a line of the trail
#[inline(always)]
pub fn record<C: Has<Audit, I>, I>(cx: &C, line: String) -> Perform<'_, C, Audit, I, ()> {
    perform(cx, |r| Audit::Record(line, r))
}

/// the lines recorded, in order
pub struct Trail(RefCell<Vec<String>>);
impl Trail {
    pub fn new() -> Self {
        Trail(RefCell::new(Vec::new()))
    }
    /// a line, kept straight — by a watch
    pub fn push(&self, line: String) {
        self.0.borrow_mut().push(line);
    }
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}
impl Default for Trail {
    fn default() -> Self {
        Self::new()
    }
}
impl Handles<Audit> for Trail {
    #[inline]
    fn handle(&self, op: Audit) -> Step<'_> {
        match op {
            Audit::Record(line, r) => {
                self.0.borrow_mut().push(line);
                Step::Now(r.give(()))
            }
        }
    }
}

// RUNNING

/// why a run did not end: an operation taken by a handler that never gives it back, with none outside to drop it
#[derive(Debug, PartialEq, Eq)]
pub struct Stuck;

/// A PROGRAM RUN TO ITS VALUE, WAITING ON THE WORLD: polled, the thread parked until a handler's future wakes it — an
/// executor of one program, no dependencies. Under another (tokio's `LocalSet`, …) a program is a future as any.
/// One that never wakes — an operation taken, never given back — waits for ever: `run` says `Stuck` instead
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut fut = pin!(fut);
    let waker = Waker::from(std::sync::Arc::new(Unpark(std::thread::current())));
    let mut c = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(a) = fut.as_mut().poll(&mut c) {
            return a;
        }
        std::thread::park();
    }
}

/// two programs at once, each going on while the other waits: both values. Their effects' handlers shared; one taking
/// the rest (`Later`: either, generate, the replay of `all`) wants one program at a time
pub async fn both<A, B>(a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut ra, mut rb) = (None, None);
    std::future::poll_fn(|c| {
        if ra.is_none()
            && let Poll::Ready(x) = a.as_mut().poll(c)
        {
            ra = Some(x);
        }
        if rb.is_none()
            && let Poll::Ready(y) = b.as_mut().poll(c)
        {
            rb = Some(y);
        }
        match (ra.take(), rb.take()) {
            (Some(x), Some(y)) => Poll::Ready((x, y)),
            (x, y) => {
                ra = x;
                rb = y;
                Poll::Pending
            }
        }
    })
    .await
}

/// an operation (a program) done now, where no `.await` is — in an interceptor: its value, or none — taken
pub fn now<F: Future>(fut: F) -> Option<F::Output> {
    run(fut).ok()
}

/// a program run to its value — every handler of its context in force
pub fn run<F: Future>(fut: F) -> Result<F::Output, Stuck> {
    let mut fut = pin!(fut);
    let mut c = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut c) {
        Poll::Ready(a) => Ok(a),
        Poll::Pending => Err(Stuck),
    }
}
