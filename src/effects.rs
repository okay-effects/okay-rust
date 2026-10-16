//! THE STANDARD EFFECTS and their handlers: reader, state, writer, choice, throws — okay-one's std.

use crate::eff::{perform, pure, Eff, Never, Reply};
use crate::handler::{Handler, K};
use std::fmt::Debug;

use crate::row::{Cons, Effect, Member, Plain};

// READER: a value asked

#[derive(Clone, Debug)]
pub enum Reader<V> {
    Ask(Reply<V>),
}
impl<V: Clone + Debug + 'static> Effect for Reader<V> {
    type Kind = Plain;
}

/// the value asked of the reader
pub fn ask<V: Clone + Debug + 'static, R: Member<Reader<V>, Ix>, Ix>() -> Eff<R, V> {
    perform(Reader::Ask)
}

/// each ask answered `v`, in place
pub fn reader<V: Clone + Debug + 'static, T, A: 'static>(v: V) -> Handler<Reader<V>, T, A, A> {
    Handler::answering(move |op| match op {
        Reader::Ask(r) => r.give(v.clone()),
    }, |a| a)
}

// STATE: a value read and written

#[derive(Clone, Debug)]
pub enum State<S> {
    Get(Reply<S>),
    Put(S, Reply<()>),
}
impl<S: Clone + Debug + 'static> Effect for State<S> {
    type Kind = Plain;
}

pub fn get<S: Clone + Debug + 'static, R: Member<State<S>, Ix>, Ix>() -> Eff<R, S> {
    perform(State::Get)
}

pub fn put<S: Clone + Debug + 'static, R: Member<State<S>, Ix>, Ix>(s: S) -> Eff<R, ()> {
    perform(|r| State::Put(s, r))
}

/// the state moved by `f`
pub fn modify<S: Clone + Debug + 'static, R: Member<State<S>, Ix> + 'static, Ix: 'static>(f: impl Fn(S) -> S + 'static) -> Eff<R, ()> {
    get::<S, R, Ix>().flat_map(move |s| put::<S, R, Ix>(f(s)))
}

/// the state from `s0`: the last state and the value
pub fn state<S: Clone + Debug + 'static, T, A: 'static>(s0: S) -> Handler<State<S>, T, A, (S, A)> {
    Handler::stateful(s0, |s: &mut S, op| match op {
        State::Get(r) => r.give(s.clone()),
        State::Put(n, r) => {
            *s = n;
            r.give(())
        }
    }, |s, a| (s, a))
}

// WRITER: values told

#[derive(Clone, Debug)]
pub enum Writer<W> {
    Tell(W, Reply<()>),
}
impl<W: Clone + Debug + 'static> Effect for Writer<W> {
    type Kind = Plain;
}

pub fn tell<W: Clone + Debug + 'static, R: Member<Writer<W>, Ix>, Ix>(w: W) -> Eff<R, ()> {
    perform(|r| Writer::Tell(w, r))
}

/// the told, in order, and the value
pub fn collect<W: Clone + Debug + 'static, T, A: 'static>() -> Handler<Writer<W>, T, A, (Vec<W>, A)> {
    Handler::stateful(Vec::new(), |s: &mut Vec<W>, op| match op {
        Writer::Tell(w, r) => {
            s.push(w);
            r.give(())
        }
    }, |s, a| (s, a))
}

// CHOICE: one of several, each path through the choices a value — multi-shot, the rest resumed for each

#[derive(Clone, Debug)]
pub enum Choose {
    /// one of `n`, its index
    Among(usize, Reply<usize>),
    /// none: the path dropped
    Fail(Reply<Never>),
}
impl Effect for Choose {
    type Kind = Plain;
}

/// one of the values, each in turn
pub fn among<A: Clone + 'static, R: Member<Choose, Ix>, Ix>(xs: Vec<A>) -> Eff<R, A> {
    perform(|r| Choose::Among(xs.len(), r)).map(move |i: usize| xs[i].clone())
}

/// true, then false
pub fn flip<R: Member<Choose, Ix>, Ix>() -> Eff<R, bool> {
    perform(|r| Choose::Among(2, r)).map(|i: usize| i == 0)
}

/// no value: this path dropped
pub fn fail<A: 'static, R: Member<Choose, Ix>, Ix>() -> Eff<R, A> {
    perform(Choose::Fail).map(|n: Never| match n {})
}

/// every value of the body, one per path, in order
pub fn all<T: 'static, A: Clone + 'static>() -> Handler<Choose, T, A, Vec<A>> {
    Handler::deep(|a| pure(vec![a]), |op, k| match op {
        Choose::Among(n, r) => paths(k, r, 0, n).map(|mut v: Vec<A>| {
            v.reverse();
            v
        }),
        Choose::Fail(_) => pure(Vec::new()),
    })
}

