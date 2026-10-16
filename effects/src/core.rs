//! Operations, capabilities, handlers: the core, on async/await.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::{poll_fn, Future};
use std::marker::PhantomData;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// An operation answering `Answer`.
pub trait Op: 'static {
    type Answer: 'static;
}

/// The one operation in flight in a run, between the body that performed it and
/// the handler that owns its capability.
pub(crate) struct Pending {
    pub cap: u64,
    pub op: Option<Box<dyn Any>>,
    pub answer: Option<Box<dyn Any>>,
    pub raised: Option<Box<dyn Any + Send>>,
}

impl Pending {
    pub fn asked(&self) -> bool {
        self.answer.is_none() && self.raised.is_none()
    }
}

#[derive(Default)]
pub(crate) struct RunInner {
    pub mailbox: RefCell<Option<Pending>>,
    next: Cell<u64>,
}

/// One program and its handlers, on one thread.
#[derive(Clone, Default)]
pub struct Run(pub(crate) Rc<RunInner>);

impl Run {
    pub fn new() -> Run {
        Run::default()
    }
    pub(crate) fn fresh(&self) -> u64 {
        let n = self.0.next.get() + 1;
        self.0.next.set(n);
        n
    }
}

/// A capability: made by a handler, handed to its body; the body performs through it.
pub struct Cap<O: Op> {
    pub(crate) id: u64,
    pub(crate) live: Rc<Cell<bool>>,
    run: Run,
    _op: PhantomData<fn(O)>,
}

impl<O: Op> Clone for Cap<O> {
    fn clone(&self) -> Self {
        Cap { id: self.id, live: self.live.clone(), run: self.run.clone(), _op: PhantomData }
    }
}

impl<O: Op> std::fmt::Debug for Cap<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cap<{}>#{}", short::<O>(), self.id)
    }
}

pub(crate) fn short<T>() -> &'static str {
    let n = std::any::type_name::<T>();
    n.rsplit("::").next().unwrap_or(n)
}

impl<O: Op> Cap<O> {
    pub(crate) fn new(r: &Run) -> Cap<O> {
        Cap { id: r.fresh(), live: Rc::new(Cell::new(true)), run: r.clone(), _op: PhantomData }
    }

    /// `cap.perform(op).await`: suspends until the owner answers.
    pub async fn perform(&self, op: O) -> O::Answer {
        if !self.live.get() {
            panic!("okay: {:?} used outside its handler — the capability escaped its scope", self);
        }
        *self.run.0.mailbox.borrow_mut() = Some(Pending { cap: self.id, op: Some(Box::new(op)), answer: None, raised: None });
        let p = poll_fn(|_| {
            let mut mb = self.run.0.mailbox.borrow_mut();
            match mb.as_ref() {
                Some(p) if p.cap == self.id && !p.asked() => Poll::Ready(mb.take().unwrap()),
                _ => Poll::Pending,
            }
        })
        .await;
        if let Some(e) = p.raised {
            std::panic::resume_unwind(e); // raised where it was performed; destructors run
        }
        match p.answer.unwrap().downcast::<O::Answer>() {
            Ok(a) => *a,
            Err(_) => panic!("okay: {:?} answered with something other than {}", self, short::<O::Answer>()),
        }
    }
}

/// What a handler does with an operation.
pub enum Reply<R, A> {
    /// the body resumes with this answer
    Resume(R),
    /// the body is dropped where it performed (its destructors run) and the
    /// handler's value is this — never resumed (the kernel's Aborting)
    Abort(A),
    /// a panic with this payload where the operation was performed
    Raise(Box<dyn Any + Send>),
}

pub(crate) struct Dead(pub(crate) Rc<Cell<bool>>);
impl Drop for Dead {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// once Pending, then Ready: hand control to whoever polls this future
async fn yield_once() {
    let mut yielded = false;
    poll_fn(|_| if yielded { Poll::Ready(()) } else { yielded = true; Poll::Pending }).await
}

/// Make a capability for `O`, run `body` with it, answer each of its operations
/// with `answer` — itself async, so it may perform through capabilities from
/// outside (a component built on another). Operations of other capabilities,
/// and real async waits, pass outward unchanged.
pub async fn handle<O, A, H, HF, F, Fut>(r: &Run, mut answer: H, body: F) -> A
where
    O: Op,
    H: FnMut(O) -> HF,
    HF: Future<Output = Reply<O::Answer, A>>,
    F: FnOnce(Cap<O>) -> Fut,
    Fut: Future<Output = A>,
{
    let cap: Cap<O> = Cap::new(r);
    let id = cap.id;
    let _dead = Dead(cap.live.clone());
    let mut fut = pin!(body(cap));
    loop {
        if let Poll::Ready(a) = poll_fn(|cx| Poll::Ready(fut.as_mut().poll(cx))).await {
            return a;
        }
        let mine = matches!(r.0.mailbox.borrow().as_ref(), Some(p) if p.cap == id && p.asked());
        if !mine {
            yield_once().await; // not mine: outward, and back in once answered
            continue;
        }
        // taken out whole: the answer may perform through outer capabilities meanwhile
        let mut p = r.0.mailbox.borrow_mut().take().unwrap();
        let op = *p.op.take().unwrap().downcast::<O>().unwrap_or_else(|_| unreachable!("a Cap<O> carries an O"));
        match answer(op).await {
            Reply::Resume(x) => p.answer = Some(Box::new(x)),
            Reply::Raise(e) => p.raised = Some(e),
            Reply::Abort(a) => return a, // the body is dropped here: its destructors run
        }
        *r.0.mailbox.borrow_mut() = Some(p);
    }
}

/// `handle` for a plain answer: `f` answers each operation (Answering).
pub async fn answering<O, A, F, Fut>(r: &Run, mut f: impl FnMut(O) -> O::Answer, body: F) -> A
where
    O: Op,
    F: FnOnce(Cap<O>) -> Fut,
    Fut: Future<Output = A>,
{
    handle(r, |op| std::future::ready(Reply::Resume(f(op))), body).await
}

pub(crate) struct Unpark(pub(crate) std::thread::Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Run a program whose every operation is handled inside it; a real async wait
/// (a timer, a socket) parks the thread until it is woken.
pub fn run<A, F, Fut>(body: F) -> A
where
    F: FnOnce(Run) -> Fut,
    Fut: Future<Output = A>,
{
    let r = Run::new();
    let mut fut = pin!(body(r.clone()));
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(a) = fut.as_mut().poll(&mut cx) {
            return a;
        }
        if let Some(p) = r.0.mailbox.borrow().as_ref() {
            if p.asked() {
                panic!("okay: an operation (capability #{}) no live handler owns", p.cap);
            }
        }
        std::thread::park();
    }
}
