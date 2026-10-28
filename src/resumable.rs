//! MULTI-SHOT CONTINUATIONS AS VALUES: a resumable program is a value of a type implementing `Rest`. Resumed with an
//! answer, it runs in place to its next STEP — its value (`Done`), or an operation (`Perform`), where it stopped. It is
//! `Clone`, so a handler resumes it as many times as it wants — a clone for each — each time from the same point, the
//! program's variables as they were. No replay: nothing runs twice but what each resumption runs.
//!
//! Two ways to write one, the same steps, the same handlers:
//!
//! - `#[okay_resumable::resumable]` (the `okay-resumable` crate, stable Rust): a function whose body stops at
//!   `ask!(Effect::Op(args))`, made into an explicit rest — an enum, a variant per stop holding the variables in
//!   scope there, `#[derive(Clone)]`;
//! - a coroutine (nightly Rust, feature `coroutines`: `coroutines`, `coroutine_trait`, `coroutine_clone`): its body
//!   stops at `ask!(…)`, which is `yield`; the rest the coroutine itself (`co`), cloned.
//!
//! A program not started is a rest too: its first resumption's answer is `unit()`, not looked at.
//!
//! An operation is an effect's variant carrying its `Reply` (`effect!`): `ask!(Choose::Among(3))` adds the reply,
//! and the answer comes back at the reply's type. A program's operations are one enum, a union of its effects
//! (`ops!`); a handler takes the operations of its effect from it (`OpOf`) and resumes the rest with `r.give(v)`.
//!
//! ```
//! use okay::resumable::*;
//! use okay::Reply;
//!
//! okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
//! okay::ops! { pub enum PickOps { Choose } }
//!
//! // a resumable program written by hand as the macro writes it: two choices, then a value
//! #[derive(Clone)]
//! enum Pick { Start, AfterA, AfterB { a: usize }, Ended }
//! impl Rest for Pick {
//!     type Op = PickOps;
//!     type Out = usize;
//!     fn resume(&mut self, v: okay::Answer) -> Step<PickOps, usize> {
//!         match std::mem::replace(self, Pick::Ended) {
//!             Pick::Start => {
//!                 *self = Pick::AfterA;
//!                 Step::Perform(PickOps::from(Choose::Among(3, Reply::new())))
//!             }
//!             Pick::AfterA => {
//!                 *self = Pick::AfterB { a: Reply::<usize>::new().take(v) };
//!                 Step::Perform(PickOps::from(Choose::Among(2, Reply::new())))
//!             }
//!             Pick::AfterB { a } => Step::Done(a * 10 + Reply::<usize>::new().take(v)),
//!             Pick::Ended => panic!("resumed after its end"),
//!         }
//!     }
//! }
//!
//! // every path: the rest resumed once for each alternative — a clone for each
//! fn all<K: Rest>(mut k: K, answer: okay::Answer, out: &mut Vec<K::Out>) where K::Op: OpOf<Choose> {
//!     match k.resume(answer) {
//!         Step::Done(a) => out.push(a),
//!         Step::Perform(op) => match op.op() {
//!             Ok(Choose::Among(n, r)) => for i in 0..n { all(k.clone(), r.give(i), out) },
//!             Err(_) => panic!("an operation this handler does not take"),
//!         },
//!     }
//! }
//! let mut out = vec![];
//! all(Pick::Start, unit(), &mut out);
//! assert_eq!(out, vec![0, 1, 10, 11, 20, 21]);
//! ```

use crate::reply::Answer;

/// A STEP of a resumable program: its value, or the operation it stopped at — the rest then resumed with its answer
pub enum Step<O, A> {
    Done(A),
    Perform(O),
}

/// THE REST of a resumable program: a value, resumed in place; cloned to resume it more than once
pub trait Rest: Clone {
    /// the program's operations: a union of its effects (`ops!`)
    type Op;
    /// the program's value
    type Out;
    /// the program from where it stopped, given the operation's answer (`unit()` to start it), to its next step
    fn resume(&mut self, answer: Answer) -> Step<Self::Op, Self::Out>;
}