/// the paths from the `i`-th on, their values reversed: each path's added at the end, the last first — no list
/// copied but each path's own
fn paths<T: 'static, A: Clone + 'static>(k: K<T, Vec<A>>, r: Reply<usize>, i: usize, n: usize) -> Eff<T, Vec<A>> {
    if i == n {
        return pure(Vec::new());
    }
    r.resume(&k, i).flat_map(move |here: Vec<A>| {
        paths(k.clone(), r, i + 1, n).map(move |mut rest: Vec<A>| {
            rest.extend(here.iter().rev().cloned());
            rest
        })
    })
}

// THROWS: an error raised, the rest dropped

#[derive(Clone, Debug)]
pub enum Throws<E> {
    Raise(E, Reply<Never>),
}
impl<E: Clone + Debug + 'static> Effect for Throws<E> {
    type Kind = Plain;
}

pub fn raise<E: Clone + Debug + 'static, A: 'static, R: Member<Throws<E>, Ix>, Ix>(e: E) -> Eff<R, A> {
    perform(|r| Throws::Raise(e, r)).map(|n: Never| match n {})
}

/// the value, or the error raised
pub fn either<E: Clone + Debug + 'static, T, A: 'static>() -> Handler<Throws<E>, T, A, Result<A, E>> {
    Handler::aborting(|a| pure(Ok(a)), |op| match op {
        Throws::Raise(e, _) => pure(Err(e)),
    })
}

// EMIT: values given out one at a time — a generator (`k` kept in the answer, the next step) or a collection

#[derive(Clone, Debug)]
pub enum Emit<W> {
    Value(W, Reply<()>),
}
impl<W: Clone + Debug + 'static> Effect for Emit<W> {
    type Kind = Plain;
}

pub fn emit<W: Clone + Debug + 'static, R: Member<Emit<W>, Ix>, Ix>(w: W) -> Eff<R, ()> {
    perform(|r| Emit::Value(w, r))
}

/// A GENERATOR: the next value and the rest — the body's `k`, kept, resumed when pulled (again and again: each pull
/// its own) — or done
pub enum Gen<W, T> {
    Done,
    Next(W, std::rc::Rc<dyn Fn() -> Eff<T, Gen<W, T>>>),
}

impl<W: Clone, T> Clone for Gen<W, T> {
    fn clone(&self) -> Self {
        match self {
            Gen::Done => Gen::Done,
            Gen::Next(w, rest) => Gen::Next(w.clone(), rest.clone()),
        }
    }
}

impl<W: Debug, T> Debug for Gen<W, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Gen::Done => f.write_str("Done"),
            Gen::Next(w, _) => write!(f, "Next({w:?}, _)"),
        }
    }
}

/// the body as a lazy generator: each value given back with the rest of the program, not resumed until pulled
pub fn generate<W: Clone + Debug + 'static, T: 'static, A: 'static>() -> Handler<Emit<W>, T, A, Gen<W, T>> {
    Handler::deep(|_| pure(Gen::Done), |op, k| match op {
        Emit::Value(w, r) => pure(Gen::Next(w, std::rc::Rc::new(move || r.resume(&k, ())))),
    })
}

/// every value emitted, in order, and the body's value
pub fn collect_emitted<W: Clone + Debug + 'static, T, A: 'static>() -> Handler<Emit<W>, T, A, (Vec<W>, A)> {
    Handler::stateful(Vec::new(), |s: &mut Vec<W>, op| match op {
        Emit::Value(w, r) => {
            s.push(w);
            r.give(())
        }
    }, |s, a| (s, a))
}

// SHIFTS: delimited control as an effect of the row — its answer `S` fixed, `T` the row outside its `reset`;
// `shift0`'s body outside the reset, `shift`'s under it (Danvy–Filinski's)

/// an erased rest of a shift: its value in, the program up to the reset out
type ShiftK<T, S> = std::rc::Rc<dyn Fn(crate::eff::Val) -> Eff<T, S>>;

type ShiftBody<T, S> = std::rc::Rc<dyn Fn(ShiftK<T, S>) -> Eff<T, S>>;
type ShiftResume<T, S> = std::rc::Rc<dyn Fn(&K<T, S>, crate::eff::Val) -> Eff<T, S>>;

pub struct Shifts<S, T> {
    body: ShiftBody<T, S>,
    resume: ShiftResume<T, S>,
}

impl<S, T> Clone for Shifts<S, T> {
    fn clone(&self) -> Self {
        Shifts { body: self.body.clone(), resume: self.resume.clone() }
    }
}

impl<S, T> Debug for Shifts<S, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Shift0(_)")
    }
}

impl<S: 'static, T: 'static> Effect for Shifts<S, T> {
    type Kind = Plain;
}

