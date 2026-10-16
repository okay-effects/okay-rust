//! STAGE 2, DELIMITED CONTROL WITH ANSWER-TYPE MODIFICATION (okay-core's `Cont[R, I, O, A]`, Materzok–Biernacki's
//! shift0 and `$`, Asai–Kameyama's typing): a program of `A` under a `$` over a row with `Control` in it, `I` the
//! answer its rest gives, `O` the answer it makes. A shift0 takes the rest up to the nearest `$` — the `$` with it —
//! as `k`, a function to a program outside; its body's answer, a program outside too, is the `$`'s. A program of
//! effects (`Eff`, no answers of its own) meets one of control at any answers: `flat_map` lifts it.
//!
//! WHAT THE TYPES REFUSE (okay-core's TestGuarantees on control, TestPState, TestAtmUses), each beside one taken:
//!
//! **A shift0: only under a `$` in its row — the nearest.**
//!
//! ```
//! use okay::*; use okay::cont::shift0;
//! let _: Cont<Reset, Eff<Nil, i64>, Eff<Nil, i64>, i64> = shift0::<Reset, Nil, _, i64, Eff<Nil, i64>, Eff<Nil, i64>>(|k| k(1));
//! ```
//! ```compile_fail
//! use okay::*; use okay::cont::shift0; use okay::effects::*;
//! // no `$` in the row
//! let _: Cont<row![Reader<i64>], Eff<Nil, i64>, Eff<Nil, i64>, i64> =
//!     shift0::<row![Reader<i64>], Nil, _, i64, Eff<Nil, i64>, Eff<Nil, i64>>(|k| k(1));
//! ```
//!
//! **A pstate: a `get` of the wrong type does not compile — the answer types do not meet.**
//!
//! ```
//! use okay::*; use okay::cont::{pstate, state};
//! let p = pstate::<Nil, i64, i64, i64>(1, |st| st.put::<i64, i64>(41).flat_map(|_| state::<Nil, (i64, i64)>().get::<i64>()));
//! assert_eq!(p.run(), (41, 41));
//! ```
//! ```compile_fail
//! use okay::*; use okay::cont::{pstate, state};
//! let p = pstate::<Nil, i64, i64, i64>(1, |st| st.put::<i64, String>("s".into()).flat_map(|_| state::<Nil, (i64, i64)>().get::<i64>()));
//! ```
//!
//! **A protocol: a query before the login does not compile; a session left open does not compile.**
//!
//! ```
//! use okay::*; use okay::cont::{pstate, state, P};
//! #[derive(Clone)] struct Closed; #[derive(Clone)] struct Open; #[derive(Clone)] struct Authed;
//! type F = (Closed, i64);
//! let st = || state::<Nil, F>();
//! let connect = move || st().get::<Closed>().flat_map(move |_| st().put::<Closed, Open>(Open));
//! let login = move || st().get::<Open>().flat_map(move |_| st().put::<Open, Authed>(Authed));
//! let query = move || st().get::<Authed>().map(|_| 8i64);
//! let close = move || st().get::<Authed>().flat_map(move |_| st().put::<Authed, Closed>(Closed));
//! let s = pstate::<Nil, Closed, Closed, i64>(Closed, move |_| {
//!     connect().flat_map(move |_| login()).flat_map(move |_| query()).flat_map(move |n| close().map(move |_| n))
//! });
//! assert_eq!(s.run().1, 8);
//! ```
//! ```compile_fail
//! use okay::*; use okay::cont::{pstate, state, P};
//! #[derive(Clone)] struct Closed; #[derive(Clone)] struct Open; #[derive(Clone)] struct Authed;
//! type F = (Closed, i64);
//! let st = || state::<Nil, F>();
//! let connect = move || st().get::<Closed>().flat_map(move |_| st().put::<Closed, Open>(Open));
//! let query = move || st().get::<Authed>().map(|_| 8i64);
//! let close = move || st().get::<Authed>().flat_map(move |_| st().put::<Authed, Closed>(Closed));
//! // no login: the query asks Authed of a state that is Open
//! let s = pstate::<Nil, Closed, Closed, i64>(Closed, move |_| {
//!     connect().flat_map(move |_| query()).flat_map(move |n| close().map(move |_| n))
//! });
//! ```
//! ```compile_fail
//! use okay::*; use okay::cont::{pstate, state, P};
//! #[derive(Clone)] struct Closed; #[derive(Clone)] struct Open; #[derive(Clone)] struct Authed;
//! type F = (Closed, i64);
//! let st = || state::<Nil, F>();
//! let connect = move || st().get::<Closed>().flat_map(move |_| st().put::<Closed, Open>(Open));
//! let login = move || st().get::<Open>().flat_map(move |_| st().put::<Open, Authed>(Authed));
//! let query = move || st().get::<Authed>().map(|_| 8i64);
//! // never closed: the session must end Closed
//! let s = pstate::<Nil, Closed, Closed, i64>(Closed, move |_| {
//!     connect().flat_map(move |_| login()).flat_map(move |_| query())
//! });
//! ```

