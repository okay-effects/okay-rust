//! THE MACHINE: a loop over the program's nodes, a stack of levels — each a handler's (or the top), its frames the
//! binds waiting inside it. An operation goes to the nearest level of its effect: answered in place, the state's moved
//! in place, or — a handler taking the rest — the levels from it up taken off as the rest `k`, data, and its clause
//! run outside them. A `k` resumed: its levels pushed back, copies, each state as it was. No recursion: binds nested
//! to any depth, in a stack of the heap.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use crate::eff::{rare, Failure, Frame, Kind, Node, Op, Rare, Val};

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

/// what a guard does with an operation passing out through it: the operation sent in its place, and what to do each
/// time its answer comes back in
pub(crate) type Passing = Rc<dyn Fn(Op) -> (Op, Option<Rc<dyn Fn()>>)>;

/// A GUARD: a level of no effect — its finalizer run when it is left (a value, a failure, an abort, a cancel), its
/// recover given a failure raised in it, its passing told of each operation going out
pub(crate) struct GuardRt {
    pub(crate) finalizer: Option<Rc<dyn Fn() -> Node>>,
    pub(crate) recover: Option<Box<dyn Fn(Failure) -> Node>>,
    pub(crate) passing: Option<Passing>,
    /// the body's value copied: a finalizer's rest may be resumed more than once
    pub(crate) copy: fn(&Val) -> Val,
}

thread_local! {
    /// a guard or a mask was entered in this thread: runs from now on in the guarded loop (a rest may hold one)
    pub(crate) static GUARDED: Cell<bool> = const { Cell::new(false) };
    /// a guard with a passing was made: operations look for one on their way out (none made, no look)
    pub(crate) static PASSING: Cell<bool> = const { Cell::new(false) };
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
    /// a guard: no effect of its own
    Guard(Rc<GuardRt>),
    /// uncancelable: a cancel asked inside waits for it to be left — `true` once one is pending
    Mask(Rc<Cell<bool>>),
    /// a `$`: its body's value through `ret`, a program run below it; a shift0's delimiter
    Dollar(Rc<dyn Fn(Val) -> Node>),
    /// not a level: a rare node (a raise, a cancel, control's), carried by a `Handle` and taken at once
    Rare(Rc<RefCell<Option<Rare>>>),
}