/// SHIFT0: the rest up to the nearest `reset` taken as `k`, the body run outside the reset — its answer the reset's
pub fn shift0<S: 'static, T: 'static, A: Clone + Debug + 'static, R: Member<Shifts<S, T>, Ix>, Ix>(
    f: impl Fn(std::rc::Rc<dyn Fn(A) -> Eff<T, S>>) -> Eff<T, S> + 'static,
) -> Eff<R, A> {
    let f = std::rc::Rc::new(f);
    perform(move |r: Reply<A>| Shifts {
        body: std::rc::Rc::new(move |k: ShiftK<T, S>| f(std::rc::Rc::new(move |a: A| k(crate::eff::Val::new(a))))),
        resume: std::rc::Rc::new(move |k: &K<T, S>, v: crate::eff::Val| r.resume(k, v.take::<A>())),
    })
}

/// SHIFT: as `shift0`, the body run under the reset — a shift in it captures up to the same reset
pub fn shift<S: 'static, T: 'static, A: Clone + Debug + 'static, R: Member<Shifts<S, T>, Ix>, Ix>(
    f: impl Fn(std::rc::Rc<dyn Fn(A) -> Eff<T, S>>) -> Eff<Cons<Shifts<S, T>, T>, S> + 'static,
) -> Eff<R, A> {
    shift0(move |k| f(k).handle(reset::<S, T>()))
}

/// THE RESET of `Shifts<S, T>`: a shift's body given the rest up to it
pub fn reset<S: 'static, T: 'static>() -> Handler<Shifts<S, T>, T, S, S> {
    Handler::deep(pure, |op: Shifts<S, T>, k| {
        let resume = op.resume.clone();
        (op.body)(std::rc::Rc::new(move |v| resume(&k, v)))
    })
}

// OVER THE WIRE: the standard effects' operations by name, their arguments a record

use crate::data::{Data, IsData};
use crate::wire::{answer_out, reply, AnswerOut, Remote};
use crate::Answer;

fn empty() -> Data {
    Data::Dict(vec![])
}

impl<V: Clone + Debug + IsData + 'static> Remote for Reader<V> {
    fn to_wire(&self) -> (String, Data) {
        ("ask".into(), empty())
    }
    fn answer(&self, d: &Data) -> Result<Answer, String> {
        match self {
            Reader::Ask(r) => Ok(r.give(V::from_data(d)?)),
        }
    }
    fn from_wire(name: &str, _: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        (name == "ask").then(|| Ok((Reader::Ask(reply()), answer_out::<V> as AnswerOut)))
    }
}

impl<S: Clone + Debug + IsData + 'static> Remote for State<S> {
    fn to_wire(&self) -> (String, Data) {
        match self {
            State::Get(_) => ("get".into(), empty()),
            State::Put(s, _) => ("put".into(), Data::Dict(vec![("s".into(), s.to_data())])),
        }
    }
    fn answer(&self, d: &Data) -> Result<Answer, String> {
        match self {
            State::Get(r) => Ok(r.give(S::from_data(d)?)),
            State::Put(_, r) => Ok(r.give(())),
        }
    }
    fn from_wire(name: &str, record: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        match name {
            "get" => Some(Ok((State::Get(reply()), answer_out::<S> as AnswerOut))),
            "put" => Some(record.field("s").and_then(S::from_data).map(|s| (State::Put(s, reply()), answer_out::<()> as AnswerOut))),
            _ => None,
        }
    }
}

impl<W: Clone + Debug + IsData + 'static> Remote for Writer<W> {
    fn to_wire(&self) -> (String, Data) {
        match self {
            Writer::Tell(w, _) => ("tell".into(), Data::Dict(vec![("w".into(), w.to_data())])),
        }
    }
    fn answer(&self, _: &Data) -> Result<Answer, String> {
        match self {
            Writer::Tell(_, r) => Ok(r.give(())),
        }
    }
    fn from_wire(name: &str, record: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        (name == "tell").then(|| record.field("w").and_then(W::from_data).map(|w| (Writer::Tell(w, reply()), answer_out::<()> as AnswerOut)))
    }
}

impl Remote for Choose {
    fn to_wire(&self) -> (String, Data) {
        match self {
            Choose::Among(n, _) => ("among".into(), Data::Dict(vec![("n".into(), n.to_data())])),
            Choose::Fail(_) => ("fail".into(), empty()),
        }
    }
    fn answer(&self, d: &Data) -> Result<Answer, String> {
        match self {
            Choose::Among(_, r) => Ok(r.give(usize::from_data(d)?)),
            Choose::Fail(_) => Err("okay: a failed path has no answer".into()),
        }
    }
    fn from_wire(name: &str, record: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        (name == "among").then(|| record.field("n").and_then(usize::from_data).map(|n| (Choose::Among(n, reply()), answer_out::<usize> as AnswerOut)))
    }
}

impl<E: Clone + Debug + IsData + 'static> Remote for Throws<E> {
    fn to_wire(&self) -> (String, Data) {
        match self {
            Throws::Raise(e, _) => ("raise".into(), Data::Dict(vec![("e".into(), e.to_data())])),
        }
    }
    fn answer(&self, _: &Data) -> Result<Answer, String> {
        Err("okay: a raise has no answer".into())
    }
    fn from_wire(_: &str, _: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        None
    }
}
