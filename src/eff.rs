//! A PROGRAM OF EFFECTS, `Eff<R, A>`: of the value `A`, its effects the row `R` — data, a tree of nodes the machine
//! runs. The types only the compiler's: the nodes hold values erased (`Box<dyn Any>`), each taken back at the type it
//! was made at, which the typed API makes the only one possible.

use std::any::TypeId;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::handler::{Handler, Interceptor, Observer};
use crate::row::{AuditAll, Audited, Effect, Mark, Member, Nil, PlainKind, Sensitive};

pub(crate) use crate::val::Val;
/// a frame: the rest of a bind, given the value — `Fn`, not `FnOnce`: a `k` is resumed as many times as its handler
/// wants
pub(crate) use crate::frame::Frame;

/// a program erased: the machine's
#[doc(hidden)]
pub struct Node(pub(crate) Kind);

pub(crate) enum Kind {
    /// a value
    Return(Val),
    /// a program, then the rest given its value
    Bind(Box<Node>, Frame),
    /// a value, then the rest given it: `pure(a).flat_map(f)`, no node for the value
    Then(Val, Frame),
    /// an operation, then the rest given its value: `perform(op).flat_map(f)` — answered in place, straight into `f`
    Inject(Op, Frame),
    /// an operation: its handler the nearest of its effect — or an observer before it
    Perform(Op),
    /// a handler entered: its level, the body under it
    Handle(Box<Node>, crate::machine::Level),
    /// a `k` resumed: its levels put back, the value to its frames
    Resume(crate::machine::K0, Val),
    /// a program made when the run gets to it
    Defer(Rc<dyn Fn() -> Node>),
}

/// the machine's rare nodes: failures, cancels, control's
pub(crate) enum Rare {
    /// a failure raised here: the levels unwound — each guard's finalizer run, the nearest recover given it
    Raise(Failure),
    /// cancelled here: the levels unwound — each finalizer run, no recover asked
    Cancel,
    /// an aborting handler's finalizers done: its level left, its program in place of the body's
    Leave(Rc<crate::machine::AbortingRt>, Op),
    /// a rest given back to a host, failed: its levels put back, the failure raised in them — `true`: the frame on
    /// top dropped first (a translating clause's, waiting for an answer that will not come)
    Fail(crate::machine::K0, Failure, bool),
    /// an operation already told to the guards it passes: to its handler
    Passed(Op),
    /// a shift0: the rest up to the nearest `$`, the `$` with it, taken as `k`; the body's program run outside
    Shift0(Rc<dyn Fn(crate::machine::K0) -> Node>),
    /// a rest given back to a host, cancelled: its levels put back, cancelled in them
    CancelK(crate::machine::K0),
}

/// a rare node as a node: a `Handle` of a level that is no level (`Level::Rare`) — so `Kind` keeps the variants it had:
/// a variant of its own (a value, or each rare node one) made the machine's loop 10–20% slower, measured
pub(crate) fn rare(r: Rare) -> Kind {
    Kind::Handle(
        Box::new(Node(Kind::Return(Val::new(())))),
        crate::machine::Level::Rare(Rc::new(std::cell::RefCell::new(Some(r)))),
    )
}

/// AN OPERATION ERASED, with what an observer needs of it: its effect, a description, a copy — and of its answer the
/// same, made where its type was known
pub(crate) struct Op {
    pub(crate) info: &'static OpInfo,
    pub(crate) value: Val,
}

impl Op {
    /// its effect's type id: in its table (not in the node — a node moved at each step)
    #[inline(always)]
    pub(crate) fn effect(&self) -> TypeId {
        self.info.effect
    }
}

/// WHAT AN OBSERVER NEEDS OF AN OPERATION, the same for each of an effect and its value's type: one table, made at
/// compile time — the node holds a reference, not the six
pub(crate) struct OpInfo {
    pub(crate) effect: TypeId,
    pub(crate) name: fn() -> &'static str,
    pub(crate) describe: fn(&Val) -> String,
    pub(crate) copy: fn(&Val) -> Val,
    pub(crate) describe_answer: fn(&Val) -> String,
    pub(crate) copy_answer: fn(&Val) -> Val,
}

/// the table of an effect `E` and its operation's value `T`
struct Info<E, T>(PhantomData<fn() -> (E, T)>);

impl<E: Effect, T: Clone + Debug + 'static> Info<E, T> {
    const INFO: OpInfo = OpInfo {
        effect: TypeId::of::<E>(),
        name: std::any::type_name::<E>,
        describe: describe_as::<E>,
        copy: copy_as::<E>,
        describe_answer: describe_as::<T>,
        copy_answer: copy_as::<T>,
    };
}

