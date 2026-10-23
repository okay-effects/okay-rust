//! A CONTEXT BUILT AT RUN TIME: handlers nested to any depth, in an order known only when the program runs — an
//! interpreter of scripts, okay-kernel's TCK — where a typed context (`Cx<…>`, a static type) cannot follow. `Dyn`
//! is a context like any (`Has<E>` for every effect: the same `eff!`/`needs!` programs run under it); behind it a chain
//! of layers, each a `dyn Layer`: a handler of one effect, found by its `TypeId`, the others passed further out. An
//! operation crosses boxed, a call through a pointer per layer — the price of a shape decided at run time.
//!
//! ```
//! use okay::dynamic::{root, With};
//! use okay::effects::{Reader, Writer};
//! use okay::typed::*;
//!
//! okay::needs! {
//!     async fn greet(cx: [Reader<i64>, Writer<String>]) -> i64 {
//!         let n = ask(cx).await;
//!         tell(cx, format!("n {n}")).await;
//!         n
//!     }
//! }
//! // the same program, its handlers nested as a configuration says — here, two readers, the nearer answering
//! let log = With::new(root(), Collect::<String>::new());
//! let outer = With::new(log.cx(), AskWith(1i64));
//! let inner = With::new(outer.cx(), AskWith(2i64));
//! assert_eq!(run(greet(&inner.cx())), Ok(2));
//! assert_eq!(log.handler().take(), vec!["n 2".to_string()]);
//! ```
//!
//! A sensitive effect's handler is a layer only under its interceptor (`With::audited`):
//!
//! ```compile_fail
//! use okay::dynamic::{root, With};
//! use okay::typed::*;
//! okay::effect! { enum Pay: Sensitive { Transfer(n: i64) -> String = transfer; } }
//! struct Bank;
//! impl Handles<Pay> for Bank {
//!     fn handle(&self, op: Pay) -> Step<'_> {
//!         match op { Pay::Transfer(n, r) => Step::Now(r.give(format!("ok {n}"))) }
//!     }
//! }
//! let bank = With::new(root(), Bank);
//! ```
use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use crate::eff::Answer;
use crate::effects::{Choose, Throws};
use crate::row::{Effect, Here};
use crate::typed::{Allowed, Audited, Got, Handles, Has, Interceptor, Pass, Raised, Step};
use crate::val::Val;

/// how an answer is copied — made where its type is known (the program's operation), carried with the operation, so a
/// layer replaying the past (`all`) keeps a copy of an answer it cannot name
pub type Cloner = fn(&Answer) -> Answer;

fn clone_answer<T: Clone + 'static>(a: &Answer) -> Answer {
    Answer(Val::new(a.0.peek::<T>().expect("an answer of its operation's type").clone()))
}

/// AN OPERATION ON ITS WAY THROUGH THE LAYERS: its effect, the operation boxed, how its answer is copied
pub struct AnyOp {
    effect: TypeId,
    name: &'static str,
    op: Val,
    clone: Cloner,
}

impl AnyOp {
    /// the operation, if it is one of `E`'s
    pub fn take<E: Effect>(self) -> Result<E, AnyOp> {
        if self.effect == TypeId::of::<E>() {
            Ok(self.op.take::<E>())
        } else {
            Err(self)
        }
    }
    /// its effect's type name
    pub fn effect(&self) -> &'static str {
        self.name
    }
}

/// A LAYER OF A DYNAMIC CONTEXT: an operation answered (now, later — taken —, or when a future is done) or passed
/// further out; an operation taken given back
pub trait Layer {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer>;
    /// the operation of `effect` this layer, or one further out, took: its answer, if given back
    fn resume(&self, effect: TypeId) -> Option<Answer>;
    /// the nearest replay (`all`, `first`) this layer is inside of
    fn replay(&self) -> Option<&Paths<'_>>;
}

/// A CONTEXT BUILT AT RUN TIME: the innermost of its layers
#[derive(Clone, Copy)]
pub struct Dyn<'a>(&'a dyn Layer);

impl<'a> Dyn<'a> {
    /// a context over a layer
    pub fn new(layer: &'a dyn Layer) -> Self {
        Dyn(layer)
    }
}

