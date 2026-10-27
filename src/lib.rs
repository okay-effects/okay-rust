//! # okay
//!
//! Algebraic effects in Rust, ONE API: a program is an `async` function over a context (`needs!`, `eff!`); what
//! answers its effects is the context it is given — handlers known at compile time (`okay::typed`: inlined), under
//! interceptors for control and audit (a sensitive effect only so), handed out to a host (`okay::host`: stepped,
//! middleware, okay's wire). The rest of a program is the suspended future: taken once. Its specification — okay-core's
//! kernel, a program as data, the rest taken many times — is the `okay-machine` crate.
//!
//! ```
//! use okay::effects::{Reader, State};
//! use okay::typed::*;
//!
//! okay::needs! {
//!     async fn twice(cx: [Reader<i64>, State<i64>]) -> i64 {
//!         let n = ask(cx).await;
//!         modify(cx, |s| s + n).await;
//!         get(cx).await * 2
//!     }
//! }
//! let cx = Cx(AskWith(5i64), Cx(StateCell::new(1i64), End));
//! assert_eq!(run(twice(&cx)), Ok(12));
//! ```

/// mimalloc as the global allocator (feature `mimalloc`)
#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod audit;
pub mod control;
pub mod data;
pub mod effects;
pub mod guarantees;
pub mod host;
pub mod services;
mod kinds;
mod reply;
pub mod typed;
#[doc(hidden)]
pub mod val;
pub mod wire;

pub use data::{Data, IsData};
pub use kinds::{Effect, Here, Kind, Plain, Sensitive, There};
#[doc(hidden)]
pub use reply::short;
pub use reply::{Answer, Loop, Never, Reply};

/// A RECORD: a struct whose fields are plain values, as data (`IsData`) — keys in lowerCamel, a Scala case class's
/// shape. `okay::data!(pub struct Receipt { pub paid: bool, pub detail: String });`
#[macro_export]
macro_rules! data {
    ($(#[$m:meta])* $vis:vis struct $name:ident { $($fvis:vis $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        $vis struct $name { $($fvis $field: $ty),* }
        impl $crate::IsData for $name {
            fn to_data(&self) -> $crate::Data {
                $crate::Data::Dict(vec![$((
                    $crate::data::camel(stringify!($field)),
                    $crate::IsData::to_data(&self.$field),
                )),*])
            }
            #[allow(unused_variables)]
            fn from_data(d: &$crate::Data) -> Result<Self, String> {
                Ok($name {
                    $($field: $crate::IsData::from_data(d.field(&$crate::data::camel(stringify!($field)))?)?),*
                })
            }
        }
    };
}

/// A FOR OVER PROGRAMS, one API: `x <- p;` binds `p`'s value, `_ <- p;` or `p;` drops it, `let x = v;` names a
/// value, the last line a program or `yield v`.
///
/// `eff! { … }` is an `async` block (`okay::typed`): compiled for the handlers it runs under — an operation answered
/// in place a call, one dropping the rest the block dropped, one taking it once the block suspended.
///
/// (A program as data, the machine's: `okay_machine::eff! { … }`.)
#[macro_export]
macro_rules! eff {
    // COMPILED BY THE HANDLERS' TYPES: an async block
    (@async yield $e:expr) => { $e };
    (@async _ <- $e:expr ; $($rest:tt)+) => {{ ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async let $x:ident = $v:expr ; $($rest:tt)+) => {{ let $x = $v; $crate::eff!(@async $($rest)+) }};
    (@async $x:ident <- $e:expr ; $($rest:tt)+) => {{ let $x = ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async $e:expr ; $($rest:tt)+) => {{ ($e).await; $crate::eff!(@async $($rest)+) }};
    (@async $e:expr) => { ($e).await };
    ($($t:tt)+) => { async move { $crate::eff!(@async $($t)+) } };
}

/// AN EFFECT DECLARED: its enum (`Clone`, `Debug`, each operation's `Reply` last), its kind, and each operation — a
/// function over a context (`transfer(cx, …)`, `okay::typed`); `wire`: its operations as data too (`Remote`). The
/// machine's `okay_machine::effect!` writes the same and its program-as-data forms (`Pay::transfer(…)`, an `Eff`). An effect whose handlers may answer later — waiting on the world: `Step::Wait` — is declared so,
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
        $vis:vis enum $name:ident : $kind:ident, waits, wire {
            $(
                $(#[$om:meta])*
                $op:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $crate::effect!(@decl [$(#[$m])*] $vis $name $kind [Waiting waiting] {
            $( [$(#[$om])*] $op ( $($arg : $ty),* ) -> $out = $f ; )*
        });
        $crate::effect!(@wire $name { $( $op ( $($arg : $ty),* ) -> $out = $f ; )* });
    };
    (
        $(#[$m:meta])*
        $vis:vis enum $name:ident : $kind:ident, wire {
            $(
                $(#[$om:meta])*
                $op:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $out:ty = $f:ident ;
            )*
        }
    ) => {
        $crate::effect!(@decl [$(#[$m])*] $vis $name $kind [Perform perform] {
            $( [$(#[$om])*] $op ( $($arg : $ty),* ) -> $out = $f ; )*
        });
        $crate::effect!(@wire $name { $( $op ( $($arg : $ty),* ) -> $out = $f ; )* });
    };
    // AN EFFECT THAT CROSSES THE WIRE: each operation by its function's name, its arguments a record
    (@wire $name:ident { $( $op:ident ( $($arg:ident : $ty:ty),* ) -> $out:ty = $f:ident ; )* }) => {
        impl $crate::wire::Remote for $name {
            #[allow(unused_variables)]
            fn to_wire(&self) -> (String, $crate::Data) {
                match self {
                    $( $name::$op($($arg,)* _) => (
                        stringify!($f).to_string(),
                        $crate::Data::Dict(vec![$(($crate::data::camel(stringify!($arg)), $crate::IsData::to_data($arg))),*]),
                    ), )*
                }
            }
            fn answer(&self, d: &$crate::Data) -> Result<$crate::Answer, String> {
                match self {
                    $( $name::$op(.., r) => Ok(r.give(<$out as $crate::IsData>::from_data(d)?)), )*
                }
            }
            fn out(&self) -> $crate::wire::AnswerOut {
                match self {
                    $( $name::$op(..) => $crate::wire::answer_out::<$out> as $crate::wire::AnswerOut, )*
                }
            }
            #[allow(unused_variables)]
            fn from_wire(name: &str, record: &$crate::Data) -> Option<Result<(Self, $crate::wire::AnswerOut), String>> {
                $(
                    if name == stringify!($f) {
                        #[allow(clippy::redundant_closure_call)]
                        let made = (|| -> Result<Self, String> {
                            Ok($name::$op($(<$ty as $crate::IsData>::from_data(record.field(&$crate::data::camel(stringify!($arg)))?)?,)* $crate::wire::reply()))
                        })();
                        return Some(made.map(|op| (op, $crate::wire::answer_out::<$out> as $crate::wire::AnswerOut)));
                    }
                )*
                None
            }
        }
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
