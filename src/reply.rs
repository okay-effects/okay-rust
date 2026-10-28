//! THE REPLY to an operation and the answer it makes; a value that never comes; a step of a loop.

use std::fmt::{self, Debug};
use std::marker::PhantomData;

use crate::val::Val;

/// THE REPLY to an operation: the only way to its value — `give` it (a handler answering in place), or `resume` the
/// rest with it (a handler taking the rest). Made by `perform` alone, one for each operation
pub struct Reply<T>(PhantomData<fn(T) -> T>);

impl<T> Clone for Reply<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Reply<T> {}

impl<T> Debug for Reply<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("_")
    }
}

/// the value given to an operation, erased: made by a `Reply`
pub struct Answer(#[doc(hidden)] pub Val);

impl<T> Reply<T> {
    /// a reply, made by a performer: the one API's operations, the machine's `perform`
    #[doc(hidden)]
    pub fn new() -> Self {
        Reply(PhantomData)
    }
}

impl<T: 'static> Reply<T> {
    /// the operation's value `t`
    pub fn give(self, t: T) -> Answer {
        Answer(Val::new(t))
    }
    /// the answer given to this operation, at its type — where a resumable program takes it back
    pub fn take(self, a: Answer) -> T {
        a.0.take::<T>()
    }
}

/// no value: an operation that never comes back (`fail`, `raise`)
#[derive(Clone, Debug)]
pub enum Never {}

/// A STEP OF A LOOP: on, with the next state, or done, with the value
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loop<S, A> {
    Next(S),
    Done(A),
}

/// a type's name, its paths dropped: `okay::effects::Reader<i64>` to `Reader<i64>`
#[doc(hidden)]
pub fn short(name: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
        } else if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            out.push_str(&word);
            word.clear();
            out.push(c);
        }
    }
    out.push_str(&word);
    out
}
