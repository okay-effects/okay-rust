//! A TYPED PROGRAM HANDED OUT TO A HOST (okay-kernel's protocol): the program an `eff!` body over a context in which
//! some effects are `Outside` — their operations go to whoever steps the program, as data (a name, a record) and typed
//! (`Asked::get`). The host answers each (`resume`, `resume_data`), fails it (a panic where it was performed), or
//! cancels the program (dropped, its destructors run). Around the host's answers, MIDDLEWARE over every operation as
//! data — audit, metrics, security, a record, its replay; and okay's WIRE (`remote`). The same program runs in
//! process under handlers of its effects, or handed out: only its context differs.
//!
//! ```
//! use okay::host::{start, Step};
//! use okay::typed::*;
//!
//! okay::effect! { pub enum Ask: Plain, wire { Ask(n: i64) -> i64 = ask_n; } }
//!
//! let mut c = start(|host| async move {
//!     let cx = Cx(host.outside::<Ask>(), End);
//!     ask_n(&cx, 1).await + ask_n(&cx, 2).await
//! });
//! let mut s = c.step();
//! while let Step::Yielded(asked) = s {
//!     assert_eq!(asked.name(), "ask_n");
//!     let Ask::Ask(n, _) = asked.get::<Ask>().unwrap();
//!     s = c.resume(n * 10);
//! }
//! assert!(matches!(s, Step::Done(30)));
//! ```
//!
//! A sensitive effect is outside only under its audit, as any handler of it: `Audited::new(host.outside(), guard)`.
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use crate::data::Data;
use crate::eff::{Answer, Reply};
use crate::row::Effect;
use crate::typed::{Handles, Step as Handled};
use crate::wire::{AnswerOut, Remote};

thread_local! {
    /// an operation is out with a host, not answered yet: branches (`join`, `race`) poll only the one that asked
    static OUT: Cell<bool> = const { Cell::new(false) };
}

/// whether an operation is out with a host now — the branches' lock
pub(crate) fn out() -> bool {
    OUT.with(|o| o.get())
}

/// AN OPERATION THE PROGRAM ASKS ITS HOST: typed (`get`), or as data (`name`, `record`)
#[derive(Clone)]
pub struct Asked {
    op: Rc<dyn Any>,
    effect: &'static str,
    name: String,
    record: Data,
    decode: fn(&dyn Any, &Data) -> Result<Answer, String>,
    out: AnswerOut,
}

fn decode<E: Remote>(op: &dyn Any, d: &Data) -> Result<Answer, String> {
    op.downcast_ref::<E>().expect("an Asked carries its effect's operation").answer(d)
}

impl Asked {
    fn of<E: Remote>(op: E) -> Asked {
        let (name, record) = op.to_wire();
        let out = op.out();
        Asked { op: Rc::new(op), effect: std::any::type_name::<E>(), name, record, decode: decode::<E>, out }
    }
    /// the operation, if it is one of `E`'s
    pub fn get<E: 'static>(&self) -> Option<&E> {
        self.op.downcast_ref::<E>()
    }
    /// its name on the wire: its function's (`ask_n`)
    pub fn name(&self) -> &str {
        &self.name
    }
    /// its arguments, a record (keys in lowerCamel)
    pub fn record(&self) -> &Data {
        &self.record
    }
    /// its effect's type
    pub fn effect(&self) -> &'static str {
        self.effect
    }
    /// an answer given by a typed handler (`r.give(v)`), as the plain value it is on the wire
    pub fn out(&self, a: Answer) -> Data {
        (self.out)(a)
    }
}

impl Debug for Asked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:?}", self.name, self.record)
    }
}

/// between a program and its host: the operation out, and its answer — or the failure to raise where it was performed
#[derive(Default)]
struct Slot {
    asked: RefCell<Option<Asked>>,
    answer: RefCell<Option<Result<Answer, String>>>,
}

/// THE PROGRAM'S HOST: the handlers of the effects it leaves to whoever steps it
#[derive(Clone)]
pub struct Host(Rc<Slot>);

impl Host {
    /// a handler of `E` whose operations go to the host
    pub fn outside<E: Remote>(&self) -> Outside<E> {
        Outside(self.0.clone(), PhantomData)
    }
}

/// THE HANDLER OF AN EFFECT LEFT TO THE HOST: each operation out as `Asked`, the program suspended until the host
/// answers it
pub struct Outside<E>(Rc<Slot>, PhantomData<fn(E)>);

impl<E: Remote> Handles<E> for Outside<E> {
    fn handle(&self, op: E) -> Handled<'_> {
        *self.0.asked.borrow_mut() = Some(Asked::of(op));
        OUT.with(|o| o.set(true));
        Handled::Later
    }
    fn resume(&self) -> Option<Answer> {
        let a = self.0.answer.borrow_mut().take()?;
        OUT.with(|o| o.set(false));
        match a {
            Ok(a) => Some(a),
            // raised where the operation was performed: the program's destructors run as it unwinds
            Err(why) => std::panic::resume_unwind(Box::new(why)),
        }
    }
}

