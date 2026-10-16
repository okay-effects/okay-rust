//! THE MACHINE: a loop over the program's nodes, a stack of levels — each a handler's (or the top), its frames the
//! binds waiting inside it. An operation goes to the nearest level of its effect: answered in place, the state's moved
//! in place, or — a handler taking the rest — the levels from it up taken off as the rest `k`, data, and its clause
//! run outside them. A `k` resumed: its levels pushed back, copies, each state as it was. No recursion: binds nested
//! to any depth, in a stack of the heap.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

use crate::eff::{Frame, Kind, Node, Op, Val};

pub(crate) struct AnsweringRt {
    pub(crate) value: Box<dyn Fn(Val) -> Val>,
    pub(crate) ret: Box<dyn Fn(Val) -> Val>,
}

/// a state's step: the state and the operation to the state moved and the value
pub(crate) type Step = Box<dyn Fn(Rc<dyn Any>, Val) -> (Rc<dyn Any>, Val)>;

pub(crate) struct StatefulRt {
    pub(crate) step: Step,
    pub(crate) ret: Box<dyn Fn(Rc<dyn Any>, Val) -> Val>,
}

pub(crate) struct DeepRt {
    pub(crate) ret: Box<dyn Fn(Val) -> Node>,
    pub(crate) apply: Box<dyn Fn(Val, K0) -> Node>,
}

/// a handler dropping the rest: its program in place of the body's, no `k` made
pub(crate) struct AbortingRt {
    pub(crate) ret: Box<dyn Fn(Val) -> Node>,
    pub(crate) abort: Box<dyn Fn(Val) -> Node>,
}

pub(crate) struct ObserveRt {
    pub(crate) ret: Box<dyn Fn(Val) -> Node>,
    pub(crate) apply: Box<dyn Fn(Op, K0) -> Node>,
}

/// a level: the top, or a handler's — its effect's type id, its runtime, a state's register
#[derive(Clone)]
pub(crate) enum Level {
    Top,
    Answering(TypeId, Rc<AnsweringRt>),
    Stateful(TypeId, Rc<StatefulRt>, Rc<dyn Any>),
    Deep(TypeId, Rc<DeepRt>),
    Aborting(TypeId, Rc<AbortingRt>),
    /// an observer: every operation on its way out stops here first
    Observe(Rc<ObserveRt>),
}

impl Level {
    /// the operations of `e` stop here: a handler of `e`, or an observer
    fn catches(&self, e: TypeId) -> bool {
        match self {
            Level::Top => false,
            Level::Answering(f, _) | Level::Stateful(f, _, _) | Level::Deep(f, _) | Level::Aborting(f, _) => *f == e,
            Level::Observe(_) => true,
        }
    }
}

/// a level and the frames waiting inside it, innermost last
pub(crate) struct Segment {
    level: Level,
    frames: Vec<Frame>,
}

/// a rest taken: the levels from a handler's up, outermost first. Resumed by the last holder of it, its levels are
/// moved back as they are — nothing copied (a handler resuming once: most); else copies of them, their frames made
/// shared the first time (a closure moved behind an `Rc`, once), each state as it was
#[derive(Clone)]
pub(crate) struct K0(Rc<RefCell<Vec<Segment>>>);

/// the vectors of levels of rests resumed once, kept for the next rest taken: none allocated for most
type Pool = Vec<Vec<Segment>>;

impl K0 {
    /// the levels from `i` up taken off `stack`, into a vector from the pool
    fn take(stack: &mut Vec<Segment>, i: usize, pool: &mut Pool) -> K0 {
        let mut taken = pool.pop().unwrap_or_default();
        taken.extend(stack.drain(i..));
        K0(Rc::new(RefCell::new(taken)))
    }

    /// the levels put back on `stack`; a vector moved back, to the pool
    fn put(self, stack: &mut Vec<Segment>, pool: &mut Pool) {
        match Rc::try_unwrap(self.0) {
            Ok(levels) => {
                let mut levels = levels.into_inner();
                stack.append(&mut levels);
                if pool.len() < 64 {
                    pool.push(levels);
                }
            }
            Err(shared) => {
                let mut levels = shared.borrow_mut();
                for s in levels.iter_mut() {
                    s.frames.iter_mut().for_each(Frame::make_shared);
                }
                stack.extend(levels.iter().map(|t| Segment {
                    level: t.level.clone(),
                    frames: t.frames.iter().map(Frame::copy_shared).collect(),
                }));
            }
        }
    }
}

/// WHERE A RUN STOPPED: its value; an operation no level of the program handles, given back with the rest — every
/// level, the top's included — for a host to resume (once, many times, or never); or a failure raised to the top
pub(crate) enum Outcome {
    Done(Val),
    Asked(Op, K0),
    Failed(crate::eff::Failure),
}

/// a program of no effect run to its value
pub(crate) fn run(node: Node) -> Val {
    match drive(Vec::new(), node.0, true) {
        Outcome::Done(v) => v,
        Outcome::Asked(..) => panic!("okay: an operation with no handler — a row not checked"),
        Outcome::Failed(f) => std::panic::resume_unwind(f.0),
    }
}

