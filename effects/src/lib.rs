//! okay-effects: algebraic effects for Rust, typed, on `async`/`.await` — the
//! design of okay (Scala 3), okay-go and okay-python, in Rust.
//!
//! A handler makes a capability and hands it to its body; the body performs
//! operations through it: `let ok = inventory.reserve(item).await` — an
//! `async fn` is a stackless coroutine, so performing suspends the body, the
//! handler that owns the capability answers, the body resumes. A handler
//! answers only ITS capability's operations — by identity, not by type — so
//! two instances of one effect coexist; an operation of another passes
//! outward. A capability used after its handler returned is refused by name.
//! Aborting or cancelling drops the body: its destructors run.
//!
//! No dependencies; stable Rust.
//!
//! ```
//! use okay_effects::{answering, op, run};
//!
//! op!(pub struct Ask {} -> i64);
//!
//! // two instances of one effect, told apart by capability
//! let got = run(|r| async move {
//!     let r = &r;
//!     answering(r, |_: Ask| 1, |outer| answering(r, |_: Ask| 2, move |inner| async move {
//!         outer.perform(Ask {}).await * 10 + inner.perform(Ask {}).await
//!     }))
//!     .await
//! });
//! assert_eq!(got, 12);
//! ```
//!
//! A program handed out and stepped by a host (the kernel's protocol):
//!
//! ```
//! use okay_effects::{op, start, Step};
//!
//! op!(pub struct Ask {} -> i64);
//!
//! let mut run = start(|h| async move {
//!     let ask = h.outside::<Ask>();
//!     ask.perform(Ask {}).await + ask.perform(Ask {}).await
//! });
//! let mut s = run.step();
//! while let Step::Yielded(asked) = s {
//!     assert_eq!(asked.name(), "ask");
//!     s = run.resume(21i64);
//! }
//! assert!(matches!(s, Step::Done(42)));
//! ```

mod core;
mod data;
mod kernel;
mod middleware;
mod par;
mod paths;
mod stream;
pub mod wire;

pub use crate::core::{answering, handle, run, Cap, Op, Reply, Run};
pub use crate::data::{camel, snake, Data, IsData, Operation};
pub use crate::kernel::{start, Asked, Computation, Host, Step};
pub use crate::middleware::{drive, only, record, replay, with, Answerer, Entry, Middleware, Routes};
pub use crate::par::{all, par, race};
pub use crate::paths::every_path;
pub use crate::stream::{stream, Emit, Stream};