/// ONE STOP of a handed-out program
#[derive(Debug)]
pub enum Step<A> {
    Done(A),
    Yielded(Asked),
    Failed(String),
    Cancelled,
}

struct Unpark(std::thread::Thread);
impl std::task::Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// A PROGRAM HANDED OUT, run one step at a time
pub struct Computation<'a, A> {
    slot: Rc<Slot>,
    fut: Option<Pin<Box<dyn Future<Output = A> + 'a>>>,
    asked: Option<Asked>,
}

/// `body` handed out: nothing runs until the first `step()`. It makes its context, with `host.outside::<E>()` for
/// each effect the host answers
pub fn start<'a, A, F, Fut>(body: F) -> Computation<'a, A>
where
    F: FnOnce(Host) -> Fut,
    Fut: Future<Output = A> + 'a,
{
    let slot = Rc::new(Slot::default());
    let fut: Pin<Box<dyn Future<Output = A> + 'a>> = Box::pin(body(Host(slot.clone())));
    Computation { slot, fut: Some(fut), asked: None }
}

fn message(p: Box<dyn Any + Send>) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "a panic".into())
}

impl<A> Computation<'_, A> {
    /// run to the value, the failure, or the next operation for the host; a real wait inside (a timer) parks the
    /// thread until it wakes
    pub fn step(&mut self) -> Step<A> {
        let Some(fut) = self.fut.as_mut() else { panic!("okay: a finished computation stepped again") };
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let before = OUT.with(|o| o.replace(false));
        let s = loop {
            match catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(&mut cx))) {
                Ok(Poll::Ready(a)) => break Step::Done(a),
                Err(p) => break Step::Failed(message(p)),
                Ok(Poll::Pending) => {}
            }
            if let Some(a) = self.slot.asked.borrow_mut().take() {
                self.asked = Some(a.clone());
                break Step::Yielded(a);
            }
            std::thread::park();
        };
        OUT.with(|o| o.set(before));
        if !matches!(s, Step::Yielded(_)) {
            self.finish();
        }
        s
    }

    /// the operation answered with `v`, of the type it answers; on to the next stop
    pub fn resume<T: 'static>(&mut self, v: T) -> Step<A> {
        self.answer(Ok(Reply::<T>::new().give(v)))
    }

    /// the operation answered with a plain value, decoded into what it answers
    pub fn resume_data(&mut self, d: &Data) -> Step<A> {
        let a = self.asked.as_ref().expect("okay: resume_data with no operation asked");
        match (a.decode)(&*a.op, d) {
            Ok(v) => self.answer(Ok(v)),
            Err(why) => self.fail(format!("okay: the answer did not decode: {why}")),
        }
    }

    /// the operation answered with a failure: raised where it was performed
    pub fn fail(&mut self, error: String) -> Step<A> {
        self.answer(Err(error))
    }

    /// stopped where it is: the program dropped, its destructors run
    pub fn cancel(&mut self) -> Step<A> {
        self.finish();
        Step::Cancelled
    }

    fn answer(&mut self, a: Result<Answer, String>) -> Step<A> {
        self.asked = None;
        *self.slot.answer.borrow_mut() = Some(a);
        self.step()
    }

    fn finish(&mut self) {
        self.fut = None;
        self.asked = None;
        *self.slot.asked.borrow_mut() = None;
        *self.slot.answer.borrow_mut() = None;
    }
}

// THE HOST'S SIDE: every operation as data, answered in one place, through middleware

/// the host's answer to any operation: a plain value, or a failure raised where it was performed
pub type Answerer<'a> = Box<dyn FnMut(&Asked) -> Result<Data, String> + 'a>;

/// around every operation — monitor, log, authorize, record; `next` answers it
pub type Middleware<'a> =
    Box<dyn FnMut(&Asked, &mut dyn FnMut(&Asked) -> Result<Data, String>) -> Result<Data, String> + 'a>;

/// the program run to its value, every operation it asks answered by `answer`
pub fn drive<A>(mut c: Computation<'_, A>, mut answer: impl FnMut(&Asked) -> Result<Data, String>) -> Result<A, String> {
    let mut s = c.step();
    loop {
        s = match s {
            Step::Done(a) => return Ok(a),
            Step::Failed(why) => return Err(why),
            Step::Cancelled => return Err("okay: cancelled".into()),
            Step::Yielded(asked) => match answer(&asked) {
                Ok(d) => c.resume_data(&d),
                Err(e) => c.fail(e),
            },
        }
    }
}

