//! WHAT AN EFFECT IS: the type of its operations, an enum — each operation a variant carrying its `Reply`; its kind
//! plain or sensitive. And the place of an effect in a list of handlers (a context) or of effects (a row), found by
//! the compiler: `Here`, `There<…>`, never written.

use std::fmt::Debug;
use std::marker::PhantomData;

/// AN EFFECT: the type of its operations, an enum — each operation a variant carrying its `Reply`. `Debug`, so an
/// observer can say what was asked; `Clone`, so it can pass it on; its `Kind` plain or sensitive
pub trait Effect: Clone + Debug + 'static {
    type Kind: Kind;
}

/// plain or sensitive
pub trait Kind: 'static {}

/// an effect handled by any handler
pub struct Plain;

/// an effect handled only under its audit: an interceptor of it
pub struct Sensitive;

impl Kind for Plain {}
impl Kind for Sensitive {}

/// the place of an effect: first
pub struct Here;

/// the place of an effect: after the first, at `I` in the rest
pub struct There<I>(PhantomData<fn() -> I>);