impl Level {
    /// the operations of `e` stop here: a handler of `e`, or an observer
    #[inline(always)]
    fn catches(&self, e: TypeId) -> bool {
        match self {
            Level::Top => false,
            Level::Answering(f, _) | Level::Stateful(f, _, _) | Level::Deep(f, _) | Level::Aborting(f, _) => *f == e,
            Level::Observe(_) => true,
            Level::Guard(_) | Level::Mask(_) | Level::Dollar(_) | Level::Rare(_) => false,
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
    Failed(Failure),
    Cancelled,
    /// the fast loop met a guard: go on in the guarded one, from this node
    Guarded(Kind),
}

/// a program of no effect run to its value
pub(crate) fn run(node: Node) -> Val {
    match drive(Vec::new(), node.0, true) {
        Outcome::Done(v) => v,
        Outcome::Asked(..) => panic!("okay: an operation with no handler — a row not checked"),
        Outcome::Failed(f) => std::panic::resume_unwind(f.0),
        Outcome::Cancelled => panic!("okay: a program cancelled itself with no host"),
        Outcome::Guarded(_) => unreachable!("the guarded loop goes on itself"),
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

/// a rest given back to a host, failed: `f` raised where the operation was, in its levels — their guards asked
pub(crate) fn fail(k: K0, f: Failure) -> Outcome {
    drive(Vec::new(), rare(Rare::Fail(k, f, false)), false)
}

/// a rest given back to a host, cancelled: its finalizers run, no recover asked
pub(crate) fn cancel(k: K0) -> Outcome {
    drive(Vec::new(), rare(Rare::CancelK(k)), false)
}

impl K0 {
    /// a rest that is a function: one transparent level, `f` its one frame (the reference interpreter's `k`)
    #[cfg(test)]
    pub(crate) fn of_fn(f: impl Fn(Val) -> Node + 'static) -> K0 {
        let g = GuardRt { finalizer: None, recover: None, passing: None, copy: crate::eff::no_copy };
        K0(Rc::new(RefCell::new(vec![Segment { level: Level::Guard(Rc::new(g)), frames: vec![Frame::new(f)] }])))
    }

    /// a rest made by `of_fn`, resumed with `v`: its function's program
    #[cfg(test)]
    pub(crate) fn call_fn(&self, v: Val) -> Node {
        let mut levels = self.0.borrow_mut();
        let [s] = levels.as_mut_slice() else { panic!("the reference resumes only its own rests") };
        let [f] = s.frames.as_mut_slice() else { panic!("the reference resumes only its own rests") };
        f.make_shared();
        f.copy_shared().call(v)
    }

    /// an uncancelable level among the rest's
    pub(crate) fn masked(&self) -> bool {
        self.0.borrow().iter().any(|s| matches!(s.level, Level::Mask(_)))
    }

    /// a cancel pending in each uncancelable level of the rest: fired when it is left
    pub(crate) fn pend_cancel(&self) {
        for s in self.0.borrow().iter() {
            if let Level::Mask(p) = &s.level {
                p.set(true);
            }
        }
    }
}

/// the loop, user code's panics caught at it: a panic is a `Raise` where it happened — the machine's stack is on the
/// heap, untouched by the unwinding, so its levels are unwound by the machine itself, their guards asked
fn drive(mut stack: Vec<Segment>, cur: Kind, top: bool) -> Outcome {
    if top {
        stack.push(Segment { level: Level::Top, frames: Vec::new() });
    }
    if !GUARDED.with(Cell::get) {
        // no guard yet: the loop on its own locals, in a function of its own — no `catch_unwind` taking their address
        return match fast(stack, cur) {
            Ok(o) => o,
            Err(go_on) => {
                let (stack, pool, next) = *go_on;
                guarded(stack, pool, next)
            }
        };
    }
    guarded(stack, Vec::new(), cur)
}

/// the loop with no guard: its stack and pool locals of its own; a guard met, both given back with the node to go on
#[inline(never)]
#[allow(clippy::type_complexity)]
fn fast(mut stack: Vec<Segment>, cur: Kind) -> Result<Outcome, Box<(Vec<Segment>, Pool, Kind)>> {
    let hole: Rc<dyn Any> = Rc::new(());
    let mut pool: Pool = Vec::new();
    match steps_in::<true>(&mut stack, &mut pool, &hole, cur) {
        Outcome::Guarded(next) => Err(Box::new((stack, pool, next))),
        o => Ok(o),
    }
}

/// the loop under `catch_unwind`: a panic a `Raise` where it happened, the levels unwound with their guards
fn guarded(mut stack: Vec<Segment>, mut pool: Pool, cur: Kind) -> Outcome {
    // in a state's place while it is moved: the state taken out, so that no one else holds it and it is written in place
    let hole: Rc<dyn Any> = Rc::new(());
    let mut cur = cur;
    loop {
        match catch_unwind(AssertUnwindSafe(|| steps(&mut stack, &mut pool, &hole, cur))) {
            Ok(o) => return o,
            Err(p) => cur = rare(Rare::Raise(Failure(p))),
        }
    }
}

/// a node then `k`'s node, `k` called each time the node's rest is (a rest resumed twice, twice)
fn then(first: Node, k: impl Fn() -> Kind + 'static) -> Kind {
    Kind::Bind(Box::new(first), Frame::new(move |_| Node(k())))
}

/// the finalizers in order, then `last`
fn finalizers_then(fins: Vec<Rc<dyn Fn() -> Node>>, last: Rc<dyn Fn() -> Kind>) -> Kind {
    match fins.split_first() {
        None => last(),
        Some((f, rest)) => {
            let rest = rest.to_vec();
            then(f(), move || finalizers_then(rest.clone(), last.clone()))
        }
    }
}

/// a failure that may be raised again after a finalizer resumed more than once: the first time itself, after that its
/// message
fn again(f: Failure) -> Rc<dyn Fn() -> Kind> {
    let once = Rc::new(RefCell::new(Some(f)));
    Rc::new(move || {
        let taken = once.borrow_mut().take();
        let msg = || Failure::new(String::from("okay: a failure raised again by a finalizer resumed twice"));
        rare(Rare::Raise(taken.unwrap_or_else(msg)))
    })
}

/// the operation through every guard with a passing between it and its handler (index `from` up), innermost first;
/// what to call each time its answer comes back in
fn passing(stack: &[Segment], from: usize, mut op: Op) -> (Op, Vec<Rc<dyn Fn()>>) {
    let mut back = Vec::new();
    for s in stack[from..].iter().rev() {
        if let Level::Guard(g) = &s.level
            && let Some(p) = &g.passing {
                let (o, b) = p(op);
                op = o;
                back.extend(b);
            }
    }
    (op, back)
}

fn steps(stack: &mut Vec<Segment>, pool: &mut Pool, hole: &Rc<dyn Any>, cur: Kind) -> Outcome {
    steps_in::<false>(stack, pool, hole, cur)
}

/// THE LOOP. `FAST`: no guard in this thread yet — the loop on its own locals, a panic out of it (no guard to ask, so
/// the same as unwinding every level to the top); a guard or a mask entered, the loop handed to the guarded one, the
/// stack as it is
#[inline(always)]
fn steps_in<const FAST: bool>(stack: &mut Vec<Segment>, pool: &mut Pool, hole: &Rc<dyn Any>, cur: Kind) -> Outcome {
    let mut cur = cur;
    // a guard with a passing made: read once, not at each operation — and set when one is entered (none in the fast
    // loop: a guard entered hands it to the guarded one)
    let mut passing_guards = !FAST && PASSING.with(Cell::get);
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
                        Some(level) => left(level, v),
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
            Kind::Inject(op, f) if !FAST && passing_guards => inject_passing(stack, op, f),
            Kind::Inject(op, f) => {
                let Some(i) = stack.iter().rposition(|s| s.level.catches(op.effect())) else {
                    // none in the program: the host's — the rest, this frame first, given back with it
                    if let Some(top) = stack.last_mut() {
                        top.frames.push(f);
                    }
                    return Outcome::Asked(op, K0::take(stack, 0, pool));
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
            Kind::Handle(_, Level::Rare(r)) => {
                let node = r.borrow_mut().take().expect("a rare node taken once");
                match rare_node(stack, pool, hole, node) {
                    Ok(k) => k,
                    Err(o) => return o,
                }
            }
            Kind::Handle(body, level) => {
                if let Level::Guard(_) | Level::Mask(_) = &level {
                    GUARDED.with(|g| g.set(true));
                    if let Level::Guard(g) = &level {
                        passing_guards |= g.passing.is_some();
                    }
                    if FAST {
                        stack.push(Segment { level, frames: Vec::new() });
                        return Outcome::Guarded(body.0);
                    }
                }
                stack.push(Segment { level, frames: Vec::new() });
                body.0
            }
            Kind::Perform(op) if !FAST && passing_guards => perform_passing(stack, op),
            Kind::Perform(op) => {
                // the nearest level of the effect, or an observer before it; none in the program — an effect of the row
                // outside it: the host's, given back with the rest, every level
                let Some(i) = stack.iter().rposition(|s| s.level.catches(op.effect())) else {
                    return Outcome::Asked(op, K0::take(stack, 0, pool));
                };
                perform_at(stack, pool, hole, i, op)
            }
            Kind::Defer(p) => p().0,
            Kind::Resume(k, v) => {
                k.put(stack, pool);
                Kind::Return(v)
            }

        }
    }
}

/// an operation to the level `i` that catches it
#[inline(always)]
fn perform_at(stack: &mut Vec<Segment>, pool: &mut Pool, hole: &Rc<dyn Any>, i: usize, op: Op) -> Kind {
    match &mut stack[i].level {
        Level::Answering(_, rt) => Kind::Return((rt.value)(op.value)),
        Level::Stateful(_, rt, s) => {
            let (s2, v) = (rt.step)(std::mem::replace(s, hole.clone()), op.value);
            *s = s2;
            Kind::Return(v)
        }
        Level::Deep(_, rt) => {
            let rt = rt.clone();
            let k = K0::take(stack, i, pool);
            (rt.apply)(op.value, k).0
        }
        // dropping the rest: the levels from it up dropped, no `k` made; its program outside it — the
        // finalizers of the guards dropped run first, inside its level
        Level::Aborting(_, rt) => {
            let rt = rt.clone();
            aborted(stack, i, rt, op)
        }
        // an observer: the rest taken as a handler's, its clause run outside it with the whole operation
        Level::Observe(rt) => {
            let rt = rt.clone();
            let k = K0::take(stack, i, pool);
            (rt.apply)(op, k).0
        }
        Level::Top | Level::Guard(_) | Level::Mask(_) | Level::Dollar(_) | Level::Rare(_) => panic!("okay: a level of no effect caught one"),
    }
}

/// a level of no effect left with a value: a guard's finalizer, a mask's pending cancel, a `$`'s `ret`
#[cold]
#[inline(never)]
fn left(level: Level, v: Val) -> Kind {
    match level {
        Level::Guard(g) => match &g.finalizer {
            None => Kind::Return(v),
            Some(fin) => {
                let (v, copy) = (Rc::new(v), g.copy);
                then(fin(), move || Kind::Return(copy(&v)))
            }
        },
        Level::Mask(pending) => if pending.get() { rare(Rare::Cancel) } else { Kind::Return(v) },
        Level::Dollar(ret) => ret(v).0,
        _ => unreachable!("a handler's level left by its own arm"),
    }
}

/// dropping the rest: the levels from `i` up dropped, no `k` made; its program outside it — the finalizers of the
/// guards dropped run first, inside its level
#[inline(always)]
fn aborted(stack: &mut Vec<Segment>, i: usize, rt: Rc<AbortingRt>, op: Op) -> Kind {
    if !stack[i + 1..].iter().any(|s| matches!(&s.level, Level::Guard(g) if g.finalizer.is_some())) {
        stack.truncate(i);
        return (rt.abort)(op.value).0;
    }
    aborted_guarded(stack, i, rt, op)
}

#[cold]
#[inline(never)]
fn aborted_guarded(stack: &mut Vec<Segment>, i: usize, rt: Rc<AbortingRt>, op: Op) -> Kind {
    let fins: Vec<Rc<dyn Fn() -> Node>> = stack[i + 1..]
        .iter()
        .rev()
        .filter_map(|s| match &s.level {
            Level::Guard(g) => g.finalizer.clone(),
            _ => None,
        })
        .collect();
    stack.truncate(i + 1);
    stack[i].frames.clear();
    let op = Rc::new(op);
    finalizers_then(fins, Rc::new(move || rare(Rare::Leave(rt.clone(), (*op).clone()))))
}

/// an operation with its rest's frame through the guards with a passing it goes out past: the frame first on the
/// top, then what to do at each resumption above it — frames, copied with a `k` — then to its handler
#[cold]
#[inline(never)]
fn inject_passing(stack: &mut [Segment], op: Op, f: Frame) -> Kind {
    if let Some(top) = stack.last_mut() {
        top.frames.push(f);
    }
    perform_passing(stack, op)
}

/// an operation through the guards with a passing it goes out past, then to its handler
#[cold]
#[inline(never)]
fn perform_passing(stack: &mut [Segment], op: Op) -> Kind {
    let found = stack.iter().rposition(|s| s.level.catches(op.effect()));
    let (op, back) = passing(stack, found.map_or(0, |i| i + 1), op);
    if !back.is_empty()
        && let Some(top) = stack.last_mut()
    {
        top.frames.push(Frame::new(move |v| {
            back.iter().for_each(|b| b());
            Node(Kind::Return(v))
        }));
    }
    rare(Rare::Passed(op))
}

/// the machine's cold nodes, out of its loop so the loop stays small: control's, a host's failure and cancel, a raise
#[cold]
#[inline(never)]
fn rare_node(stack: &mut Vec<Segment>, pool: &mut Pool, hole: &Rc<dyn Any>, r: Rare) -> Result<Kind, Outcome> {
    Ok(match r {
        Rare::Shift0(f) => {
            let Some(i) = stack.iter().rposition(|s| matches!(s.level, Level::Dollar(_))) else {
                panic!("okay: a shift0 with no $ around — a row not checked")
            };
            let k = K0::take(stack, i, pool);
            f(k).0
        }
        Rare::Passed(op) => {
            let Some(i) = stack.iter().rposition(|s| s.level.catches(op.effect())) else {
                return Err(Outcome::Asked(op, K0::take(stack, 0, pool)));
            };
            perform_at(stack, pool, hole, i, op)
        }
        Rare::Fail(k, f, unframe) => {
            if unframe
                && let Some(top) = stack.last_mut() {
                    top.frames.pop();
                }
            k.put(stack, pool);
            rare(Rare::Raise(f))
        }
        Rare::CancelK(k) => {
            k.put(stack, pool);
            rare(Rare::Cancel)
        }
        // the aborting handler's level left (its frames dropped already), its program in place of the body's
        Rare::Leave(rt, op) => {
            stack.pop();
            (rt.abort)(op.value).0
        }
        // unwound level by level: a recover takes the failure, a finalizer runs and the failure goes on
        Rare::Raise(f) => loop {
            match stack.pop().map(|s| s.level) {
                None | Some(Level::Top) => return Err(Outcome::Failed(f)),
                Some(Level::Guard(g)) => {
                    if let Some(rec) = &g.recover {
                        break rec(f).0;
                    }
                    if let Some(fin) = &g.finalizer {
                        let raise = again(f);
                        break then(fin(), move || raise());
                    }
                }
                // a cancel pending in an uncancelable region it fails out of: the cancel, not the failure
                Some(Level::Mask(pending)) if pending.get() => break rare(Rare::Cancel),
                Some(_) => {}
            }
        },
        Rare::Cancel => loop {
            match stack.pop().map(|s| s.level) {
                None | Some(Level::Top) => return Err(Outcome::Cancelled),
                Some(Level::Guard(g)) => {
                    if let Some(fin) = &g.finalizer {
                        break then(fin(), || rare(Rare::Cancel));
                    }
                }
                Some(_) => {}
            }
        },
    })
}