impl Op {
    #[inline(always)]
    fn of<E: Effect, T: Clone + Debug + 'static>(e: E) -> Op {
        Op { info: &Info::<E, T>::INFO, value: Val::new(e) }
    }
}

impl Clone for Op {
    fn clone(&self) -> Self {
        Op { info: self.info, value: (self.info.copy)(&self.value) }
    }
}

fn describe_as<T: Debug + 'static>(v: &Val) -> String {
    match v.peek::<T>() {
        Some(t) => format!("{t:?}"),
        None => String::from("?"),
    }
}

pub(crate) fn copy_val<T: Clone + 'static>(v: &Val) -> Val {
    copy_as::<T>(v)
}

fn copy_as<T: Clone + 'static>(v: &Val) -> Val {
    match v.peek::<T>() {
        Some(t) => Val::new(t.clone()),
        None => panic!("okay: a value copied at another type than it was made at"),
    }
}

/// A PROGRAM OF EFFECTS over the row `R`, of the value `A`
pub struct Eff<R, A> {
    pub(crate) node: Node,
    _t: PhantomData<fn() -> (R, A)>,
}

impl<R, A> Eff<R, A> {
    pub(crate) fn of(kind: Kind) -> Self {
        Eff { node: Node(kind), _t: PhantomData }
    }
}

/// what a program is to the machine: its node, and a program back from one
pub trait Program: Sized {
    #[doc(hidden)]
    fn into_node(self) -> Node;
    #[doc(hidden)]
    fn from_node(n: Node) -> Self;
}

impl<R, A> Program for Eff<R, A> {
    fn into_node(self) -> Node {
        self.node
    }
    fn from_node(n: Node) -> Self {
        Eff { node: n, _t: PhantomData }
    }
}

/// THE ANSWERS MET at a bind, by the kind of the rest: a program of effects after a program of effects, one of
/// effects. (Stage 2: a program moving the answer, `Cont`, each of the four pairs its own impl — none overlapping,
/// as `Eff` and `Cont` are types of their own.)
#[diagnostic::on_unimplemented(message = "a `{Self}` cannot be followed by a `{N}`: the rows or the answers do not meet")]
pub trait Bind<N> {
    type Out: Program;
}

impl<R, A, B> Bind<Eff<R, B>> for Eff<R, A> {
    type Out = Eff<R, B>;
}

/// a program, then `frame`: a value or an operation fused with it, no node made for them
#[inline(always)]
fn bound(k: Kind, frame: Frame) -> Kind {
    match k {
        Kind::Return(v) => Kind::Then(v, frame),
        Kind::Perform(op) => Kind::Inject(op, frame),
        k => Kind::Bind(Box::new(Node(k)), frame),
    }
}

/// `a`, over any row
pub fn pure<R, A: 'static>(a: A) -> Eff<R, A> {
    Eff::of(Kind::Return(Val::new(a)))
}

/// `p`, made when the run gets to it: a recursion built lazily
pub fn defer<R: 'static, A: 'static>(p: impl Fn() -> Eff<R, A> + 'static) -> Eff<R, A> {
    Eff::of(Kind::Defer(Rc::new(move || p().node)))
}

/// A STEP OF A LOOP: on, with the next state, or done, with the value
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loop<S, A> {
    Next(S),
    Done(A),
}

/// A LOOP IN CONSTANT STACK (cats' `tailRecM`): `step` from a state to a program of the next step — `Next(s)` on,
/// `Done(a)` its value. Mutual recursion is a loop whose state names the function and its arguments
/// (`enum Call { Even(i64), Odd(i64) }`); the step's program may perform any effect of its row
pub fn tail_rec<R: 'static, S: 'static, A: 'static>(
    s0: S,
    step: impl Fn(S) -> Eff<R, Loop<S, A>> + 'static,
) -> Eff<R, A> {
    looping(s0, Rc::new(step))
}

#[allow(clippy::type_complexity)]
fn looping<R: 'static, S: 'static, A: 'static>(s: S, step: Rc<dyn Fn(S) -> Eff<R, Loop<S, A>>>) -> Eff<R, A> {
    let on = step.clone();
    step(s).flat_map(move |l| match l {
        Loop::Next(s) => looping(s, on.clone()),
        Loop::Done(a) => pure(a),
    })
}

