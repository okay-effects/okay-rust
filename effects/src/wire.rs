//! Steps over a wire: each side sees only operations and answers.
//!
//! An operation crosses as its name and one record of its fields (keys in
//! lowerCamel, as a Scala case class's): `Charge { user, cents }` is
//! `charge {user, cents}`. An answer comes back as a plain value, decoded into
//! what the operation answers. No transport here: a caller is whatever sends a
//! request and waits — okay's Rust worker's `okay_call`.

use crate::data::{Data, IsData, Operation};
use crate::middleware::Answerer;

/// Every operation answered over the wire through `call(name, args)`.
/// Middleware goes around it as around any Answerer.
pub fn remote<'a>(mut call: impl FnMut(&str, Vec<Data>) -> Result<Data, String> + 'a) -> Answerer<'a> {
    Box::new(move |a| {
        let args = match a.record() {
            Data::Dict(kv) if kv.is_empty() => vec![],
            r => vec![r.clone()],
        };
        call(a.name(), args)
    })
}

/// The other way: a Rust answer to `O`, callable from the wire by name with the
/// operation's record as its argument, its answer a plain value.
pub fn serve<O: Operation>(answer: impl Fn(O) -> O::Answer) -> (String, impl Fn(Vec<Data>) -> Result<Data, String>) {
    (O::name(), move |args: Vec<Data>| {
        let rec = args.into_iter().next().unwrap_or(Data::Dict(vec![]));
        Ok(answer(O::from_data(&rec)?).to_data())
    })
}
