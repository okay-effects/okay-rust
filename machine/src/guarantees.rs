//! WHAT THE TYPES REFUSE — okay-core's TestGuarantees, each refusal (`compile_fail`) beside a program like it that is
//! taken. The ones under a `$` wait for stage 2's control.
//!
//! **`run`: only with no effect left.**
//!
//! ```
//! use okay_machine::*;
//! assert_eq!(pure::<Nil, i64>(1).run(), 1);
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! ask::<i64, row![Reader<i64>], _>().run();
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! type R = row![Reader<i64>, Writer<String>];
//! ask::<i64, R, _>().handle(reader(1)).run();
//! ```
//!
//! **An operation: only of an effect in its row.**
//!
//! ```
//! use okay_machine::*; use okay_machine::effects::*;
//! let _ = ask::<i64, row![Writer<String>, Reader<i64>], _>();
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! let _ = ask::<i64, row![Writer<String>], _>();
//! ```
//!
//! **A handler: only of an effect in its row.**
//!
//! ```
//! use okay_machine::*; use okay_machine::effects::*;
//! let _ = ask::<i64, row![Writer<String>, Reader<i64>], _>().handle(reader(1));
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! let _ = tell::<String, row![Writer<String>], _>(String::new()).handle(reader(1));
//! ```
//!
//! **A handler over the row outside it: only where its effects are there** — a reader telling each ask.
//!
//! ```
//! use okay_machine::*; use okay_machine::effects::*;
//! fn ask_logged<T: Member<Writer<String>, Ix> + 'static, Ix: 'static>(v: i64) -> Handler<Reader<i64>, T, i64, i64> {
//!     Handler::deep(pure, move |op, k| match op {
//!         Reader::Ask(r) => tell::<String, T, Ix>("asked".into()).flat_map(move |_| r.resume(&k, v)),
//!     })
//! }
//! type R = row![Reader<i64>, Writer<String>];
//! assert_eq!(ask::<i64, R, _>().handle(ask_logged(1)).handle(collect()).run(), (vec!["asked".to_string()], 1));
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! fn ask_logged<T: Member<Writer<String>, Ix> + 'static, Ix: 'static>(v: i64) -> Handler<Reader<i64>, T, i64, i64> {
//!     Handler::deep(pure, move |op, k| match op {
//!         Reader::Ask(r) => tell::<String, T, Ix>("asked".into()).flat_map(move |_| r.resume(&k, v)),
//!     })
//! }
//! // no writer outside the reader's handler
//! let _ = ask::<i64, row![Reader<i64>], _>().handle(ask_logged(1));
//! ```
//!
//! **A clause: a program over the row outside, not of its own effect.**
//!
//! ```
//! use okay_machine::*; use okay_machine::effects::*;
//! type T = row![Writer<String>];
//! let _: Handler<Reader<i64>, T, i64, i64> = Handler::deep(pure, |op, k| match op {
//!     Reader::Ask(r) => tell::<String, T, _>(String::new()).flat_map(move |_| r.resume(&k, 1)),
//! });
//! ```
//! ```compile_fail
//! use okay_machine::*; use okay_machine::effects::*;
//! type T = row![Writer<String>];
//! let _: Handler<Reader<i64>, T, i64, i64> = Handler::deep(pure, |op, k| match op {
//!     Reader::Ask(r) => ask::<i64, T, _>().flat_map(move |n| r.resume(&k, n)),
//! });
//! ```
