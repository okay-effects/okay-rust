//! THE AUDIT TRAIL as an effect: a record told — what an interceptor or an observer writes, handled outside them, so
//! the trail is a value a test can read and a handler can send anywhere.
//!
//! A SENSITIVE EFFECT is handled only under its audit. With a sensitive `Pay`:
//!
//! ```
//! use okay::*;
//! use okay::audit::*;
//! #[derive(Clone, Debug)]
//! enum Pay { Transfer(i64, Reply<String>) }
//! impl Effect for Pay { type Kind = Sensitive; }
//! fn bank<T, A: 'static>() -> Handler<Pay, T, A, A> {
//!     Handler::answering(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! }
//! fn passing<T: Member<Audited<Pay>, I> + Member<Audit, J> + 'static, I: 'static, J: 'static>()
//!     -> Interceptor<Pay, T, String, String> {
//!     Interceptor::new(pure, |op, k, pass| match op {
//!         Pay::Transfer(n, r) => record::<T, J>(format!("transfer {n}")).flat_map(move |_| {
//!             let k = k.clone();
//!             pass.perform::<T, I, String>(move |r2| Pay::Transfer(n, r2)).flat_map(move |s| r.resume(&k, s))
//!         }),
//!     })
//! }
//! let p: Eff<row![Pay, Audit], String> = perform(|r| Pay::Transfer(5, r));
//! assert_eq!(
//!     p.audit::<Pay, _, _>(passing()).handle_audited::<Pay, _, _>(bank()).handle(trail()).run(),
//!     (vec!["transfer 5".to_string()], "ok 5".to_string())
//! );
//! ```
//!
//! Not by a plain `handle`:
//!
//! ```compile_fail
//! # use okay::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, T, A, A> {
//! #     Handler::answering(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Eff<row![Pay], String> = perform(|r| Pay::Transfer(5, r));
//! p.handle(bank()).run();
//! ```
//!
//! nor by `handle_audited` with no audit before it:
//!
//! ```compile_fail
//! # use okay::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, T, A, A> {
//! #     Handler::answering(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Eff<row![Pay], String> = perform(|r| Pay::Transfer(5, r));
//! p.handle_audited::<Pay, _, _>(bank()).run();
//! ```
//!
//! nor intercepted as a plain one:
//!
//! ```compile_fail
//! # use okay::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, T, A, A> {
//! #     Handler::answering(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Eff<row![Pay], String> = perform(|r| Pay::Transfer(5, r));
//! let _ = p.intercept(bank());
//! ```
//!
//! and outside its audit no program performs it — the row there has `Audited<Pay>`, not `Pay`:
//!
//! ```compile_fail
//! # use okay::*;
//! # use okay::audit::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn passing<T: Member<Audited<Pay>, I> + 'static, I: 'static>() -> Interceptor<Pay, T, String, String> {
//! #     Interceptor::new(pure, |op, k, pass| match op {
//! #         Pay::Transfer(n, r) => pass.perform::<T, I, String>(move |r2| Pay::Transfer(n, r2)).flat_map(move |s| r.resume(&k, s)),
//! #     })
//! # }
//! let p: Eff<row![Pay], String> = perform(|r| Pay::Transfer(5, r));
//! let audited = p.audit::<Pay, _, _>(passing());
//! let _unaudited = audited.flat_map(|_| perform(|r| Pay::Transfer(1_000_000, r)));
//! ```

use crate::eff::{perform, Eff, Reply};
use crate::handler::Handler;
use crate::row::{Effect, Member, Plain};

#[derive(Clone, Debug)]
pub enum Audit {
    Record(String, Reply<()>),
}
impl Effect for Audit {
    type Kind = Plain;
}

/// a line of the trail
pub fn record<R: Member<Audit, Ix>, Ix>(line: String) -> Eff<R, ()> {
    perform(|r| Audit::Record(line, r))
}

/// the trail kept, in order, beside the value
pub fn trail<T, A: 'static>() -> Handler<Audit, T, A, (Vec<String>, A)> {
    Handler::stateful(Vec::new(), |s: &mut Vec<String>, op| match op {
        Audit::Record(line, r) => {
            s.push(line);
            r.give(())
        }
    }, |s, a| (s, a))
}
