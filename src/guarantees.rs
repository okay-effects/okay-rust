//! WHAT THE TYPES REFUSE — okay-core's TestGuarantees, each refusal (`compile_fail`) beside a program like it that is
//! taken. The ones under a `$` wait for stage 2's control.
//!
//! **`run`: only with no effect left.**
//!
//! ```
//! use okay::*;
//! assert_eq!(pure::<Nil, i64>(1).run(), 1);
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
//! ask::<i64, row![Reader<i64>], _>().run();
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
//! type R = row![Reader<i64>, Writer<String>];
//! ask::<i64, R, _>().handle(reader(1)).run();
//! ```
//!
//! **An operation: only of an effect in its row.**
//!
//! ```
//! use okay::*; use okay::effects::*;
//! let _ = ask::<i64, row![Writer<String>, Reader<i64>], _>();
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
//! let _ = ask::<i64, row![Writer<String>], _>();
//! ```
//!
//! **A handler: only of an effect in its row.**
//!
//! ```
//! use okay::*; use okay::effects::*;
//! let _ = ask::<i64, row![Writer<String>, Reader<i64>], _>().handle(reader(1));
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
//! let _ = tell::<String, row![Writer<String>], _>(String::new()).handle(reader(1));
//! ```
//!
//! **A handler over the row outside it: only where its effects are there** — a reader telling each ask.
//!
//! ```
//! use okay::*; use okay::effects::*;
//! fn ask_logged<T: Member<Writer<String>, Ix> + 'static, Ix: 'static>(v: i64) -> Handler<Reader<i64>, T, i64, i64> {
//!     Handler::deep(pure, move |op, k| match op {
//!         Reader::Ask(r) => tell::<String, T, Ix>("asked".into()).flat_map(move |_| r.resume(&k, v)),
//!     })
//! }
//! type R = row![Reader<i64>, Writer<String>];
//! assert_eq!(ask::<i64, R, _>().handle(ask_logged(1)).handle(collect()).run(), (vec!["asked".to_string()], 1));
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
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
//! use okay::*; use okay::effects::*;
//! type T = row![Writer<String>];
//! let _: Handler<Reader<i64>, T, i64, i64> = Handler::deep(pure, |op, k| match op {
//!     Reader::Ask(r) => tell::<String, T, _>(String::new()).flat_map(move |_| r.resume(&k, 1)),
//! });
//! ```
//! ```compile_fail
//! use okay::*; use okay::effects::*;
//! type T = row![Writer<String>];
//! let _: Handler<Reader<i64>, T, i64, i64> = Handler::deep(pure, |op, k| match op {
//!     Reader::Ask(r) => ask::<i64, T, _>().flat_map(move |n| r.resume(&k, n)),
//! });
//! ```
//!
//! **A state whose type the steps move, in `okay::typed`: by ownership (tests/typestate.rs).** A query only after the
//! login; a session only closed; an old state not used again.
//!
//! ```
//! # mod db {
//! #     pub struct Closed; pub struct Open; pub struct Authed;
//! #     pub struct Sess<S>(S);
//! #     pub fn begin() -> Sess<Closed> { Sess(Closed) }
//! #     pub fn connect(_: Sess<Closed>) -> Sess<Open> { Sess(Open) }
//! #     pub fn login(_: Sess<Open>) -> Sess<Authed> { Sess(Authed) }
//! #     pub fn close(_: Sess<Authed>) -> Sess<Closed> { Sess(Closed) }
//! #     okay::needs! {
//! #         pub async fn query(cx: [okay::effects::Reader<i64>], s: Sess<Authed>) -> (i64, Sess<Authed>) {
//! #             (okay::typed::ask(cx).await, s)
//! #         }
//! #     }
//! #     /// a session owes its connection back closed
//! #     pub async fn session<A>(body: impl AsyncFnOnce(Sess<Closed>) -> (A, Sess<Closed>)) -> A {
//! #         body(begin()).await.0
//! #     }
//! # }
//! # use okay::typed::*;
//! # let cx = Cx(AskWith(8i64), End);
//! let n = run(db::session(async |s| {
//!     let s = db::login(db::connect(s));
//!     let (n, s) = db::query(&cx, s).await;
//!     (n, db::close(s))
//! }));
//! assert_eq!(n, Ok(8));
//! ```
//! ```compile_fail
//! # mod db {
//! #     pub struct Closed; pub struct Open; pub struct Authed;
//! #     pub struct Sess<S>(S);
//! #     pub fn begin() -> Sess<Closed> { Sess(Closed) }
//! #     pub fn connect(_: Sess<Closed>) -> Sess<Open> { Sess(Open) }
//! #     pub fn login(_: Sess<Open>) -> Sess<Authed> { Sess(Authed) }
//! #     pub fn close(_: Sess<Authed>) -> Sess<Closed> { Sess(Closed) }
//! #     okay::needs! {
//! #         pub async fn query(cx: [okay::effects::Reader<i64>], s: Sess<Authed>) -> (i64, Sess<Authed>) {
//! #             (okay::typed::ask(cx).await, s)
//! #         }
//! #     }
//! #     /// a session owes its connection back closed
//! #     pub async fn session<A>(body: impl AsyncFnOnce(Sess<Closed>) -> (A, Sess<Closed>)) -> A {
//! #         body(begin()).await.0
//! #     }
//! # }
//! # use okay::typed::*;
//! # let cx = Cx(AskWith(8i64), End);
//! // no login: the query asks Sess<Authed> of a Sess<Open>
//! let _ = run(db::session(async |s| {
//!     let s = db::connect(s);
//!     let (n, s) = db::query(&cx, s).await;
//!     (n, db::close(s))
//! }));
//! ```
//! ```compile_fail
//! # mod db {
//! #     pub struct Closed; pub struct Open; pub struct Authed;
//! #     pub struct Sess<S>(S);
//! #     pub fn begin() -> Sess<Closed> { Sess(Closed) }
//! #     pub fn connect(_: Sess<Closed>) -> Sess<Open> { Sess(Open) }
//! #     pub fn login(_: Sess<Open>) -> Sess<Authed> { Sess(Authed) }
//! #     pub fn close(_: Sess<Authed>) -> Sess<Closed> { Sess(Closed) }
//! #     okay::needs! {
//! #         pub async fn query(cx: [okay::effects::Reader<i64>], s: Sess<Authed>) -> (i64, Sess<Authed>) {
//! #             (okay::typed::ask(cx).await, s)
//! #         }
//! #     }
//! #     /// a session owes its connection back closed
//! #     pub async fn session<A>(body: impl AsyncFnOnce(Sess<Closed>) -> (A, Sess<Closed>)) -> A {
//! #         body(begin()).await.0
//! #     }
//! # }
//! # use okay::typed::*;
//! # let cx = Cx(AskWith(8i64), End);
//! // never closed: the session owes a Sess<Closed>
//! let _ = run(db::session(async |s| {
//!     let s = db::login(db::connect(s));
//!     db::query(&cx, s).await
//! }));
//! ```
//! ```compile_fail
//! # mod db {
//! #     pub struct Closed; pub struct Open; pub struct Authed;
//! #     pub struct Sess<S>(S);
//! #     pub fn begin() -> Sess<Closed> { Sess(Closed) }
//! #     pub fn connect(_: Sess<Closed>) -> Sess<Open> { Sess(Open) }
//! #     pub fn login(_: Sess<Open>) -> Sess<Authed> { Sess(Authed) }
//! #     pub fn close(_: Sess<Authed>) -> Sess<Closed> { Sess(Closed) }
//! #     okay::needs! {
//! #         pub async fn query(cx: [okay::effects::Reader<i64>], s: Sess<Authed>) -> (i64, Sess<Authed>) {
//! #             (okay::typed::ask(cx).await, s)
//! #         }
//! #     }
//! #     /// a session owes its connection back closed
//! #     pub async fn session<A>(body: impl AsyncFnOnce(Sess<Closed>) -> (A, Sess<Closed>)) -> A {
//! #         body(begin()).await.0
//! #     }
//! # }
//! # use okay::typed::*;
//! # let cx = Cx(AskWith(8i64), End);
//! // an old state used again: moved
//! let _ = run(db::session(async |s| {
//!     let open = db::connect(s);
//!     let a = db::login(open);
//!     let b = db::login(open);
//!     (0, db::close(a))
//! }));
//! ```
