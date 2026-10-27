//! OVER A WIRE: a program's operations of the row outside it, and their answers, as plain values — okay's worker wire
//! (programs as data, `perform`/`continue`) with no transport here. An operation crosses as a name and a record (its
//! arguments, keys in lowerCamel: a Scala case class's shape), its answer a plain value. The rest is data, so the
//! other side may resume it many times — a Scala `Choice` handler takes every path of a Rust program.

use std::any::TypeId;
use std::collections::HashMap;
use std::rc::Rc;

use crate::data::{Data, IsData};
use crate::eff::{Answer, Failure};
use crate::step::{Step, Yield};

pub use okay::wire::{answer_out, no_answer, reply, serve, AnswerOut, Remote};

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