fn through(ms: &mut [Middleware<'_>], answer: &mut Answerer<'_>, a: &Asked) -> Result<Data, String> {
    match ms.split_first_mut() {
        None => answer(a),
        Some((m, rest)) => m(a, &mut |x| through(rest, answer, x)),
    }
}

/// `answer` wrapped in middleware, the first outermost
pub fn with<'a>(mut answer: Answerer<'a>, mut ms: Vec<Middleware<'a>>) -> Answerer<'a> {
    Box::new(move |a| through(&mut ms, &mut answer, a))
}

/// `m` for the operations of `E` alone; the others pass by untouched
pub fn only<'a, E: Effect>(
    mut m: impl FnMut(&E, &Asked, &mut dyn FnMut(&Asked) -> Result<Data, String>) -> Result<Data, String> + 'a,
) -> Middleware<'a> {
    Box::new(move |a, next| match a.get::<E>() {
        Some(e) => m(e, a, next),
        None => next(a),
    })
}

/// ANSWERS BY EFFECT, each typed: a handler of `E` answers through the operation's reply (`r.give(v)`)
#[derive(Default)]
pub struct Routes<'a> {
    by: HashMap<&'static str, Answerer<'a>>,
}

impl<'a> Routes<'a> {
    pub fn new() -> Self {
        Routes { by: HashMap::new() }
    }

    /// `f` answers the operations of `E`: `|op| match op { E::Get(k, r) => Ok(r.give(…)) }`
    pub fn on<E: Remote>(mut self, mut f: impl FnMut(E) -> Result<Answer, String> + 'a) -> Self {
        let answer: Answerer<'a> = Box::new(move |a| {
            let op = a.get::<E>().expect("routed by its effect").clone();
            f(op).map(|v| a.out(v))
        });
        self.by.insert(std::any::type_name::<E>(), answer);
        self
    }

    /// a handler of `E`, the same one a context takes, answers its operations at the host
    pub fn handler<E: Remote>(self, h: impl Handles<E> + 'a) -> Self {
        self.on::<E>(move |op| match h.handle(op) {
            Handled::Now(a) => Ok(a),
            _ => Err(format!("okay: a handler of {} at the host answers at once", std::any::type_name::<E>())),
        })
    }

    pub fn answerer(mut self) -> Answerer<'a> {
        Box::new(move |a| match self.by.get_mut(a.effect()) {
            Some(f) => f(a),
            None => Err(format!("okay: no route for {}", a.name())),
        })
    }
}

/// one operation and how it was answered
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub record: Data,
    pub answer: Result<Data, String>,
}

/// every operation and its answer appended to `log`
pub fn record<'a>(log: Rc<RefCell<Vec<Entry>>>) -> Middleware<'a> {
    Box::new(move |a, next| {
        let v = next(a);
        log.borrow_mut().push(Entry { name: a.name().into(), record: a.record().clone(), answer: v.clone() });
        v
    })
}

/// a recorded run answered again from its log, nothing reached; an operation other than the one recorded is refused,
/// the run having diverged
pub fn replay<'a>(log: Vec<Entry>) -> Answerer<'a> {
    let mut i = 0;
    Box::new(move |a| {
        let Some(e) = log.get(i) else { return Err(format!("okay replay: {a:?} past the end of the log")) };
        i += 1;
        if e.name != a.name() || &e.record != a.record() {
            return Err(format!("okay replay: diverged at {}: recorded {} {:?}, now {a:?}", i - 1, e.name, e.record));
        }
        e.answer.clone()
    })
}

/// every operation answered over okay's wire through `call(name, args)` — no transport here: whatever sends a
/// request and waits. Middleware goes around it as around any answerer
pub fn remote<'a>(mut call: impl FnMut(&str, Vec<Data>) -> Result<Data, String> + 'a) -> Answerer<'a> {
    Box::new(move |a| {
        let args = match a.record() {
            Data::Dict(kv) if kv.is_empty() => vec![],
            r => vec![r.clone()],
        };
        call(a.name(), args)
    })
}

/// EVERY PATH of a handed-out program: its value for each combination of answers, depth first — `answers` gives every
/// answer an operation may have; `program` starts it anew (a future cannot be cloned: the answers so far replayed)
pub fn every_path<'a, A>(
    answers: impl Fn(&Asked) -> Vec<Data>,
    program: impl Fn() -> Computation<'a, A>,
) -> Result<Vec<A>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<Vec<usize>> = vec![vec![]];
    while let Some(path) = stack.pop() {
        let mut c = program();
        let mut s = c.step();
        let mut depth = 0;
        loop {
            match s {
                Step::Done(a) => {
                    out.push(a);
                    break;
                }
                Step::Failed(why) => return Err(why),
                Step::Cancelled => break,
                Step::Yielded(asked) => {
                    let all = answers(&asked);
                    if depth < path.len() {
                        s = c.resume_data(&all[path[depth]]);
                        depth += 1;
                    } else {
                        for i in (0..all.len()).rev() {
                            stack.push([path.clone(), vec![i]].concat());
                        }
                        break;
                    }
                }
            }
        }
    }
    Ok(out)
}
