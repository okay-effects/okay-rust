//! HANDLERS, values of three forms, each over the row `T` outside it, of the body's value `A` to its own `B` — as
//! okay-one's: answering in place (`answering`), with a state (`stateful`), taking the rest (`deep`). Typed here,
//! erased into a level of the machine.

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::eff::{Answer, AnyOp, Eff, Node, Op, Pass, Val};
use crate::machine::{AbortingRt, AnsweringRt, DeepRt, K0, Level, ObserveRt, StatefulRt};
use crate::row::Effect;

/// the types of a handler, the compiler's only
type Types<E, T, A, B> = PhantomData<fn() -> (E, T, A, B)>;

/// A HANDLER of `E`, over the row `T` outside it, of the body's `A` to `B`
pub struct Handler<E, T, A, B> {
    pub(crate) level: Level,
    _t: Types<E, T, A, B>,
}

/// THE REST of a handler taking it, data: resumed by an operation's `Reply`, as many times as wanted — each time its
/// levels put back as they were when it was taken, a state's included
pub struct K<T, B> {
    pub(crate) k: K0,
    _t: PhantomData<fn() -> (T, B)>,
}

impl<T, B> Clone for K<T, B> {
    fn clone(&self) -> Self {
        K { k: self.k.clone(), _t: PhantomData }
    }
}

impl<E: Effect, T, A: 'static, B: 'static> Handler<E, T, A, B> {
    /// ANSWERING IN PLACE: each operation its value at once, the rest never taken; `ret` of the body's value
    pub fn answering(value: impl Fn(E) -> Answer + 'static, ret: impl Fn(A) -> B + 'static) -> Self {
        let rt = AnsweringRt {
            value: Box::new(move |op: Val| value(op.take::<E>()).0),
            ret: Box::new(move |v: Val| Val::new(ret(v.take::<A>()))),
        };
        Handler { level: Level::Answering(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }

    /// WITH A STATE `S`, from `s0`: each operation its value and the state moved (`step`), in place; `ret` of the
    /// last state and the body's value. Taken into a rest, the state as it was then: each resumption its own
    pub fn stateful<S: Clone + 'static>(
        s0: S,
        step: impl Fn(&mut S, E) -> Answer + 'static,
        ret: impl Fn(S, A) -> B + 'static,
    ) -> Self {
        let rt = StatefulRt {
            step: Box::new(move |s: Rc<dyn Any>, op: Val| {
                let mut s = state::<S>(s);
                // the state written in place when no rest holds it, copied when one does
                let Answer(v) = step(Rc::make_mut(&mut s), op.take::<E>());
                (s as Rc<dyn Any>, v)
            }),
            ret: Box::new(move |s: Rc<dyn Any>, v: Val| {
                let s = Rc::try_unwrap(state::<S>(s)).unwrap_or_else(|s| (*s).clone());
                Val::new(ret(s, v.take::<A>()))
            }),
        };
        Handler { level: Level::Stateful(TypeId::of::<E>(), Rc::new(rt), Rc::new(s0)), _t: PhantomData }
    }

    /// DROPPING THE REST: each operation's program in place of the body's — the rest never taken, no `k` made (an
    /// error, an early answer); `ret` of the body's value
    pub fn aborting(ret: impl Fn(A) -> Eff<T, B> + 'static, abort: impl Fn(E) -> Eff<T, B> + 'static) -> Self {
        let rt = AbortingRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            abort: Box::new(move |op: Val| abort(op.take::<E>()).node),
        };
        Handler { level: Level::Aborting(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }

    /// TAKING THE REST: each operation its clause, given the rest `k` — resumed none, once or many times, by the
    /// operation's `Reply`; the clause a program over the row outside. `ret` of the body's value
    pub fn deep(
        ret: impl Fn(A) -> Eff<T, B> + 'static,
        apply: impl Fn(E, K<T, B>) -> Eff<T, B> + 'static,
    ) -> Self {
        let rt = DeepRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            apply: Box::new(move |op: Val, k: K0| -> Node {
                apply(op.take::<E>(), K { k, _t: PhantomData }).node
            }),
        };
        Handler { level: Level::Deep(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }
}

/// AN INTERCEPTOR of the sensitive effect `S`: its clauses given each operation, the rest `k`, and the way on
/// (`Pass`) — over the row `T` outside the audit, `Audited<S>` in it; `ret` of the body's value
pub struct Interceptor<S, T, A, B> {
    pub(crate) level: Level,
    _t: Types<S, T, A, B>,
}

impl<S: Effect, T, A: 'static, B: 'static> Interceptor<S, T, A, B> {
    pub fn new(
        ret: impl Fn(A) -> Eff<T, B> + 'static,
        apply: impl Fn(S, K<T, B>, Pass<S>) -> Eff<T, B> + 'static,
    ) -> Self {
        let rt = DeepRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            apply: Box::new(move |op: Val, k: K0| -> Node {
                apply(op.take::<S>(), K { k, _t: PhantomData }, Pass::new()).node
            }),
        };
        Interceptor { level: Level::Deep(TypeId::of::<S>(), Rc::new(rt)), _t: PhantomData }
    }
}

/// AN OBSERVER of every operation on its way out, whatever its effect: its clause given the operation (`AnyOp`) and
/// the rest `k` — over the row `T` outside it; `ret` of the body's value
pub struct Observer<T, A, B> {
    pub(crate) level: Level,
    _t: Types<(), T, A, B>,
}

impl<T, A: 'static, B: 'static> Observer<T, A, B> {
    pub fn new(
        ret: impl Fn(A) -> Eff<T, B> + 'static,
        apply: impl Fn(AnyOp, K<T, B>) -> Eff<T, B> + 'static,
    ) -> Self {
        let rt = ObserveRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            apply: Box::new(move |op: Op, k: K0| -> Node { apply(AnyOp(op), K { k, _t: PhantomData }).node }),
        };
        Observer { level: Level::Observe(Rc::new(rt)), _t: PhantomData }
    }
}

/// a state taken back at its type
fn state<S: 'static>(s: Rc<dyn Any>) -> Rc<S> {
    match s.downcast::<S>() {
        Ok(s) => s,
        Err(_) => panic!("okay: a state of another type than its handler's"),
    }
}