/// the outermost layer: no handler — an operation reaching it has none
struct Root;
impl Layer for Root {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer> {
        panic!("okay: no handler of `{}` in the dynamic context", op.name)
    }
    fn resume(&self, _: TypeId) -> Option<Answer> {
        None
    }
    fn replay(&self) -> Option<&Paths<'_>> {
        None
    }
}

/// the empty dynamic context, the outermost
pub fn root() -> Dyn<'static> {
    Dyn(&Root)
}

fn got_of(step: Step<'_>) -> Got<'_, Answer> {
    match step {
        Step::Now(a) => Got::Now(a),
        Step::Later => Got::Taken,
        Step::Wait(f) => Got::Wait(f),
    }
}

impl<E: Effect> Has<E, Here> for Dyn<'_> {
    fn perform<T: Clone + Debug + 'static>(&self, op: E) -> Got<'_, T> {
        let op = AnyOp {
            effect: TypeId::of::<E>(),
            name: std::any::type_name::<E>(),
            op: Val::new(op),
            clone: clone_answer::<T>,
        };
        match self.0.perform(op) {
            Got::Now(a) => Got::Now(a.0.take::<T>()),
            Got::Taken => Got::Taken,
            Got::Wait(f) => Got::Wait(Box::pin(async move { f.await.0.take::<T>() })),
        }
    }
    fn resume<T: Clone + Debug + 'static>(&self) -> Option<T> {
        self.0.resume(TypeId::of::<E>()).map(|a| a.0.take::<T>())
    }
}

/// A HANDLER OF `E` AS A LAYER over a dynamic context: its operations answered by it, the others passed out
pub struct With<'o, E, H> {
    handler: H,
    outer: Dyn<'o>,
    _e: PhantomData<fn(E)>,
}

impl<'o, E: Effect, H: Handles<E>> With<'o, E, H> {
    /// `handler` over `outer` — a plain effect's; a sensitive one's only under its interceptor (`audited`)
    pub fn new(outer: Dyn<'o>, handler: H) -> Self
    where
        H: Allowed<E::Kind>,
    {
        With { handler, outer, _e: PhantomData }
    }
    /// the context with this layer innermost
    pub fn cx(&self) -> Dyn<'_> {
        Dyn(self)
    }
    /// its handler — its state seen
    pub fn handler(&self) -> &H {
        &self.handler
    }
    pub fn into_handler(self) -> H {
        self.handler
    }
}

impl<E: Effect, H: Handles<E>> Layer for With<'_, E, H> {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer> {
        match op.take::<E>() {
            Ok(e) => got_of(self.handler.handle(e)),
            Err(op) => self.outer.0.perform(op),
        }
    }
    fn resume(&self, effect: TypeId) -> Option<Answer> {
        if effect == TypeId::of::<E>() { self.handler.resume() } else { self.outer.0.resume(effect) }
    }
    fn replay(&self) -> Option<&Paths<'_>> {
        self.outer.0.replay()
    }
}

/// A HANDLER UNDER ITS INTERCEPTOR AS A LAYER: the interceptor's own operations to the context outside
pub struct AuditedLayer<'o, E, H, I, Ix = ()> {
    audited: Audited<H, I>,
    outer: Dyn<'o>,
    _e: PhantomData<fn(E, Ix)>,
}

impl<'o, E: Effect, H: Handles<E>, I: for<'x> Interceptor<E, Dyn<'x>, Ix>, Ix> AuditedLayer<'o, E, H, I, Ix> {
    pub fn cx(&self) -> Dyn<'_> {
        Dyn(self)
    }
    pub fn audited(&self) -> &Audited<H, I> {
        &self.audited
    }
}

impl<'o, E: Effect, H: Handles<E>> With<'o, E, H> {
    /// a handler under its interceptor over `outer` — the only way a sensitive effect is a layer. The interceptor's
    /// own operations go to `outer` (at `Here`: a dynamic context has every effect there), or none (`()`)
    pub fn audited<I: for<'x> Interceptor<E, Dyn<'x>, Ix>, Ix>(
        outer: Dyn<'o>,
        audited: Audited<H, I>,
    ) -> AuditedLayer<'o, E, H, I, Ix> {
        AuditedLayer { audited, outer, _e: PhantomData }
    }
}

