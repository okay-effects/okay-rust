//! THE AUDIT TRAIL as an effect: a record told — what an interceptor or a watch writes, handled outside them, so the
//! trail is a value a test can read and a handler can send anywhere (`typed::record`, `typed::Trail`).

use crate::kinds::{Effect, Plain};
use crate::reply::Reply;

#[derive(Clone, Debug)]
pub enum Audit {
    Record(String, Reply<()>),
}
impl Effect for Audit {
    type Kind = Plain;
}
