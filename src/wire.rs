//! OVER A WIRE: an effect's operations and their answers as plain values — okay's worker wire, no transport here. An
//! operation crosses as a name and a record (its arguments, keys in lowerCamel: a Scala case class's shape), its
//! answer a plain value: a program handed out to a host (`okay::host::remote`), a Rust handler answering okay
//! (`serve`).

use crate::data::{Data, IsData};
use crate::kinds::Effect;
use crate::reply::Answer;

/// AN EFFECT THAT CROSSES: each operation as a name and a record, and its answer made from a plain value by the
/// operation's own reply. `effect! { pub enum E: Plain, wire { … } }` writes it
pub trait Remote: Effect {
    /// the operation's name and its record
    fn to_wire(&self) -> (String, Data);
    /// the operation answered with `d`, decoded into what it answers
    fn answer(&self, d: &Data) -> Result<Answer, String>;
    /// how an answer to this operation goes out as a plain value
    fn out(&self) -> AnswerOut;
    /// an operation of this effect from the wire, if `name` is one of its; and its answer as a plain value
    fn from_wire(name: &str, record: &Data) -> Option<Result<(Self, AnswerOut), String>>;
}

/// how an answer given by a Rust handler goes back as a plain value
pub type AnswerOut = fn(Answer) -> Data;

#[doc(hidden)]
pub fn answer_out<T: IsData + 'static>(a: Answer) -> Data {
    a.0.take::<T>().to_data()
}

#[doc(hidden)]
pub fn reply<T>() -> crate::reply::Reply<T> {
    crate::reply::Reply::new()
}

/// the answer of an operation that has none (a raise, a failed path): never made
#[doc(hidden)]
pub fn no_answer(_: Answer) -> Data {
    unreachable!("okay: an operation with no answer answered")
}

/// THE OTHER WAY: a Rust handler of `E` answering operations that come over the wire — a name and a record in, a plain
/// value out; `None` when the name is none of `E`'s
pub fn serve<E: Remote>(handler: impl Fn(E) -> Answer) -> impl Fn(&str, &Data) -> Option<Result<Data, String>> {
    move |name, record| E::from_wire(name, record).map(|r| r.map(|(op, out)| out(handler(op))))
}