impl<E: Effect, H: Handles<E>, I: for<'x> Interceptor<E, Dyn<'x>, Ix>, Ix> Layer for AuditedLayer<'_, E, H, I, Ix> {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer> {
        match op.take::<E>() {
            Ok(e) => got_of(self.audited.interceptor.intercept(e, Pass(&self.audited.handler), &self.outer)),
            Err(op) => self.outer.0.perform(op),
        }
    }
    fn resume(&self, effect: TypeId) -> Option<Answer> {
        if effect == TypeId::of::<E>() { self.audited.handler.resume() } else { self.outer.0.resume(effect) }
    }
    fn replay(&self) -> Option<&Paths<'_>> {
        self.outer.0.replay()
    }
}

// DROPPING THE REST: either over a dynamic context

/// `body` under a handler of `Throws<X>` put innermost: its value, or the error raised — the body dropped there
pub async fn either<'o, X, A>(cx: Dyn<'o>, body: impl AsyncFnOnce(Dyn<'_>) -> A) -> Result<A, X>
where
    X: Clone + Debug + 'static,
{
    let layer: With<'o, Throws<X>, Raised<X>> = With::new(cx, Raised::new());
    let mut fut = pin!(body(layer.cx()));
    std::future::poll_fn(|c| match fut.as_mut().poll(c) {
        Poll::Ready(a) => Poll::Ready(Ok(a)),
        Poll::Pending => match layer.handler.0.borrow_mut().take() {
            Some(e) => Poll::Ready(Err(e)),
            None => Poll::Pending,
        },
    })
    .await
}

// TAKING THE REST MANY TIMES over a dynamic context: by replay, as `typed::all` — the answers from further out
// copied by the cloner each operation carries

enum Told {
    Choice(usize, usize),
    Answer(Answer),
}

/// the layer `all` puts innermost: the choices of the path now, the answers from further out told before the last of
/// them — told again, not asked again, when the body runs again
pub struct Paths<'o> {
    told: RefCell<Vec<Told>>,
    at: Cell<usize>,
    failed: Cell<bool>,
    taken: Cell<Option<Cloner>>,
    /// the shared handlers made, each with the place in the past it was made at; the next to be found again
    made: RefCell<Vec<(usize, Rc<dyn Any>)>>,
    made_at: Cell<usize>,
    outer: Dyn<'o>,
}

impl<'o> Paths<'o> {
    fn new(outer: Dyn<'o>) -> Self {
        Paths {
            told: RefCell::new(Vec::new()),
            at: Cell::new(0),
            failed: Cell::new(false),
            taken: Cell::new(None),
            made: RefCell::new(Vec::new()),
            made_at: Cell::new(0),
            outer,
        }
    }

    fn tell(&self, t: Told) {
        self.told.borrow_mut().push(t);
        self.at.set(self.at.get() + 1);
    }

    fn next(&self) -> bool {
        let mut told = self.told.borrow_mut();
        while let Some(t) = told.pop() {
            if let Told::Choice(i, among) = t
                && i + 1 < among
            {
                told.push(Told::Choice(i + 1, among));
                // the shared handlers made after the choice are the old path's: made again on the new one
                let keep = told.len();
                self.made.borrow_mut().retain(|(at, _)| *at < keep);
                return true;
            }
        }
        false
    }

    /// the past is being replayed here: the next operation's answer was told before
    fn replaying(&self) -> bool {
        self.at.get() < self.told.borrow().len()
    }

