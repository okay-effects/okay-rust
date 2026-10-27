//! WHAT THE TYPES REFUSE in the one API (besides `okay::typed`'s own: a sensitive effect's bare handler, one under
//! something not its interceptor, a handler reached past its interceptor; a context outliving its handlers).
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