use std::marker::PhantomData;
use std::rc::Rc;

use crate::eff::{Bind, Eff, Frame, Kind, Node, Program, Val};
use crate::handler::Handler;
use crate::machine::Level;
use crate::row::{Delim, Effect, Member, Nearest, PlainKind};

/// A PROGRAM OF CONTROL over the row `R`: of the value `A`, the answer of its rest `I`, its own answer `O`
pub struct Cont<R, I, O, A> {
    pub(crate) node: Node,
    _t: Types<R, I, O, A>,
}

/// the types of a program of control, the compiler's only
type Types<R, I, O, A> = PhantomData<fn() -> (R, I, O, A)>;

impl<R, I, O, A> Cont<R, I, O, A> {
    pub(crate) fn of(kind: Kind) -> Self {
        Cont { node: Node(kind), _t: PhantomData }
    }
}

impl<R, I, O, A> Program for Cont<R, I, O, A> {
    fn into_node(self) -> Node {
        self.node
    }
    fn from_node(n: Node) -> Self {
        Cont { node: n, _t: PhantomData }
    }
}

/// a program over the row `T`: of effects or of control — what a shift0's `k` gives and what a `$` answers
pub trait Over<T>: Program {}
impl<T, A> Over<T> for Eff<T, A> {}
impl<T, I, O, A> Over<T> for Cont<T, I, O, A> {}

// THE ANSWERS MET at a bind: control after control — the first's rest the second; a program of effects either side
// at the other's answers

impl<R, T, O, X, I, A> Bind<Cont<R, I, T, A>> for Cont<R, T, O, X> {
    type Out = Cont<R, I, O, A>;
}

impl<R, A, I, O, B> Bind<Cont<R, I, O, B>> for Eff<R, A> {
    type Out = Cont<R, I, O, B>;
}

impl<R, T, O, X, B> Bind<Eff<R, B>> for Cont<R, T, O, X> {
    type Out = Cont<R, T, O, B>;
}

/// `a` under any `$`, at any answers
pub fn pure_c<R, S, A: 'static>(a: A) -> Cont<R, S, S, A> {
    Cont::of(Kind::Return(Val::new(a)))
}

impl<R, A: 'static> Eff<R, A> {
    /// a program of effects as one of control, at the answers `S` of its place
    pub fn cont<S>(self) -> Cont<R, S, S, A> {
        Cont::of(self.node.0)
    }
}

impl<R, I, O, A: 'static> Cont<R, I, O, A> {
    /// `f` given the value: its program the rest, at the answers met
    pub fn flat_map<N: Program, F>(self, f: F) -> <Self as Bind<N>>::Out
    where
        F: Fn(A) -> N + 'static,
        Self: Bind<N>,
    {
        let frame = Frame::new(move |v: Val| f(v.take::<A>()).into_node());
        <Self as Bind<N>>::Out::from_node(Node(Kind::Bind(Box::new(self.node), frame)))
    }

    /// `f` of the value
    pub fn map<B: 'static>(self, f: impl Fn(A) -> B + 'static) -> Cont<R, I, O, B> {
        let frame = Frame::new(move |v: Val| Node(Kind::Return(Val::new(f(v.take::<A>())))));
        Cont::of(Kind::Bind(Box::new(self.node), frame))
    }

    /// an effect handled under the `$` by a handler LEAVING THE REST (answering, stateful, translating): a shift0
    /// under it takes it into its `k`. One taking the rest is refused: its delimiter would be one among the program's
    pub fn handle<E: Effect, Ix, B: 'static>(self, h: Passing<E, R::Rest, A, B>) -> Cont<R::Rest, I, O, B>
    where
        E::Kind: PlainKind,
        R: Member<E, Ix>,
    {
        Cont::of(Kind::Handle(Box::new(self.node), h.0.level))
    }
}

/// A HANDLER LEAVING THE REST: one that may be under a `$` — answering in place, with a state, translating
pub struct Passing<E, T, A, B>(Handler<E, T, A, B>);

impl<E: Effect, T, A: 'static, B: 'static> Passing<E, T, A, B> {
    pub fn answering(value: impl Fn(E) -> crate::Answer + 'static, ret: impl Fn(A) -> B + 'static) -> Self {
        Passing(Handler::answering(value, ret))
    }
    pub fn stateful<S: Clone + 'static>(
        s0: S,
        step: impl Fn(&mut S, E) -> crate::Answer + 'static,
        ret: impl Fn(S, A) -> B + 'static,
    ) -> Self {
        Passing(Handler::stateful(s0, step, ret))
    }
}

impl<E: Effect, T, A: 'static> Passing<E, T, A, A> {
    pub fn translating(apply: impl Fn(E) -> Eff<T, crate::Answer> + 'static) -> Self {
        Passing(Handler::translating(apply))
    }
}

impl<E, T, A, B> From<Passing<E, T, A, B>> for Handler<E, T, A, B> {
    fn from(p: Passing<E, T, A, B>) -> Self {
        p.0
    }
}

