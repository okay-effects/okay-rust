//! OVER A WIRE: a program's operations of the row outside it, and their answers, as plain values — okay's worker wire
//! (programs as data, `perform`/`continue`) with no transport here. An operation crosses as a name and a record (its
//! arguments, keys in lowerCamel: a Scala case class's shape), its answer a plain value. The rest is data, so the
//! other side may resume it many times — a Scala `Choice` handler takes every path of a Rust program.

use std::any::TypeId;
use std::collections::HashMap;
use std::rc::Rc;

use crate::data::{Data, IsData};
use crate::eff::{Answer, Failure};
use crate::row::Effect;
use crate::step::{Step, Yield};

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
pub fn reply<T>() -> crate::Reply<T> {
    crate::Reply::new()
}

type Encode = Rc<dyn Fn(&crate::eff::Op) -> (String, Data)>;
type Decode = Rc<dyn Fn(&crate::eff::Op, &Data) -> Result<Answer, String>>;

/// THE EFFECTS that may cross, by type: an operation of another is refused by name
#[derive(Clone, Default)]
pub struct Remotes {
    by: HashMap<TypeId, (Encode, Decode)>,
}

impl Remotes {
    pub fn new() -> Remotes {
        Remotes::default()
    }

    /// the operations of `E` cross
    pub fn with<E: Remote>(mut self) -> Remotes {
        let enc: Encode = Rc::new(|op| (op.info.copy)(&op.value).take::<E>().to_wire());
        let dec: Decode = Rc::new(|op, d| (op.info.copy)(&op.value).take::<E>().answer(d));
        self.by.insert(TypeId::of::<E>(), (enc, dec));
        self
    }
}

/// A PROGRAM ON THE WIRE: its value, an operation and the rest — a function of the answer, called as many times as the
/// other side resumes it — or a failure
pub enum Wire {
    Done(Data),
    Perform { name: String, args: Vec<Data>, k: Rc<dyn Fn(Result<Data, String>) -> Wire> },
    Failed(String),
}

/// a stepped program as a program on the wire: each operation of the row outside it encoded, each answer decoded
pub fn program<R: 'static, A: IsData + 'static>(s: Step<R, A>, remotes: &Remotes) -> Wire {
    match s {
        Step::Done(a) => Wire::Done(a.to_data()),
        Step::Failed(f) => Wire::Failed(f.message()),
        Step::Cancelled => Wire::Failed("cancelled".into()),
        Step::Yield(y) => {
            let Some((enc, dec)) = remotes.by.get(&y.effect_id()).cloned() else {
                return Wire::Failed(format!("okay: {} does not cross the wire (Remotes::with)", y.effect()));
            };
            let (name, record) = enc(y.erased_op());
            let args = match &record {
                Data::Dict(kv) if kv.is_empty() => vec![],
                r => vec![r.clone()],
            };
            let remotes = remotes.clone();
            let y: Rc<Yield<R, A>> = Rc::new(y);
            Wire::Perform {
                name,
                args,
                k: Rc::new(move |answer| {
                    let next = match answer.and_then(|d| dec(y.erased_op(), &d)) {
                        Ok(a) => y.resume(a),
                        Err(why) => y.fail(Failure::new(why)),
                    };
                    program(next, &remotes)
                }),
            }
        }
    }
}

/// THE OTHER WAY: a Rust handler of `E` answering operations that come over the wire — a name and a record in, a plain
/// value out; `None` when the name is none of `E`'s
pub fn serve<E: Remote>(handler: impl Fn(E) -> Answer) -> impl Fn(&str, &Data) -> Option<Result<Data, String>> {
    move |name, record| E::from_wire(name, record).map(|r| r.map(|(op, out)| out(handler(op))))
}
