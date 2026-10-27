//! # okay-machine
//!
//! The SPECIFICATION of okay's one API (the `okay` crate): okay-core's kernel in Rust — a program of effects is DATA (`Eff<R, A>`, a tree of nodes), its effects a ROW (`R`, a list of
//! types), its handlers VALUES of three forms not tied to it (answering in place, with a state, taking the rest), the
//! rest `k` data too — resumed as many times as a handler wants, each time as it was taken. A machine runs it.
//!
//! ```
//! use okay_machine::*;
//! use okay_machine::effects::*;
//!
//! type R = row![Reader<i64>, State<i64>];
//! let p: Eff<R, i64> = eff! { data;
//!     n <- ask::<i64, _, _>();
//!     _ <- modify::<i64, _, _>(move |s| s + n);
//!     s <- get::<i64, _, _>();
//!     yield s * 2
//! };
//! assert_eq!(p.handle(reader(5)).handle(state(1)).run(), (6, 12));
//! ```
//!
//! A program runs only when every effect of its row is handled:
//!
//! ```compile_fail
//! use okay_machine::*;
//! use okay_machine::effects::*;
//! let p: Eff<row![Reader<i64>], i64> = ask::<i64, _, _>();
//! p.run();
//! ```
//!
//! and performs only the effects of its row:
//!
//! ```compile_fail
//! use okay_machine::*;
//! use okay_machine::effects::*;
//! let p: Eff<row![Reader<i64>], i64> = get::<i64, _, _>();
//! ```

pub mod audit;
pub mod cont;
pub mod effects;
pub mod guarantees;
mod eff;
mod frame;
mod handler;
mod machine;
#[cfg(test)]
mod reference;
mod row;
mod step;
pub mod wire;

#[doc(hidden)]
pub use okay as __okay;
pub use okay::{data, host, needs, typed};

pub use eff::{defer, perform, pure, tail_rec, Loop, Answer, AnyOp, Bind, Eff, Failure, Guard, Never, Node, Pass, PassingOut, Program, Reply, Resume};
pub use step::{Step, Yield};
pub use cont::{dollar, pure_c, Cont, Over, Passing, K1};
pub use okay::{Data, IsData};
pub use handler::{Handler, Interceptor, Observer, K};
pub use row::{AuditAll, Audited, Cons, Control, Delim, Effect, Nearest, Reset, Here, Kind, Mark, MarkKind, Member, Nil, Plain, PlainKind, Sensitive, There};

/// A TAIL CALL: `call!(f(a, b))` — `f(a, b)` made when the machine gets to it, not now; so functions calling each
/// other in tail position (`is_even` → `is_odd` → …) run in constant stack, however deep. The arguments are evaluated
/// when the call is made, and cloned: a rest resumed twice calls again
#[macro_export]
macro_rules! call {
    ($($f:ident)::+ ( $($arg:expr),* $(,)? )) => {
        $crate::defer(move || $($f)::+($(($arg).clone()),*))
    };
}

/// A FOR OVER PROGRAMS, one API: `x <- p;` binds `p`'s value, `_ <- p;` or `p;` drops it, `let x = v;` names a
/// value, the last line a program or `yield v`.
///
/// `eff! { … }` is the one API's (`okay_machine::eff!`, an `async` block).
///
/// `eff! { data; … }` is a program as data (`Eff`, the machine): for a rest taken many times (`choose`). Its rests
/// are closures called as many times as a handler resumes them: each takes copies (`clone`) of the names bound before
/// it — so a name bound is `Clone`.
#[macro_export]
macro_rules! eff {
    // A PROGRAM AS DATA, the machine's
    (data; $($t:tt)+) => { $crate::eff!(@data [] $($t)+) };
    (@data [$($b:ident)*] yield $e:expr) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        $crate::pure($e)
    }};
    (@data [$($b:ident)*] _ <- $e:expr ; $($rest:tt)+) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        ($e).flat_map(move |_| $crate::eff!(@data [$($b)*] $($rest)+))
    }};
    (@data [$($b:ident)*] let $x:ident = $v:expr ; $($rest:tt)+) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        let $x = $v;
        $crate::eff!(@data [$($b)* $x] $($rest)+)
    }};
    (@data [$($b:ident)*] $x:ident <- $e:expr ; $($rest:tt)+) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        ($e).flat_map(move |$x| $crate::eff!(@data [$($b)* $x] $($rest)+))
    }};
    (@data [$($b:ident)*] $e:expr ; $($rest:tt)+) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        ($e).flat_map(move |_| $crate::eff!(@data [$($b)*] $($rest)+))
    }};
    (@data [$($b:ident)*] $e:expr) => {{
        $( #[allow(unused_variables, clippy::redundant_clone)] let $b = $b.clone(); )*
        $e
    }};
    ($($t:tt)+) => { $crate::__okay::eff!($($t)+) };
}

/// AN EFFECT DECLARED: its enum (`Clone`, `Debug`, each operation's `Reply` last), its kind, and each operation twice —
/// for a program as data (`Pay::transfer(…)`, an `Eff`) and for one compiled by its handlers' types (`transfer(cx, …)`,
/// `okay_machine::typed`). An effect whose handlers may answer later — waiting on the world: `Step::Wait` — is declared so,
/// `enum Net: Plain, waits { … }`: its operations keep the future they wait on (`typed::Waiting`)
///
/// ```
/// use okay_machine::*;
/// use okay_machine::typed::{run, Cx, End, Handles, Step};
///
/// effect! {
///     /// a counter
///     pub enum Count: Plain {
///         /// one more: the count now
///         Add(n: i64) -> i64 = add;
///         /// the count
///         Now() -> i64 = now;
///     }
/// }
///
/// struct Counter(std::cell::Cell<i64>);
/// impl Handles<Count> for Counter {
///     fn handle(&self, op: Count) -> Step<'_> {
///         match op {
///             Count::Add(n, r) => { self.0.set(self.0.get() + n); Step::Now(r.give(self.0.get())) }
///             Count::Now(r) => Step::Now(r.give(self.0.get())),
///         }
///     }
/// }
///
/// let cx = Cx(Counter(std::cell::Cell::new(0)), End);
/// assert_eq!(run(eff! { add(&cx, 2); add(&cx, 3); now(&cx) }), Ok(5));
///
/// let p: Eff<row![Count], i64> = eff! { data; Count::add(2); Count::now() };
/// let counted = p.handle(Handler::stateful(0i64, |c: &mut i64, op| match op {
///     Count::Add(n, r) => { *c += n; r.give(*c) }
///     Count::Now(r) => r.give(*c),
/// }, |_, a| a));
/// assert_eq!(counted.run(), 2);
/// ```
#[macro_export]
macro_rules! effect {
    (
        $(#[$m:meta])*
        $vis:vis enum $name:ident : $kind:ident $(, $flag:ident)* {
            $(
                $(#[$om:meta])*
                $op:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $crate::__okay::effect! {
            $(#[$m])*
            $vis enum $name : $kind $(, $flag)* {
                $( $(#[$om])* $op ( $($arg : $ty),* ) -> $out = $f ; )*
            }
        }
        impl $name {
            $(
                $(#[$om])*
                #[allow(clippy::too_many_arguments)]
                $vis fn $f<OkayRow: $crate::Member<$name, OkayIx>, OkayIx>($($arg: $ty),*) -> $crate::Eff<OkayRow, $out> {
                    $crate::perform(move |r| $name::$op($($arg,)* r))
                }
            )*
        }
    };
}

