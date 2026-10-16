//! A handed-out program answered in one place: every operation, as data, through middleware.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::data::{Data, IsData, Operation};
use crate::kernel::{Asked, Computation, Step};

/// The host's answer to any operation: a plain value, or a failure raised where it was performed.
pub type Answerer<'a> = Box<dyn FnMut(&Asked) -> Result<Data, String> + 'a>;

/// Around every operation — monitor, log, authorize, record; `next` answers it.
pub type Middleware<'a> = Box<dyn FnMut(&Asked, &mut dyn FnMut(&Asked) -> Result<Data, String>) -> Result<Data, String> + 'a>;

/// The program run to its value, every operation it performs for the host answered by `answer`.
pub fn drive<A>(mut c: Computation<A>, mut answer: impl FnMut(&Asked) -> Result<Data, String>) -> Result<A, String> {
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

/// `answer` wrapped in middleware, the first outermost.
pub fn with<'a>(mut answer: Answerer<'a>, mut ms: Vec<Middleware<'a>>) -> Answerer<'a> {
    Box::new(move |a| through(&mut ms, &mut answer, a))
}

/// `m` for the operations of type `O` alone; the others pass by untouched.
pub fn only<'a, O: Operation>(
    mut m: impl FnMut(&O, &Asked, &mut dyn FnMut(&Asked) -> Result<Data, String>) -> Result<Data, String> + 'a,
) -> Middleware<'a> {
    Box::new(move |a, next| match a.get::<O>() {
        Some(o) => m(o, a, next),
        None => next(a),
    })
}

/// Answers by the operation's name, each typed.
#[derive(Default)]
pub struct Routes<'a> {
    by: HashMap<String, Answerer<'a>>,
}

impl<'a> Routes<'a> {
    pub fn new() -> Self {
        Routes { by: HashMap::new() }
    }

    /// `f` answers the operations of type `O`.
    pub fn on<O: Operation>(mut self, mut f: impl FnMut(&O) -> Result<O::Answer, String> + 'a) -> Self {
        self.by.insert(O::name(), Box::new(move |a| f(a.get::<O>().expect("routed by its name")).map(|x| x.to_data())));
        self
    }

    pub fn answerer(mut self) -> Answerer<'a> {
        Box::new(move |a| match self.by.get_mut(a.name()) {
            Some(f) => f(a),
            None => Err(format!("okay: no route for {}", a.name())),
        })
    }
}

/// One operation and how it was answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub record: Data,
    pub answer: Result<Data, String>,
}

/// Every operation and its answer appended to `log`.
pub fn record<'a>(log: Rc<RefCell<Vec<Entry>>>) -> Middleware<'a> {
    Box::new(move |a, next| {
        let v = next(a);
        log.borrow_mut().push(Entry { name: a.name().into(), record: a.record().clone(), answer: v.clone() });
        v
    })
}

/// A recorded run answered again from its log, nothing reached; an operation other
/// than the one recorded is refused, the run having diverged.
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