/// AN OPERATION of `E`, in the row `R`: `make` its variant given its reply — `perform(Reader::Ask)`, `perform(|r|
/// State::Put(s, r))`; the program's value what its handler gives back. The value `Clone` and `Debug`: what crosses
/// the boundary of effects can be told and kept
pub fn perform<R, E: Effect, Ix, T: Clone + Debug + 'static>(make: impl FnOnce(Reply<T>) -> E) -> Eff<R, T>
where
    R: Member<E, Ix>,
{
    Eff::of(Kind::Perform(Op::of::<E, T>(make(Reply(PhantomData)))))
}

impl<R, A: 'static> Eff<R, A> {
    /// `f` given the value: its program the rest — at the answers met (`Bind`)
    pub fn flat_map<N: Program, F>(self, f: F) -> <Self as Bind<N>>::Out
    where
        F: Fn(A) -> N + 'static,
        Self: Bind<N>,
    {
        let frame = Frame::new(move |v: Val| f(v.take::<A>()).into_node());
        <Self as Bind<N>>::Out::from_node(Node(bound(self.node.0, frame)))
    }

    /// `f` of the value
    pub fn map<B: 'static>(self, f: impl Fn(A) -> B + 'static) -> Eff<R, B> {
        let frame = Frame::new(move |v: Val| Node(Kind::Return(Val::new(f(v.take::<A>())))));
        Eff::of(bound(self.node.0, frame))
    }

    /// a PLAIN effect `E` handled by `h`, wherever it is in the row: the row without it, `T`, the handler's
    pub fn handle<E: Effect, Ix, B: 'static>(self, h: Handler<E, R::Rest, A, B>) -> Eff<R::Rest, B>
    where
        E::Kind: PlainKind,
        R: Member<E, Ix>,
    {
        Eff::of(Kind::Handle(Box::new(self.node), h.level))
    }

    /// a SENSITIVE effect handled — only under its audit: `Audited<S>` in the row, put there by `audit` or `observe`
    pub fn handle_audited<S: Effect<Kind = Sensitive>, Ix, B: 'static>(
        self,
        h: Handler<S, R::Rest, A, B>,
    ) -> Eff<R::Rest, B>
    where
        R: Member<Audited<S>, Ix>,
    {
        Eff::of(Kind::Handle(Box::new(self.node), h.level))
    }

    /// a PLAIN effect INTERCEPTED for this program: its operations to `h`'s clauses, which run outside it — their own
    /// operations of `E` reach the handler further out. The row the same
    pub fn intercept<E: Effect, Ix, B: 'static>(self, h: Handler<E, R, A, B>) -> Eff<R, B>
    where
        E::Kind: PlainKind,
        R: Member<E, Ix>,
    {
        Eff::of(Kind::Handle(Box::new(self.node), h.level))
    }

    /// a SENSITIVE effect AUDITED: its operations to `i`'s clauses — each passed on (`Pass`), changed, or refused;
    /// outside, `Audited<S>` in the row: no program there performs `S`, and only `handle_audited` handles it
    pub fn audit<S: Effect<Kind = Sensitive>, Ix, B: 'static>(
        self,
        i: Interceptor<S, <R as Mark<S, Ix>>::Out, A, B>,
    ) -> Eff<<R as Mark<S, Ix>>::Out, B>
    where
        R: Mark<S, Ix>,
    {
        Eff::of(Kind::Handle(Box::new(self.node), i.level))
    }

    /// every operation OBSERVED on its way out, whatever its effect: `o`'s clause given it — told, passed on or
    /// refused. Outside, every sensitive effect of the row audited
    pub fn observe<B: 'static>(self, o: Observer<<R as AuditAll>::Out, A, B>) -> Eff<<R as AuditAll>::Out, B>
    where
        R: AuditAll,
    {
        Eff::of(Kind::Handle(Box::new(self.node), o.level))
    }
}

impl<R, A: 'static> Eff<R, A> {
    /// ENSURING: `fin` run when this program is left — with its value (kept), a failure (raised on after it), an abort
    /// past it, a cancel. Under a rest resumed twice, twice
    pub fn ensure(self, fin: impl Fn() -> Eff<R, ()> + 'static) -> Eff<R, A>
    where
        A: Clone,
    {
        let g = crate::machine::GuardRt {
            finalizer: Some(Rc::new(move || fin().node)),
            recover: None,
            passing: None,
            copy: copy_as::<A>,
        };
        Eff::of(Kind::Handle(Box::new(self.node), crate::machine::Level::Guard(Rc::new(g))))
    }

    /// RECOVERING: a failure raised in this program — user code's panic, a host's `fail` — given to `f`, its program in
    /// place of this one's. Not an effect's raise (`Throws`): that is its handler's
    pub fn recover(self, f: impl Fn(Failure) -> Eff<R, A> + 'static) -> Eff<R, A> {
        let g = crate::machine::GuardRt {
            finalizer: None,
            recover: Some(Box::new(move |e| f(e).node)),
            passing: None,
            copy: no_copy,
        };
        Eff::of(Kind::Handle(Box::new(self.node), crate::machine::Level::Guard(Rc::new(g))))
    }

    /// UNCANCELABLE: a cancel asked inside waits for this program to be left
    pub fn uncancelable(self) -> Eff<R, A> {
        Eff::of(Kind::Handle(Box::new(self.node), crate::machine::Level::Mask(Rc::new(std::cell::Cell::new(false)))))
    }

    /// GUARDED by `g`: its finalizer when left, its passing told of each operation going out
    pub fn guard(self, g: Guard<R, A>) -> Eff<R, A>
    where
        A: Clone,
    {
        let rt = crate::machine::GuardRt { finalizer: g.finalizer, recover: None, passing: g.passing, copy: copy_as::<A> };
        Eff::of(Kind::Handle(Box::new(self.node), crate::machine::Level::Guard(Rc::new(rt))))
    }
}

