//! The kernel's protocol: a program handed out, stepped by a host that sees only
//! operations and answers (as in okay-kernel).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use crate::core::{Cap, Run};
use crate::data::{Data, IsData, Operation};

type Decode = fn(&Data) -> Result<Box<dyn Any>, String>;

struct Codec {
    describe: fn(&dyn Any) -> (String, Data),
    decode: Decode,
}

fn describe<O: Operation>(op: &dyn Any) -> (String, Data) {
    let o = op.downcast_ref::<O>().expect("a host capability carries its operation");
    (O::name(), o.to_data())
}

fn decode<O: Operation>(d: &Data) -> Result<Box<dyn Any>, String> {
    Ok(Box::new(<O::Answer as IsData>::from_data(d)?))
}

/// The outermost owner of a handed-out program: capabilities the host answers.
#[derive(Clone)]
pub struct Host {
    run: Run,
    codecs: Rc<RefCell<HashMap<u64, Codec>>>,
}

impl Host {
    /// A capability whose operations go to whoever steps the program.
    pub fn outside<O: Operation>(&self) -> Cap<O> {
        let c: Cap<O> = Cap::new(&self.run);
        self.codecs.borrow_mut().insert(c.id, Codec { describe: describe::<O>, decode: decode::<O> });
        c
    }
}

/// An operation the program asks its host: typed by `get`, or as data.
pub struct Asked {
    op: Box<dyn Any>,
    name: String,
    record: Data,
}

impl Asked {
    /// the operation, if it is an `O`
    pub fn get<O: 'static>(&self) -> Option<&O> {
        self.op.downcast_ref::<O>()
    }
    /// its name on the wire
    pub fn name(&self) -> &str {
        &self.name
    }
    /// its fields, as a record
    pub fn record(&self) -> &Data {
        &self.record
    }
}

impl std::fmt::Debug for Asked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {:?}", self.name, self.record)
    }
}

/// One stop of a handed-out program.
#[derive(Debug)]
pub enum Step<A> {
    Done(A),
    Yielded(Asked),
    Failed(String),
    Cancelled,
}

/// A program handed out to a host, run one step at a time.
pub struct Computation<A> {
    run: Run,
    host: Host,
    fut: Option<Pin<Box<dyn Future<Output = A>>>>,
    decode: Option<Decode>,
}

/// Hand `body` out: nothing runs until the first `step()`.
pub fn start<A, F, Fut>(body: F) -> Computation<A>
where
    F: FnOnce(Host) -> Fut,
    Fut: Future<Output = A> + 'static,
{
    let run = Run::new();
    let host = Host { run: run.clone(), codecs: Rc::default() };
    let fut: Pin<Box<dyn Future<Output = A>>> = Box::pin(body(host.clone()));
    Computation { run, host, fut: Some(fut), decode: None }
}

fn message(p: Box<dyn Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "a panic".into())
}

struct Unpark(std::thread::Thread);
impl std::task::Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

impl<A> Computation<A> {
    /// Run to the value, the failure, or the first operation for the host.
    pub fn step(&mut self) -> Step<A> {
        let Some(fut) = self.fut.as_mut() else { panic!("okay: a finished computation stepped again") };
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        loop {
            match catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(&mut cx))) {
                Ok(Poll::Ready(a)) => return self.finish(Step::Done(a)),
                Err(p) => return self.finish(Step::Failed(message(p))),
                Ok(Poll::Pending) => {}
            }
            let mut mb = self.run.0.mailbox.borrow_mut();
            match mb.as_mut() {
                Some(p) if p.asked() => {
                    let codecs = self.host.codecs.borrow();
                    let Some(codec) = codecs.get(&p.cap) else {
                        let cap = p.cap;
                        drop(codecs);
                        drop(mb);
                        return self.finish(Step::Failed(format!("okay: an operation (capability #{cap}) no handler owns")));
                    };
                    let op = p.op.take().expect("asked once");
                    let (name, record) = (codec.describe)(op.as_ref());
                    self.decode = Some(codec.decode);
                    return Step::Yielded(Asked { op, name, record });
                }
                _ => {
                    drop(mb);
                    std::thread::park(); // a real async wait inside: until it wakes
                }
            }
        }
    }

    /// The operation answered with `v`, of the type it answers; run to the next stop.
    pub fn resume<T: 'static>(&mut self, v: T) -> Step<A> {
        self.answer(Box::new(v))
    }

    /// The operation answered with a plain value, decoded into what it answers.
    pub fn resume_data(&mut self, d: &Data) -> Step<A> {
        let decode = self.decode.expect("okay: resume_data with no operation asked");
        match decode(d) {
            Ok(v) => self.answer(v),
            Err(why) => self.fail(format!("okay: the answer did not decode: {why}")),
        }
    }

    /// The operation answered with a failure: a panic where it was performed.
    pub fn fail(&mut self, error: String) -> Step<A> {
        if let Some(p) = self.run.0.mailbox.borrow_mut().as_mut() {
            p.raised = Some(Box::new(error));
        }
        self.step()
    }

    /// Stopped where it is: the program is dropped, its destructors run.
    pub fn cancel(&mut self) -> Step<A> {
        self.fut = None;
        *self.run.0.mailbox.borrow_mut() = None;
        Step::Cancelled
    }

    fn answer(&mut self, v: Box<dyn Any>) -> Step<A> {
        if let Some(p) = self.run.0.mailbox.borrow_mut().as_mut() {
            p.answer = Some(v);
        }
        self.step()
    }

    fn finish(&mut self, s: Step<A>) -> Step<A> {
        self.fut = None;
        *self.run.0.mailbox.borrow_mut() = None;
        s
    }
}