/// AN OPERATION OF `E` in a union of effects: the operation, or the union given back (another effect's)
pub trait OpOf<E>: Sized {
    fn op(self) -> Result<E, Self>;
}

impl<E: crate::Effect> OpOf<E> for E {
    fn op(self) -> Result<E, Self> {
        Ok(self)
    }
}

/// the answer an operation with no value gets (`()`), and what a program starts with
pub fn unit() -> Answer {
    crate::Reply::<()>::new().give(())
}

/// A UNION OF EFFECTS: `ops! { pub enum ShopOps { Choose, Log } }` — a variant per effect, `From` each, `OpOf` each
#[macro_export]
macro_rules! ops {
    ($(#[$m:meta])* $vis:vis enum $name:ident { $($e:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug)]
        $vis enum $name { $($e($e)),+ }
        $(
            impl From<$e> for $name {
                fn from(o: $e) -> Self { $name::$e(o) }
            }
            impl $crate::resumable::OpOf<$e> for $name {
                #[allow(unreachable_patterns)]
                fn op(self) -> Result<$e, Self> {
                    match self { $name::$e(o) => Ok(o), other => Err(other) }
                }
            }
        )+
    };
}

/// AN OPERATION ASKED: `ask!(Choose::Among(3))` — the variant given its reply, the program stopped, the answer taken
/// back at the reply's type. In a `#[resumable]` fn the macro makes it a stop; in a coroutine it is `yield`
#[macro_export]
macro_rules! ask {
    ($($p:ident)::+ ( $($a:expr),* $(,)? )) => {{
        let okay_reply = $crate::Reply::new();
        let okay_answer = yield ::core::convert::From::from($($p)::+ ($($a,)* okay_reply));
        okay_reply.take(okay_answer)
    }};
}

/// A RESUMABLE PROGRAM AS A COROUTINE (nightly; the crate using it enables `coroutines`, `coroutine_clone`,
/// `stmt_expr_attributes`): `coroutine!(move -> usize { … ask!(…) … })` — its first resumption's argument dropped at
/// once, so that nothing but the program's own variables live across a stop (and it stays `Clone`)
#[macro_export]
macro_rules! coroutine {
    (move -> $ret:ty $body:block) => {
        #[coroutine]
        move |okay_first: $crate::Answer| -> $ret {
            ::core::mem::drop(okay_first);
            $body
        }
    };
    (move $body:block) => {
        #[coroutine]
        move |okay_first: $crate::Answer| {
            ::core::mem::drop(okay_first);
            $body
        }
    };
}

/// A COROUTINE AS A RESUMABLE PROGRAM (nightly, feature `coroutines`): `co(#[coroutine] move |_| { … ask!(…) … })` —
/// its rest the coroutine itself, cloned to resume again (`coroutine_clone`)
#[cfg(feature = "coroutines")]
mod coroutine {
    use super::*;
    use std::ops::{Coroutine, CoroutineState};
    use std::pin::Pin;

    /// a coroutine as a rest: resumed in place (`Unpin`: it holds no borrow of itself across a stop)
    #[derive(Clone)]
    pub struct Co<G>(pub G);

    impl<G> Rest for Co<G>
    where
        G: Coroutine<Answer> + Clone + Unpin,
    {
        type Op = G::Yield;
        type Out = G::Return;
        #[inline]
        fn resume(&mut self, answer: Answer) -> Step<G::Yield, G::Return> {
            match Pin::new(&mut self.0).resume(answer) {
                CoroutineState::Yielded(op) => Step::Perform(op),
                CoroutineState::Complete(a) => Step::Done(a),
            }
        }
    }

    /// a coroutine as a rest, not started: `co::<Ops, A, _>(…)` names its operations' union and its value
    pub fn co<O, A, G>(g: G) -> Co<G>
    where
        G: Coroutine<Answer, Yield = O, Return = A> + Clone + Unpin,
    {
        Co(g)
    }
}

#[cfg(feature = "coroutines")]
pub use coroutine::{co, Co};