impl<R, A> Eff<R, A> {
    /// the same program over the row with `E` first: it performs none of `E`, so it is one of that row too (an
    /// operation finds its handler by its effect, not by its place)
    pub fn widen<E>(self) -> Eff<crate::row::Cons<E, R>, A> {
        Eff::of(self.node.0)
    }
}

pub(crate) fn no_copy(_: &Val) -> Val {
    unreachable!("okay: a recovering guard keeps no value")
}

/// what a guard's passing gives back: the operation sent in its place, and what to do each time its answer comes back
pub type PassingOut = (AnyOp, Option<Rc<dyn Fn()>>);

/// A GUARD: a finalizer, run when the program under it is left; a passing, told of each operation going out of it —
/// its own effect or another's — and sending one in its place
pub struct Guard<R, A> {
    finalizer: Option<Rc<dyn Fn() -> Node>>,
    passing: Option<crate::machine::Passing>,
    _t: PhantomData<fn() -> (R, A)>,
}

impl<R, A> Default for Guard<R, A> {
    fn default() -> Self {
        Guard { finalizer: None, passing: None, _t: PhantomData }
    }
}

impl<R, A> Guard<R, A> {
    pub fn new() -> Self {
        Guard::default()
    }

    /// run when the program under it is left
    pub fn finalizer(mut self, fin: impl Fn() -> Eff<R, ()> + 'static) -> Self {
        self.finalizer = Some(Rc::new(move || fin().node));
        self
    }

    /// told of each operation going out: the one sent in its place, and what to do at each resumption
    pub fn passing(mut self, f: impl Fn(AnyOp) -> PassingOut + 'static) -> Self {
        crate::machine::PASSING.with(|p| p.set(true));
        self.passing = Some(Rc::new(move |op: Op| {
            let (o, back) = f(AnyOp(op));
            (o.0, back)
        }));
        self
    }
}

impl<A: 'static> Eff<Nil, A> {
    /// the value: a program of no effect run
    pub fn run(self) -> A {
        crate::machine::run(self.node).take::<A>()
    }
}


/// THE REPLY to an operation: the only way to its value — `give` it (a handler answering in place), or `resume` the
/// rest with it (a handler taking the rest). Made by `perform` alone, one for each operation
pub struct Reply<T>(PhantomData<fn(T) -> T>);

impl<T> Clone for Reply<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Reply<T> {}

impl<T> Debug for Reply<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("_")
    }
}

/// the value given to an operation, erased: made by a `Reply`
pub struct Answer(pub(crate) Val);

impl<T> Reply<T> {
    /// a reply, made by a performer: the machine's `perform`, level 1's operations
    pub(crate) fn new() -> Self {
        Reply(PhantomData)
    }
}

impl<T: 'static> Reply<T> {
    /// the operation's value `t`
    pub fn give(self, t: T) -> Answer {
        Answer(Val::new(t))
    }

    /// the rest `k` resumed with `t`: a program over the row outside the handler, of its answer
    pub fn resume<R, B>(self, k: &crate::handler::K<R, B>, t: T) -> Eff<R, B> {
        Eff::of(Kind::Resume(k.k.clone(), Val::new(t)))
    }
}

/// A FAILURE: what user code panicked with, or a host failed an operation with — its payload, and a message of it
pub struct Failure(pub(crate) Box<dyn std::any::Any + Send>);

