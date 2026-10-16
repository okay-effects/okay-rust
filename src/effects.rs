//! THE STANDARD EFFECTS and their handlers: reader, state, writer, choice, throws — okay-one's std.

use crate::eff::{perform, pure, Eff, Never, Reply};
use crate::handler::{Handler, K};
use std::fmt::Debug;

use crate::row::{Effect, Member, Plain};

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