/// THE REST OF A SHIFT0: its value in, a program outside the `$` out — called any number of times, each its own
pub type K1<A, P> = Rc<dyn Fn(A) -> P>;

/// SHIFT0: the rest up to the nearest `$` over `T` (the `$` with it) as `k`; `f`'s answer, a program over `T`, the `$`'s
pub fn shift0<R, T, Ix, A: 'static, P, O>(f: impl Fn(K1<A, P>) -> O + 'static) -> Cont<R, P, O, A>
where
    R: Nearest<T, Ix>,
    P: Over<T> + 'static,
    O: Over<T>,
{
    Cont::of(Kind::Shift0(Rc::new(move |k| {
        let k: K1<A, P> = Rc::new(move |a: A| P::from_node(Node(Kind::Resume(k.clone(), Val::new(a)))));
        f(k).into_node()
    })))
}

/// A `$` over `T`: `body` under it; its value through `ret`, or a shift0's body — the answer, a program over `T`
pub fn dollar<T, I, P, A: 'static>(body: Cont<Delim<T>, I, P, A>, ret: impl Fn(A) -> I + 'static) -> P
where
    I: Over<T>,
    P: Over<T>,
{
    let ret = Rc::new(move |v: Val| ret(v.take::<A>()).into_node());
    P::from_node(Node(Kind::Handle(Box::new(body.node), Level::Dollar(ret))))
}

/// A RESET over `T`: a `$` whose answer is the body's value, as a program of effects over `T`
pub fn reset<T: 'static, S: 'static>(body: Cont<Delim<T>, Eff<T, S>, Eff<T, S>, S>) -> Eff<T, S> {
    dollar(body, crate::pure)
}

/// PSTATE: a state whose TYPE may change, carried by the ANSWER TYPE — Danvy–Filinski's state passing, the type moved
/// by Asai–Kameyama's answer-type modification, under a `$`. Not an effect of the row: `get` and `put` are shift0s up to
/// the run's `$`, whose answer is the rest of the run given the state. A `put` of another type moves the answer from
/// `At<S2>` to `At<S>`: a `get` of the wrong type, a `put` out of order, does not compile
pub struct PState<R, F>(PhantomData<fn() -> (R, F)>);

/// the rest of the run given the state `S`: a program of the run's value `F`
pub type At<R, S, F> = Rc<dyn Fn(S) -> Eff<R, F>>;

/// a program answering the rest of the run at the state `S`
pub type P<R, S, F> = Eff<R, At<R, S, F>>;

// the answers in these types are the point: a state's type before and after each step
#[allow(clippy::type_complexity)]
impl<R: 'static, F: 'static> PState<R, F> {
    /// the state, its type `S` unchanged: `shift0(k => s => k(s)(s))`
    pub fn get<S: Clone + 'static>(&self) -> Cont<Delim<R>, P<R, S, F>, P<R, S, F>, S> {
        shift0::<Delim<R>, R, crate::row::Here, S, P<R, S, F>, P<R, S, F>>(|k| {
            crate::pure(Rc::new(move |s: S| {
                let again = s.clone();
                k(s).flat_map(move |f: At<R, S, F>| f(again.clone()))
            }) as At<R, S, F>)
        })
    }

    /// a state of another type, `S2`, for the rest; the state before, `S`: `shift0(k => _ => k(())(s2))`
    pub fn put<S: 'static, S2: Clone + 'static>(&self, s2: S2) -> Cont<Delim<R>, P<R, S2, F>, P<R, S, F>, ()> {
        shift0::<Delim<R>, R, crate::row::Here, (), P<R, S2, F>, P<R, S, F>>(move |k| {
            let s2 = s2.clone();
            crate::pure(Rc::new(move |_: S| {
                let s2 = s2.clone();
                k(()).flat_map(move |f: At<R, S2, F>| f(s2.clone()))
            }) as At<R, S, F>)
        })
    }
}

/// the state's handle: a type, no value of its own — made anywhere a step needs it
pub fn state<R, F>() -> PState<R, F> {
    PState(PhantomData)
}

/// `pstate(s0, body)`: `body` run from the state `s0`, of type `S0`, to one of type `S1`, under its `$` over the row
/// `R`: the last state and the value, a program of effects over `R`. The state given to the body: its steps take it
#[allow(clippy::type_complexity)]
pub fn pstate<R: 'static, S0: Clone + 'static, S1: 'static, W: Clone + 'static>(
    s0: S0,
    body: impl FnOnce(&PState<R, (S1, W)>) -> Cont<Delim<R>, P<R, S1, (S1, W)>, P<R, S0, (S1, W)>, W>,
) -> Eff<R, (S1, W)> {
    let st = PState(PhantomData);
    let ran: P<R, S0, (S1, W)> = dollar(body(&st), |w: W| {
        crate::pure(Rc::new(move |s1: S1| crate::pure((s1, w.clone()))) as At<R, S1, (S1, W)>)
    });
    ran.flat_map(move |f: At<R, S0, (S1, W)>| f(s0.clone()))
}