impl Failure {
    /// a failure of any payload: a message, an error
    pub fn new(payload: impl std::any::Any + Send) -> Failure {
        Failure(Box::new(payload))
    }
    /// its message: the payload if a string, else a word of its kind
    pub fn message(&self) -> String {
        self.0
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| self.0.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| String::from("a failure"))
    }
    /// what a panic was caught with (`catch_unwind`'s error)
    pub fn from_panic(p: Box<dyn std::any::Any + Send>) -> Failure {
        Failure(p)
    }
    /// the payload, if it is a `P`
    pub fn payload<P: 'static>(&self) -> Option<&P> {
        self.0.downcast_ref::<P>()
    }
}

impl Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failure({:?})", self.message())
    }
}

/// no value: an operation that never comes back (`fail`, `raise`)
#[derive(Clone, Debug)]
pub enum Never {}

/// AN OPERATION SEEN BY AN OBSERVER, whatever its effect: what it is, and the way on
#[derive(Clone)]
pub struct AnyOp(pub(crate) Op);

impl AnyOp {
    /// its effect, the type's name without its path: `Reader<i64>`
    pub fn effect(&self) -> String {
        short((self.0.info.name)())
    }

    /// the operation as written: `Put(6, _)`
    pub fn describe(&self) -> String {
        (self.0.info.describe)(&self.0.value)
    }

    /// the operation, if it is of the effect `E`
    pub fn op<E: Effect>(&self) -> Option<E> {
        (self.0.effect() == TypeId::of::<E>()).then(|| (self.0.info.copy)(&self.0.value).take::<E>())
    }

    /// another operation of the same effect in its place (a guard's passing): of another effect, refused
    pub fn with<E: Effect>(&self, e: E) -> AnyOp {
        assert_eq!(self.0.effect(), TypeId::of::<E>(), "okay: an operation replaced by one of another effect");
        AnyOp(Op { info: self.0.info, value: Val::new(e) })
    }

    /// passed on, to the handler further out: its answer to the rest `k`
    pub fn proceed<T, B>(self, k: &crate::handler::K<T, B>) -> Eff<T, B> {
        let k0 = k.k.clone();
        let frame = Frame::new(move |v: Val| Node(Kind::Resume(k0.clone(), v)));
        Eff::of(Kind::Inject(self.0, frame))
    }

    /// passed on, its answer told to `after` — a program run before the rest gets it, each time it gets it (a rest
    /// resumed twice, twice)
    pub fn proceed_then<T: 'static, B>(
        self,
        k: &crate::handler::K<T, B>,
        after: impl Fn(String) -> Eff<T, ()> + 'static,
    ) -> Eff<T, B> {
        let k0 = k.k.clone();
        let describe = self.0.info.describe_answer;
        let copy = self.0.info.copy_answer;
        let after = Rc::new(after);
        let frame = Frame::new(move |v: Val| {
            let told = (*after)(describe(&v)).node;
            let k0 = k0.clone();
            let v = Rc::new(v);
            let back = Frame::new(move |_| Node(Kind::Resume(k0.clone(), copy(&v))));
            Node(bound(told.0, back))
        });
        Eff::of(Kind::Inject(self.0, frame))
    }
}

/// a type's name, its paths dropped: `okay::effects::Reader<i64>` to `Reader<i64>`
pub(crate) fn short(name: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
        } else if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            out.push_str(&word);
            word.clear();
            out.push(c);
        }
    }
    out.push_str(&word);
    out
}

/// THE WAY ON for an auditor of `S`: its operations performed further out, typed — given to the clauses of an
/// interceptor of `S` alone, so no other program performs `S` past the audit
pub struct Pass<S>(PhantomData<fn() -> S>);

impl<S> Clone for Pass<S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S> Copy for Pass<S> {}

impl<S: Effect> Pass<S> {
    pub(crate) fn new() -> Self {
        Pass(PhantomData)
    }

    /// an operation of `S` performed past the audit — the one asked, or another
    pub fn perform<T, Ix, X: Clone + Debug + 'static>(self, make: impl FnOnce(Reply<X>) -> S) -> Eff<T, X>
    where
        T: Member<Audited<S>, Ix>,
    {
        Eff::of(Kind::Perform(Op::of::<S, X>(make(Reply(PhantomData)))))
    }
}

#[cfg(test)]
mod sizes {
    /// the node moved at each step of the machine: its size is its speed (an operation's effect in its table, not in
    /// the node: `Kind` 96 → 80 bytes, the state lanes −11–15%). A variant bigger than these: measure first
    #[test]
    fn a_node_is_small() {
        use std::mem::size_of;
        assert_eq!((size_of::<super::Val>(), size_of::<super::Op>(), size_of::<super::Frame>()), (40, 48, 32));
        assert!(size_of::<super::Kind>() <= 80, "Kind {} bytes", size_of::<super::Kind>());
    }
}
