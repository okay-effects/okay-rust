//! THE AUDIT TRAIL as an effect: a record told — what an interceptor or an observer writes, handled outside them, so
//! the trail is a value a test can read and a handler can send anywhere.
//!
//! A SENSITIVE EFFECT is handled only under its audit. With a sensitive `Pay`:
//!
//! ```
//! use okay_machine::*;
//! use okay_machine::audit::*;
//! #[derive(Clone, Debug)]
//! enum Pay { Transfer(i64, Reply<String>) }
//! impl Effect for Pay { type Kind = Sensitive; }
//! fn bank<T, A: 'static>() -> Handler<Pay, A, A, T> {
//!     Handler::in_place(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! }
//! fn passing<T: Member<Audited<Pay>, I> + Member<Audit, J> + 'static, I: 'static, J: 'static>()
//!     -> Interceptor<Pay, String, String, T> {
//!     Interceptor::new(pure, |op, k, pass| match op {
//!         Pay::Transfer(n, r) => record::<T, J>(format!("transfer {n}")).flat_map(move |_| {
//!             let k = k.clone();
//!             pass.perform::<T, I, String>(move |r2| Pay::Transfer(n, r2)).flat_map(move |s| r.resume(&k, s))
//!         }),
//!     })
//! }
//! let p: Prog<String, row![Pay, Audit]> = perform(|r| Pay::Transfer(5, r));
//! assert_eq!(
//!     p.audit::<Pay, _, _>(passing()).handle_audited::<Pay, _, _>(bank()).handle(trail()).run(),
//!     (vec!["transfer 5".to_string()], "ok 5".to_string())
//! );
//! ```
//!
//! Not by a plain `handle`:
//!
//! ```compile_fail
//! # use okay_machine::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, A, A, T> {
//! #     Handler::in_place(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Prog<String, row![Pay]> = perform(|r| Pay::Transfer(5, r));
//! p.handle(bank()).run();
//! ```
//!
//! nor by `handle_audited` with no audit before it:
//!
//! ```compile_fail
//! # use okay_machine::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, A, A, T> {
//! #     Handler::in_place(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Prog<String, row![Pay]> = perform(|r| Pay::Transfer(5, r));
//! p.handle_audited::<Pay, _, _>(bank()).run();
//! ```
//!
//! nor intercepted as a plain one:
//!
//! ```compile_fail
//! # use okay_machine::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn bank<T, A: 'static>() -> Handler<Pay, A, A, T> {
//! #     Handler::in_place(|op| match op { Pay::Transfer(n, r) => r.give(format!("ok {n}")) }, |a| a)
//! # }
//! let p: Prog<String, row![Pay]> = perform(|r| Pay::Transfer(5, r));
//! let _ = p.intercept(bank());
//! ```
//!
//! and outside its audit no program performs it — the row there has `Audited<Pay>`, not `Pay`:
//!
//! ```compile_fail
//! # use okay_machine::*;
//! # use okay_machine::audit::*;
//! # #[derive(Clone, Debug)] enum Pay { Transfer(i64, Reply<String>) }
//! # impl Effect for Pay { type Kind = Sensitive; }
//! # fn passing<T: Member<Audited<Pay>, I> + 'static, I: 'static>() -> Interceptor<Pay, String, String, T> {
//! #     Interceptor::new(pure, |op, k, pass| match op {
//! #         Pay::Transfer(n, r) => pass.perform::<T, I, String>(move |r2| Pay::Transfer(n, r2)).flat_map(move |s| r.resume(&k, s)),
//! #     })
//! # }
//! let p: Prog<String, row![Pay]> = perform(|r| Pay::Transfer(5, r));
//! let audited = p.audit::<Pay, _, _>(passing());
//! let _unaudited = audited.flat_map(|_| perform(|r| Pay::Transfer(1_000_000, r)));
//! ```

use crate::eff::{perform, Prog};
use crate::handler::Handler;
use crate::row::Member;

pub use okay::audit::Audit;

/// a line of the trail
pub fn record<R: Member<Audit, Ix>, Ix>(line: String) -> Prog<(), R> {
    perform(|r| Audit::Record(line, r))
}

/// the trail kept, in order, beside the value
pub fn trail<T, A: 'static>() -> Handler<Audit, A, (Vec<String>, A), T> {
    Handler::stateful(Vec::new(), |s: &mut Vec<String>, op| match op {
        Audit::Record(line, r) => {
            s.push(line);
            r.give(())
        }
    }, |s, a| (s, a))
}