/// a program run from its start until it is done, fails, or asks its host
pub(crate) fn start(node: Node) -> Outcome {
    drive(Vec::new(), node.0, true)
}

/// a rest given back to a host, resumed with `v`: its levels put back on an empty stack, run until the next stop
pub(crate) fn resume(k: K0, v: Val) -> Outcome {
    drive(Vec::new(), Kind::Resume(k, v), false)
}

/// a rest given back to a host, failed: `f` raised where the operation was — with no guard among its levels, it
/// unwinds every one of them to the top
pub(crate) fn fail(k: K0, f: crate::eff::Failure) -> Outcome {
    drop(k);
    Outcome::Failed(f)
}

/// the loop: `top` when the stack starts with the top level (a run started; a rest resumed brings its own)
fn drive(mut stack: Vec<Segment>, cur: Kind, top: bool) -> Outcome {
    if top {
        stack.push(Segment { level: Level::Top, frames: Vec::new() });
    }
    // in a state's place while it is moved: the state taken out, so that no one else holds it and it is written in place
    let hole: Rc<dyn Any> = Rc::new(());
    let mut pool: Pool = Vec::new();
    let mut cur = cur;
    loop {
        cur = match cur {
            Kind::Return(v) => {
                let Some(top) = stack.last_mut() else { return Outcome::Done(v) };
                match top.frames.pop() {
                    Some(f) => f.call(v).0,
                    // a level's frames all done: the level left, the value through its `ret`
                    None => match stack.pop().map(|s| s.level) {
                        None | Some(Level::Top) => return Outcome::Done(v),
                        Some(Level::Answering(_, rt)) => Kind::Return((rt.ret)(v)),
                        Some(Level::Stateful(_, rt, s)) => Kind::Return((rt.ret)(s, v)),
                        Some(Level::Deep(_, rt)) => (rt.ret)(v).0,
                        Some(Level::Aborting(_, rt)) => (rt.ret)(v).0,
                        Some(Level::Observe(rt)) => (rt.ret)(v).0,
                    },
                }
            }
            Kind::Bind(m, f) => {
                if let Some(top) = stack.last_mut() {
                    top.frames.push(f);
                }
                m.0
            }
            Kind::Then(v, f) => f.call(v).0,
            Kind::Inject(op, f) => {
                let Some(i) = stack.iter().rposition(|s| s.level.catches(op.effect)) else {
                    // none in the program: the host's — the rest, this frame first, given back with it
                    if let Some(top) = stack.last_mut() {
                        top.frames.push(f);
                    }
                    return Outcome::Asked(op, K0::take(&mut stack, 0, &mut pool));
                };
                match &mut stack[i].level {
                    // answered in place: the value straight into the rest, no frame pushed
                    Level::Answering(_, rt) => f.call((rt.value)(op.value)).0,
                    Level::Stateful(_, rt, s) => {
                        let (s2, v) = (rt.step)(std::mem::replace(s, hole.clone()), op.value);
                        *s = s2;
                        f.call(v).0
                    }
                    // the rest taken: its frame first on the top, into `k`
                    _ => {
                        if let Some(top) = stack.last_mut() {
                            top.frames.push(f);
                        }
                        Kind::Perform(op)
                    }
                }
            }
            Kind::Handle(body, level) => {
                stack.push(Segment { level, frames: Vec::new() });
                body.0
            }
            Kind::Perform(op) => {
                // the nearest level of the effect, or an observer before it; none in the program — an effect of the row
                // outside it: the host's, given back with the rest, every level
                let Some(i) = stack.iter().rposition(|s| s.level.catches(op.effect)) else {
                    return Outcome::Asked(op, K0::take(&mut stack, 0, &mut pool));
                };
                match &mut stack[i].level {
                    Level::Answering(_, rt) => Kind::Return((rt.value)(op.value)),
                    Level::Stateful(_, rt, s) => {
                        let (s2, v) = (rt.step)(std::mem::replace(s, hole.clone()), op.value);
                        *s = s2;
                        Kind::Return(v)
                    }
                    Level::Deep(_, rt) => {
                        let rt = rt.clone();
                        let k = K0::take(&mut stack, i, &mut pool);
                        (rt.apply)(op.value, k).0
                    }
                    // dropping the rest: the levels from it up dropped, no `k` made; its program outside it
                    Level::Aborting(_, rt) => {
                        let rt = rt.clone();
                        stack.truncate(i);
                        (rt.abort)(op.value).0
                    }
                    // an observer: the rest taken as a handler's, its clause run outside it with the whole operation
                    Level::Observe(rt) => {
                        let rt = rt.clone();
                        let k = K0::take(&mut stack, i, &mut pool);
                        (rt.apply)(op, k).0
                    }
                    Level::Top => panic!("okay: the top handles no effect"),
                }
            }
            Kind::Defer(p) => p().0,
            Kind::Resume(k, v) => {
                k.put(&mut stack, &mut pool);
                Kind::Return(v)
            }
        }
    }
}
