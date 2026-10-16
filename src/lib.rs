//! # okay-rs
//!
//! okay-one in Rust: a program of effects is DATA (`Eff<R, A>`, a tree of nodes), its effects a ROW (`R`, a list of
//! types), its handlers VALUES of three forms not tied to it (answering in place, with a state, taking the rest), the
//! rest `k` data too — resumed as many times as a handler wants, each time as it was taken. A machine runs it.
//!
//! ```
//! use okay::*;
//! use okay::effects::*;
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
//! use okay::*;
//! use okay::effects::*;
//! let p: Eff<row![Reader<i64>], i64> = ask::<i64, _, _>();
//! p.run();
//! ```
//!
//! and performs only the effects of its row:
//!
//! ```compile_fail
//! use okay::*;
//! use okay::effects::*;
//! let p: Eff<row![Reader<i64>], i64> = get::<i64, _, _>();
//! ```

pub mod effects;
mod eff;
mod frame;
mod handler;
mod machine;
mod row;
mod val;

pub mod audit;
pub mod direct;
pub mod typed;

pub use eff::{defer, perform, pure, tail_rec, Loop, Answer, AnyOp, Bind, Eff, Never, Node, Pass, Program, Reply};
pub use handler::{Handler, Interceptor, Observer, K};
pub use row::{AuditAll, Audited, Cons, Effect, Here, Kind, Mark, MarkKind, Member, Nil, Plain, PlainKind, Sensitive, There};

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
/// `eff! { … }` is an `async` block (`okay::typed`): compiled for the handlers it runs under — an operation answered
/// in place a call, one dropping the rest the block dropped, one taking it once the block suspended.
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
    // COMPILED BY THE HANDLERS' TYPES: an async block
    (@async yield $e:expr) => { $e };
    (@async _ <- $e:expr ; $($rest:tt)+) => {{ ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async let $x:ident = $v:expr ; $($rest:tt)+) => {{ let $x = $v; $crate::eff!(@async $($rest)+) }};
    (@async $x:ident <- $e:expr ; $($rest:tt)+) => {{ let $x = ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async $e:expr ; $($rest:tt)+) => {{ ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async $e:expr) => { ($e).await };
    ($($t:tt)+) => { async move { $crate::eff!(@async $($t)+) } };
}

/// AN EFFECT DECLARED: its enum (`Clone`, `Debug`, each operation's `Reply` last), its kind, and each operation twice —
/// for a program as data (`Pay::transfer(…)`, an `Eff`) and for one compiled by its handlers' types (`transfer(cx, …)`,
/// `okay::typed`). An effect whose handlers may answer later — waiting on the world: `Step::Wait` — is declared so,
/// `enum Net: Plain, waits { … }`: its operations keep the future they wait on (`typed::Waiting`)
///
/// ```
/// use okay::*;
/// use okay::typed::{run, Cx, End, Handles, Step};
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
        $vis:vis enum $name:ident : $kind:ident, waits {
            $(
                $(#[$om:meta])*
                $op:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $crate::effect!(@decl [$(#[$m])*] $vis $name $kind [Waiting waiting] {
            $( [$(#[$om])*] $op ( $($arg : $ty),* ) -> $out = $f ; )*
        });
    };
    (
        $(#[$m:meta])*
        $vis:vis enum $name:ident : $kind:ident {
            $(
                $(#[$om:meta])*
                $op:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $crate::effect!(@decl [$(#[$m])*] $vis $name $kind [Perform perform] {
            $( [$(#[$om])*] $op ( $($arg : $ty),* ) -> $out = $f ; )*
        });
    };
    (
        @decl [$(#[$m:meta])*] $vis:vis $name:ident $kind:ident [$fut:ident $make:ident] {
            $(
                [$(#[$om:meta])*]
                $op:ident ( $($arg:ident : $ty:ty),* ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $(#[$m])*
        #[derive(Clone, Debug)]
        $vis enum $name {
            $( $(#[$om])* $op($($ty,)* $crate::Reply<$out>), )*
        }

        impl $crate::Effect for $name {
            type Kind = $crate::$kind;
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

        $(
            $(#[$om])*
            #[inline(always)]
            #[allow(clippy::too_many_arguments)]
            $vis fn $f<'c, OkayCx: $crate::typed::Has<$name, OkayIx>, OkayIx>(
                cx: &'c OkayCx $(, $arg: $ty)*
            ) -> $crate::typed::$fut<'c, OkayCx, $name, OkayIx, $out> {
                $crate::typed::$make(cx, move |r| $name::$op($($arg,)* r))
            }
        )*
    };
}

/// AN ASYNC FUNCTION OVER A CONTEXT, ITS EFFECTS LISTED: `cx: [Reader<i64>, State<i64>]` — the context's type and an
/// index for each effect made, the bounds `Has<E, Ix>` written (as `row![…]` for a program as data). Type parameters
/// of the function too, their bounds traits named by paths (`async fn f<V: Clone + std::fmt::Debug>(cx: [Reader<V>])
/// -> V`), each `'static` as an effect's values are
///
/// ```
/// use okay::effects::{Reader, State, Writer};
/// use okay::typed::*;
///
/// okay::needs! {
///     /// counts by what it asks
///     async fn counted(cx: [Reader<i64>, State<i64>, Writer<String>], n: i64) -> i64 {
///         okay::eff! {
///             d <- ask(cx);
///             _ <- modify(cx, |s| s + d * n);
///             s <- get(cx);
///             _ <- tell(cx, format!("s {s}"));
///             yield s
///         }.await
///     }
/// }
///
/// let cx = Cx(AskWith(2i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
/// assert_eq!(run(counted(&cx, 10)), Ok(21));
/// ```
#[macro_export]
macro_rules! needs {
    (
        $(#[$m:meta])*
        $vis:vis async fn $name:ident $(< $($g:ident $(: $($gb:ident)::+ $(+ $($gbs:ident)::+)*)?),* $(,)? >)?
            ($cx:ident : [$($e:ty),+ $(,)?] $(, $arg:ident : $t:ty)* $(,)?) $(-> $ret:ty)?
        $body:block
    ) => {
        $crate::needs!(@zip [$(#[$m])*] [$vis] [$name] [$($([$g: 'static $(+ $($gb)::+ $(+ $($gbs)::+)*)?])*)?] [$cx] [$($arg : $t),*] [$($ret)?] $body;
            []; [$($e),+]; [I0 I1 I2 I3 I4 I5 I6 I7 I8 I9 I10 I11 I12 I13 I14 I15]);
    };
    (@zip $m:tt $vis:tt $name:tt $g:tt $cx:tt $args:tt $ret:tt $body:block;
        [$($acc:tt)*]; [$e:ty $(, $rest:ty)*]; [$i:ident $($is:ident)*]) => {
        $crate::needs!(@zip $m $vis $name $g $cx $args $ret $body; [$($acc)* ($e, $i)]; [$($rest),*]; [$($is)*]);
    };
    (@zip [$(#[$m:meta])*] [$vis:vis] [$name:ident] [$([$g:ident : $($gb:tt)+])*] [$cx:ident] [$($arg:ident : $t:ty),*]
        [$($ret:ty)?] $body:block; [$(($e:ty, $i:ident))*]; []; [$($is:ident)*]) => {
        $(#[$m])*
        $vis async fn $name<$($g: $($gb)+,)* OkayCx, $($i),*>($cx: &OkayCx $(, $arg: $t)*) $(-> $ret)?
        where
            OkayCx: $($crate::typed::Has<$e, $i> +)*
        $body
    };
}