    /// a shared handler: the one made at this place on an earlier path — the same object, as the rest of the
    /// machine's `k` holds it — or one made now
    fn shared<H: 'static>(&self, make: impl FnOnce() -> H) -> Rc<H> {
        let i = self.made_at.get();
        self.made_at.set(i + 1);
        if let Some((_, h)) = self.made.borrow().get(i) {
            return h.clone().downcast::<H>().unwrap_or_else(|_| not_replayed());
        }
        let h = Rc::new(make());
        self.made.borrow_mut().push((self.told.borrow().len(), h.clone()));
        h
    }

    async fn path<A>(&self, body: &impl AsyncFn(Dyn<'_>) -> A) -> Option<A> {
        self.at.set(0);
        self.made_at.set(0);
        self.failed.set(false);
        let mut fut = pin!(body(Dyn(self)));
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

impl Layer for Paths<'_> {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer> {
        let at = self.at.get();
        let clone = op.clone;
        match op.take::<Choose>() {
            Ok(Choose::Among(n, r)) => {
                let i = match self.told.borrow().get(at) {
                    Some(Told::Choice(i, _)) => Some(*i),
                    Some(Told::Answer(_)) => not_replayed(),
                    None => None,
                };
                let i = match i {
                    Some(i) => {
                        self.at.set(at + 1);
                        i
                    }
                    None if n == 0 => {
                        self.failed.set(true);
                        return Got::Taken;
                    }
                    None => {
                        self.tell(Told::Choice(0, n));
                        0
                    }
                };
                Got::Now(r.give(i))
            }
            Ok(Choose::Fail(_)) => {
                self.failed.set(true);
                Got::Taken
            }
            Err(op) => {
                let replayed = match self.told.borrow().get(at) {
                    Some(Told::Answer(a)) => Some(clone(a)),
                    Some(Told::Choice(..)) => not_replayed(),
                    None => None,
                };
                if let Some(a) = replayed {
                    self.at.set(at + 1);
                    return Got::Now(a);
                }
                match self.outer.0.perform(op) {
                    Got::Now(a) => {
                        self.tell(Told::Answer(clone(&a)));
                        Got::Now(a)
                    }
                    Got::Taken => {
                        self.taken.set(Some(clone));
                        Got::Taken
                    }
                    Got::Wait(f) => Got::Wait(Box::pin(async move {
                        let a = f.await;
                        self.tell(Told::Answer(clone(&a)));
                        a
                    })),
                }
            }
        }
    }
    fn resume(&self, effect: TypeId) -> Option<Answer> {
        let a = self.outer.0.resume(effect)?;
        if let Some(clone) = self.taken.take() {
            self.tell(Told::Answer(clone(&a)));
        }
        Some(a)
    }
    fn replay(&self) -> Option<&Paths<'_>> {
        Some(self)
    }
}

// A SHARED HANDLER: one object for every path of the replay around it, as one level of the machine's `k` is for every
// resumption of it — a cell all of them see

/// A HANDLER SHARED BY THE PATHS of the replay it is inside: made once at its place (found again on each later path),
/// its operations in the past told again, not done again — so it holds what the last path left, as a handler in a
/// `k` resumed many times does. Outside any replay, a layer as `With`
pub struct Shared<'o, E, H> {
    handler: Rc<H>,
    outer: Dyn<'o>,
    _e: PhantomData<fn(E)>,
}

impl<'o, E: Effect, H: Handles<E> + Allowed<E::Kind> + 'static> Shared<'o, E, H> {
    pub fn new(outer: Dyn<'o>, make: impl FnOnce() -> H) -> Self {
        let handler = match outer.0.replay() {
            Some(paths) => paths.shared(make),
            None => Rc::new(make()),
        };
        Shared { handler, outer, _e: PhantomData }
    }
    pub fn cx(&self) -> Dyn<'_> {
        Dyn(self)
    }
    pub fn handler(&self) -> &H {
        &self.handler
    }
}

impl<E: Effect, H: Handles<E>> Layer for Shared<'_, E, H> {
    fn perform(&self, op: AnyOp) -> Got<'_, Answer> {
        let clone = op.clone;
        match op.take::<E>() {
            Ok(e) => match self.outer.0.replay() {
                Some(paths) if paths.replaying() => {
                    let at = paths.at.get();
                    let a = match paths.told.borrow().get(at) {
                        Some(Told::Answer(a)) => clone(a),
                        _ => not_replayed(),
                    };
                    paths.at.set(at + 1);
                    Got::Now(a)
                }
                Some(paths) => match self.handler.handle(e) {
                    Step::Now(a) => {
                        paths.tell(Told::Answer(clone(&a)));
                        Got::Now(a)
                    }
                    other => got_of(other),
                },
                None => got_of(self.handler.handle(e)),
            },
            Err(op) => self.outer.0.perform(op),
        }
    }
    fn resume(&self, effect: TypeId) -> Option<Answer> {
        if effect == TypeId::of::<E>() { self.handler.resume() } else { self.outer.0.resume(effect) }
    }
    fn replay(&self) -> Option<&Paths<'_>> {
        self.outer.0.replay()
    }
}

/// every value of the body, one per path, in order — the body run again for each, its past told it again
pub async fn all<A>(cx: Dyn<'_>, body: impl AsyncFn(Dyn<'_>) -> A) -> Vec<A> {
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
pub async fn first<A>(cx: Dyn<'_>, body: impl AsyncFn(Dyn<'_>) -> A) -> Option<A> {
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

