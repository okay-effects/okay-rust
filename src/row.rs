//! ROWS OF EFFECTS: a list of types, `Cons<E, R>` ending in `Nil` — `row![Reader<i64>, State<i64>]`. An effect's
//! place in a row found by the compiler (`Member`'s index, `Here` or `There`), as okay-one's `In`; the row without it
//! (`Rest`) what a handler of it leaves, as okay-one's `Removed`.
//!
//! A SENSITIVE effect (`Kind = Sensitive`) is handled only under an audit: an interceptor or an observer turns it into
//! `Audited<E>` in the row outside it — no program there can perform it, and only `handle_audited` takes it off.

use std::fmt::Debug;
use std::marker::PhantomData;

/// the empty row: a program of no effect, run by `run`
pub struct Nil;

/// `E` first, then the row `R`
pub struct Cons<E, R>(PhantomData<fn() -> (E, R)>);

/// AN EFFECT of a row: the type of its operations, an enum — each operation a variant carrying its `Reply`. `Debug`,
/// so an observer can say what was asked; `Clone`, so it can pass it on; its `Kind` plain or sensitive
pub trait Effect: Clone + Debug + 'static {
    type Kind: Kind;
}

/// plain or sensitive
pub trait Kind: 'static {}
/// an effect handled by any handler
pub struct Plain;
/// an effect handled only under an audit: `handle_audited`, its row `Audited<E>`
pub struct Sensitive;
impl Kind for Plain {}
impl Kind for Sensitive {}

/// THE KIND OF A PLAIN EFFECT: handled by any handler, intercepted by any interceptor — a sensitive one's is not
#[diagnostic::on_unimplemented(
    message = "a sensitive effect is handled only under its audit",
    label = "this effect is sensitive",
    note = "audit it first (`audit` with an interceptor of it, or `observe`), then handle it with `handle_audited`"
)]
pub trait PlainKind {}
impl PlainKind for Plain {}

/// `E` audited: in the row outside an audit of it — performed by no one there, handled by `handle_audited`
pub struct Audited<E>(PhantomData<fn() -> E>);

/// the place of an effect: first
pub struct Here;
/// the place of an effect: after the first, at `I` in the rest
pub struct There<I>(PhantomData<fn() -> I>);

/// `E` IN THE ROW `Self`, at `Ix` — the index found by the compiler, never written; `Rest` the row without it
#[diagnostic::on_unimplemented(
    message = "the effect `{E}` is not in the row `{Self}`",
    label = "no `{E}` in this row",
    note = "a program performs only the effects of its row; add `{E}` to the row, or handle it before; a sensitive effect outside its audit is `Audited<{E}>`, performed by no one"
)]
pub trait Member<E, Ix> {
    type Rest;
}

impl<E, R> Member<E, Here> for Cons<E, R> {
    type Rest = R;
}

impl<E, F, R, Ix> Member<E, There<Ix>> for Cons<F, R>
where
    R: Member<E, Ix>,
{
    type Rest = Cons<F, R::Rest>;
}

/// `E` at `Ix` in `Self` MARKED audited: the row outside an audit of `E`
#[diagnostic::on_unimplemented(message = "the effect `{E}` is not in the row `{Self}`: nothing to audit")]
pub trait Mark<E, Ix> {
    type Out;
}

impl<E, R> Mark<E, Here> for Cons<E, R> {
    type Out = Cons<Audited<E>, R>;
}

impl<E, F, R, Ix> Mark<E, There<Ix>> for Cons<F, R>
where
    R: Mark<E, Ix>,
{
    type Out = Cons<F, R::Out>;
}

/// EVERY SENSITIVE EFFECT of `Self` marked audited, the plain ones as they are: the row outside an observer, which
/// sees them all
pub trait AuditAll {
    type Out;
}

impl AuditAll for Nil {
    type Out = Nil;
}

impl<E: Effect, R: AuditAll> AuditAll for Cons<E, R>
where
    E::Kind: MarkKind<E>,
{
    type Out = Cons<<E::Kind as MarkKind<E>>::Out, R::Out>;
}

impl<E, R: AuditAll> AuditAll for Cons<Audited<E>, R> {
    type Out = Cons<Audited<E>, R::Out>;
}

/// by the kind: a plain effect as it is, a sensitive one audited
pub trait MarkKind<E> {
    type Out;
}

impl<E> MarkKind<E> for Plain {
    type Out = E;
}

impl<E> MarkKind<E> for Sensitive {
    type Out = Audited<E>;
}

/// `row![A, B, C]`: `Cons<A, Cons<B, Cons<C, Nil>>>`
#[macro_export]
macro_rules! row {
    () => { $crate::Nil };
    ($e:ty $(, $rest:ty)* $(,)?) => { $crate::Cons<$e, $crate::row![$($rest),*]> };
}
