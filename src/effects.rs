//! THE STANDARD EFFECTS: their operations — a value asked (`Reader`), read and written (`State`), told (`Writer`), an
//! error raised (`Throws`). Their handlers and operations in `okay::typed`; over the wire each by name, its arguments
//! a record.

use std::fmt::Debug;

use crate::data::{Data, IsData};
use crate::kinds::{Effect, Plain};
use crate::reply::{Answer, Never, Reply};
use crate::wire::{answer_out, no_answer, reply, AnswerOut, Remote};

#[derive(Clone, Debug)]
pub enum Reader<V> {
    Ask(Reply<V>),
}
impl<V: Clone + Debug + 'static> Effect for Reader<V> {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
pub enum State<S> {
    Get(Reply<S>),
    Put(S, Reply<()>),
}
impl<S: Clone + Debug + 'static> Effect for State<S> {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
pub enum Writer<W> {
    Tell(W, Reply<()>),
}
impl<W: Clone + Debug + 'static> Effect for Writer<W> {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
pub enum Throws<E> {
    Raise(E, Reply<Never>),
}
impl<E: Clone + Debug + 'static> Effect for Throws<E> {
    type Kind = Plain;
}

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
    fn out(&self) -> AnswerOut {
        answer_out::<V>
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
    fn out(&self) -> AnswerOut {
        match self {
            State::Get(_) => answer_out::<S>,
            State::Put(..) => answer_out::<()>,
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
    fn out(&self) -> AnswerOut {
        answer_out::<()>
    }
    fn from_wire(name: &str, record: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        (name == "tell").then(|| record.field("w").and_then(W::from_data).map(|w| (Writer::Tell(w, reply()), answer_out::<()> as AnswerOut)))
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
    fn out(&self) -> AnswerOut {
        no_answer
    }
    fn from_wire(_: &str, _: &Data) -> Option<Result<(Self, AnswerOut), String>> {
        None
    }
}
