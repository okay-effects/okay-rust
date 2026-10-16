//! LEVEL 1 — EFFECTS PASSED DOWN, DIRECT STYLE (evidence passing, as Koka and Effekt do): for the effects whose
//! handler answers and goes on — a reader, a state, a writer, a log, an audit; most of a program's. An effect is a
//! trait, its handler a value implementing it, a program a generic function taking the effects it uses as parameters:
//! its signature is its row. An operation is a call of the handler's method — no node, no search, no `k`; monomorphized,
//! inlined. An interceptor is a wrapper of a handler. A sensitive effect's trait asks [`AuditedOnly`], which only
//! okay's wrappers — an audit, an observer — implement: a handler not under one cannot be given to a program.
//!
//! The effects that take the rest — a choice resumed twice, a generator, async — stay level 2, the machine (`Eff`).
//!
//! ```
//! use okay::direct::*;
//!
//! fn counting(st: &mut impl State<i64>, log: &mut impl Writer<String>, n: i64) -> i64 {
//!     for _ in 0..n {
//!         let x = st.get();
//!         st.put(x + 1);
//!     }
//!     log.tell(format!("counted {n}"));
//!     st.get()
//! }
//!
//! let (mut st, mut log) = (Cell(0), Log::new());
//! assert_eq!(counting(&mut st, &mut log, 1000), 1000);
//! assert_eq!(log.0, vec!["counted 1000".to_string()]);
//! ```
//!
//! A sensitive effect is given to a program only under its audit:
//!
//! ```compile_fail
//! use okay::direct::*;
//! pub trait Pay: AuditedOnly { fn transfer(&mut self, amount: i64) -> String; }
//! pub trait Bank { fn send(&mut self, amount: i64) -> String; }
//! struct MyBank;
//! impl Bank for MyBank { fn send(&mut self, amount: i64) -> String { format!("ok {amount}") } }
//! impl Pay for MyBank { fn transfer(&mut self, amount: i64) -> String { self.send(amount) } }   // AuditedOnly: sealed
//! ```

use std::cell::RefCell;
use std::fmt::Debug;

// THE STANDARD EFFECTS, as traits

/// a value asked
pub trait Reader<V> {
    fn ask(&mut self) -> V;
}

/// a value read and written
pub trait State<S> {
    fn get(&mut self) -> S;
    fn put(&mut self, s: S);
    fn modify(&mut self, f: impl FnOnce(S) -> S)
    where
        Self: Sized,
    {
        let s = self.get();
        self.put(f(s));
    }
}

/// values told
pub trait Writer<W> {
    fn tell(&mut self, w: W);
}

// THEIR HANDLERS, values

/// each ask answered the same value
#[derive(Clone, Debug)]
pub struct Const<V>(pub V);
impl<V: Clone> Reader<V> for Const<V> {
    #[inline]
    fn ask(&mut self) -> V {
        self.0.clone()
    }
}

/// a state in place
#[derive(Clone, Debug)]
pub struct Cell<S>(pub S);
impl<S: Clone> State<S> for Cell<S> {
    #[inline]
    fn get(&mut self) -> S {
        self.0.clone()
    }
    #[inline]
    fn put(&mut self, s: S) {
        self.0 = s;
    }
}

/// the told, in order
#[derive(Clone, Debug, Default)]
pub struct Log<W>(pub Vec<W>);
impl<W> Log<W> {
    pub fn new() -> Self {
        Log(Vec::new())
    }
}
impl<W> Writer<W> for Log<W> {
    #[inline]
    fn tell(&mut self, w: W) {
        self.0.push(w);
    }
}

// INTERCEPTING A PLAIN EFFECT: a wrapper, its clause given the operation and the handler further out

/// a writer intercepted: `f` given each value told and the handler outside
pub struct InterceptWriter<'h, H, F>(pub &'h mut H, pub F);
impl<W, H: Writer<W>, F: FnMut(W, &mut H)> Writer<W> for InterceptWriter<'_, H, F> {
    #[inline]
    fn tell(&mut self, w: W) {
        (self.1)(w, self.0)
    }
}

