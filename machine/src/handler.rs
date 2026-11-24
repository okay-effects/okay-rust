//! HANDLERS, values of three forms, each over the row `R` outside it, of the body's value `A` to its own `B` — as
//! okay-one's: in place (`in_place`), with a state (`stateful`), general, taking the rest (`new`). Typed here,
//! erased into a level of the machine.

use std::any::{Any, TypeId};
use std::marker::PhantomData;
use std::rc::Rc;

use crate::eff::{Answer, AnyOp, Prog, Node, Op, Pass, Val};
use crate::machine::{AbortingRt, AnsweringRt, DeepRt, K0, Level, ObserveRt, StatefulRt};
use crate::row::Effect;

/// the types of a handler, the compiler's only
type Types<E, A, B, R> = PhantomData<fn() -> (E, A, B, R)>;

/// A HANDLER of `E`, over the row `R` outside it, of the body's `A` to `B`
pub struct Handler<E, A, B, R> {
    pub(crate) level: Level,
    _t: Types<E, A, B, R>,
}

/// THE REST of a handler taking it, data: resumed by an operation's `Reply`, as many times as wanted — each time its
/// levels put back as they were when it was taken, a state's included
pub struct K<B, R> {
    pub(crate) k: K0,
    _t: PhantomData<fn() -> (B, R)>,
}

impl<B, R> Clone for K<B, R> {
    fn clone(&self) -> Self {
        K { k: self.k.clone(), _t: PhantomData }
    }
}

impl<E: Effect, R, A: 'static, B: 'static> Handler<E, A, B, R> {
    /// ANSWERING IN PLACE: each operation its value at once, the rest never taken; `ret` of the body's value
    pub fn in_place(value: impl Fn(E) -> Answer + 'static, ret: impl Fn(A) -> B + 'static) -> Self {
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
    pub fn abort(ret: impl Fn(A) -> Prog<B, R> + 'static, abort: impl Fn(E) -> Prog<B, R> + 'static) -> Self {
        let rt = AbortingRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            abort: Box::new(move |op: Val| abort(op.take::<E>()).node),
        };
        Handler { level: Level::Aborting(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }

    /// TAKING THE REST: each operation its clause, given the rest `k` — resumed none, once or many times, by the
    /// operation's `Reply`; the clause a program over the row outside. `ret` of the body's value
    pub fn new(
        ret: impl Fn(A) -> Prog<B, R> + 'static,
        apply: impl Fn(E, K<B, R>) -> Prog<B, R> + 'static,
    ) -> Self {
        let rt = DeepRt {
            ret: Box::new(move |v: Val| ret(v.take::<A>()).node),
            apply: Box::new(move |op: Val, k: K0| -> Node {
                apply(op.take::<E>(), K { k, _t: PhantomData }).node
            }),
        };
        Handler { level: Level::Deep(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }

    /// the old name of [`Handler::in_place`]
    #[deprecated(note = "the family's name is `Handler::in_place`")]
    pub fn answering(value: impl Fn(E) -> Answer + 'static, ret: impl Fn(A) -> B + 'static) -> Self {
        Self::in_place(value, ret)
    }

    /// the old name of [`Handler::abort`]
    #[deprecated(note = "the family's name is `Handler::abort`")]
    pub fn aborting(ret: impl Fn(A) -> Prog<B, R> + 'static, abort: impl Fn(E) -> Prog<B, R> + 'static) -> Self {
        Self::abort(ret, abort)
    }

    /// the old name of [`Handler::new`] (the general handler)
    #[deprecated(note = "the family's name is `Handler::new`")]
    pub fn deep(
        ret: impl Fn(A) -> Prog<B, R> + 'static,
        apply: impl Fn(E, K<B, R>) -> Prog<B, R> + 'static,
    ) -> Self {
        Self::new(ret, apply)
    }
}

impl<E: Effect, R, A: 'static> Handler<E, A, A, R> {
    /// TRANSLATING: each operation answered by a program over the row outside (`r.give(x)` at its end) — the rest
    /// resumed once with it; a failure in that program raised where the operation was, inside the body, so the body's
    /// own recover sees it
    pub fn translating(apply: impl Fn(E) -> Prog<Answer, R> + 'static) -> Self {
        let rt = DeepRt {
            ret: Box::new(|v: Val| Node(crate::eff::Kind::Return(v))),
            apply: Box::new(move |op: Val, k: K0| -> Node {
                use crate::eff::Kind;
                let program = apply(op.take::<E>()).node;
                let back = k.clone();
                let fail = crate::machine::GuardRt {
                    finalizer: None,
                    recover: Some(Box::new(move |f| Node(crate::eff::rare(crate::eff::Rare::Fail(k.clone(), f, true))))),
                    passing: None,
                    copy: crate::eff::no_copy,
                };
                let guarded = Node(Kind::Handle(Box::new(program), Level::Guard(Rc::new(fail))));
                Node(Kind::Bind(
                    Box::new(guarded),
                    crate::eff::Frame::new(move |a: Val| Node(Kind::Resume(back.clone(), a.take::<Answer>().0))),
                ))
            }),
        };
        Handler { level: Level::Deep(TypeId::of::<E>(), Rc::new(rt)), _t: PhantomData }
    }
}

/// AN INTERCEPTOR of the sensitive effect `S`: its clauses given each operation, the rest `k`, and the way on
/// (`Pass`) — over the row `R` outside the audit, `Audited<S>` in it; `ret` of the body's value
pub struct Interceptor<S, A, B, R> {
    pub(crate) level: Level,
    _t: Types<S, A, B, R>,
}

impl<S: Effect, R, A: 'static, B: 'static> Interceptor<S, A, B, R> {
    pub fn new(
        ret: impl Fn(A) -> Prog<B, R> + 'static,
        apply: impl Fn(S, K<B, R>, Pass<S>) -> Prog<B, R> + 'static,
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
/// the rest `k` — over the row `R` outside it; `ret` of the body's value
pub struct Observer<A, B, R> {
    pub(crate) level: Level,
    _t: Types<(), A, B, R>,
}

impl<R, A: 'static, B: 'static> Observer<A, B, R> {
    pub fn new(
        ret: impl Fn(A) -> Prog<B, R> + 'static,
        apply: impl Fn(AnyOp, K<B, R>) -> Prog<B, R> + 'static,
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