// SENSITIVE EFFECTS: only under an audit

mod sealed {
    pub trait Sealed {}
}

/// WHAT A SENSITIVE EFFECT'S TRAIT ASKS (`pub trait Pay: AuditedOnly`): implemented by okay's wrappers alone —
/// [`Audited`], [`Observed`] — so no handler of it reaches a program but through one
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not under an audit: a sensitive effect reaches a program only through one",
    label = "not an audit",
    note = "wrap the handler: `audit(&mut handler, interceptor)` or `observe(&mut handler, &observer)`"
)]
pub trait AuditedOnly: sealed::Sealed {}

/// A HANDLER UNDER ITS AUDIT: the handler `H`, the interceptor `I` (its policy: told, passed on changed, refused) — the
/// effect's author implements the effect's trait for it, calling `I` around `H`
pub struct Audited<'h, H, I> {
    pub handler: &'h mut H,
    pub interceptor: I,
}
impl<H, I> sealed::Sealed for Audited<'_, H, I> {}
impl<H, I> AuditedOnly for Audited<'_, H, I> {}

/// `h` under the interceptor `i`
pub fn audit<H, I>(handler: &mut H, interceptor: I) -> Audited<'_, H, I> {
    Audited { handler, interceptor }
}

// OBSERVING: every operation of a handler told, with its answer

/// what an observer is told of each operation: its effect, the operation, its answer
pub trait Observer {
    fn seen(&mut self, effect: &'static str, op: &dyn Debug, answer: &dyn Debug);
}

/// a handler observed: each operation passed on, then told with its answer. Sensitive effects too — an observer is
/// an audit of every effect it wraps. One observer for many handlers: shared in a `RefCell`
pub struct Observed<'h, 'o, H, O> {
    pub handler: &'h mut H,
    pub observer: &'o RefCell<O>,
}

impl<H, O: Observer> Observed<'_, '_, H, O> {
    /// an operation told: its effect, itself, its answer
    pub fn seen(&self, effect: &'static str, op: &dyn Debug, answer: &dyn Debug) {
        self.observer.borrow_mut().seen(effect, op, answer)
    }
}
impl<H, O> sealed::Sealed for Observed<'_, '_, H, O> {}
impl<H, O> AuditedOnly for Observed<'_, '_, H, O> {}

/// `h` observed by `o`
pub fn observe<'h, 'o, H, O: Observer>(handler: &'h mut H, observer: &'o RefCell<O>) -> Observed<'h, 'o, H, O> {
    Observed { handler, observer }
}

impl<V: Debug, H: Reader<V>, O: Observer> Reader<V> for Observed<'_, '_, H, O> {
    fn ask(&mut self) -> V {
        let v = self.handler.ask();
        self.seen("Reader", &"Ask", &v);
        v
    }
}
impl<S: Debug + Clone, H: State<S>, O: Observer> State<S> for Observed<'_, '_, H, O> {
    fn get(&mut self) -> S {
        let s = self.handler.get();
        self.seen("State", &"Get", &s);
        s
    }
    fn put(&mut self, s: S) {
        let shown = format!("Put({s:?})");
        self.handler.put(s);
        self.seen("State", &format_args!("{shown}"), &());
    }
}
impl<W: Debug, H: Writer<W>, O: Observer> Writer<W> for Observed<'_, '_, H, O> {
    fn tell(&mut self, w: W) {
        let shown = format!("Tell({w:?})");
        self.handler.tell(w);
        self.seen("Writer", &format_args!("{shown}"), &());
    }
}

/// an observer keeping the trail as lines: `Effect Op`, then `  → answer`
#[derive(Clone, Debug, Default)]
pub struct Trail(pub Vec<String>);
impl Observer for Trail {
    fn seen(&mut self, effect: &'static str, op: &dyn Debug, answer: &dyn Debug) {
        self.0.push(format!("{effect} {op:?}"));
        self.0.push(format!("  → {answer:?}"));
    }
}
