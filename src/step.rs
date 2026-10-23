//! A PROGRAM STEPPED (okay-core's `Step`, okay-kernel's protocol): run until it asks its host — an operation of the
//! row outside it, no handler of the program's own for it — and given back there with the rest. The host answers it
//! (`resume`), fails it (`fail`), or drops it (`cancel`); the rest is data, so it may be resumed again — each
//! resumption its own, every level as it was when the operation was asked.

use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::eff::{short, Answer, Eff, Failure, Op};
use crate::machine::{self, Outcome, K0};
use crate::row::Effect;

/// WHERE A STEPPED PROGRAM STANDS
pub enum Step<R, A> {
    /// its value
    Done(A),
    /// an operation for the host, and the way on
    Yield(Yield<R, A>),
    /// a failure raised to the top: user code panicked, or the host failed an operation
    Failed(Failure),
    /// dropped by its host
    Cancelled,
}

impl<R, A: Debug + 'static> Debug for Step<R, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Done(a) => write!(f, "Done({a:?})"),
            Step::Yield(y) => write!(f, "Yield({} {})", y.effect(), y.describe()),
            Step::Failed(e) => write!(f, "Failed({:?})", e.message()),
            Step::Cancelled => f.write_str("Cancelled"),
        }
    }
}

/// AN OPERATION GIVEN BACK to the host, with the rest: answered by the operation's `Reply`, as many times as wanted
pub struct Yield<R, A> {
    op: Op,
    k: K0,
    /// cancelled under an uncancelable region: the cancel fires when the region is left
    cancelling: bool,
    _t: PhantomData<fn() -> (R, A)>,
}

impl<R, A> Clone for Yield<R, A> {
    fn clone(&self) -> Self {
        Yield { op: self.op.clone(), k: self.k.clone(), cancelling: self.cancelling, _t: PhantomData }
    }
}

impl<R, A: 'static> Yield<R, A> {
    /// the operation, if it is of the effect `E`: matched, its variant's `Reply` gives the answer
    pub fn op<E: Effect>(&self) -> Option<E> {
        (self.op.effect() == std::any::TypeId::of::<E>()).then(|| (self.op.info.copy)(&self.op.value).take::<E>())
    }

    /// its effect's name: `Reader<i64>`
    pub fn effect(&self) -> String {
        short((self.op.info.name)())
    }

    /// the operation as written: `Put(6, _)`
    pub fn describe(&self) -> String {
        (self.op.info.describe)(&self.op.value)
    }

    /// the rest resumed with the operation's value (`r.give(v)` of its reply): run to the next stop. The rest is
    /// kept: resumed again, each resumption its own
    pub fn resume(&self, a: Answer) -> Step<R, A> {
        let k = self.k.clone();
        if self.cancelling {
            k.pend_cancel();
        }
        guarded(move || machine::resume(k, a.0))
    }

    /// the operation answered with a failure, raised where it was performed — in the rest's levels, a recover there
    /// given it, each finalizer run on the way out
    pub fn fail(&self, f: Failure) -> Step<R, A> {
        let k = self.k.clone();
        guarded(move || machine::fail(k, f))
    }

    pub(crate) fn effect_id(&self) -> std::any::TypeId {
        self.op.effect()
    }

    pub(crate) fn erased_op(&self) -> &Op {
        &self.op
    }

    /// CANCELLED: the rest's finalizers run, no recover asked — `Cancelled`. Under an uncancelable region the same
    /// step is given back, to be resumed: the cancel fires when the region is left
    pub fn cancel(self) -> Step<R, A> {
        if self.k.masked() {
            return Step::Yield(Yield { cancelling: true, ..self });
        }
        let k = self.k;
        guarded(move || machine::cancel(k))
    }

}

fn outcome<R, A: 'static>(o: Outcome) -> Step<R, A> {
    match o {
        Outcome::Done(v) => Step::Done(v.take::<A>()),
        Outcome::Asked(op, k) => Step::Yield(Yield { op, k, cancelling: false, _t: PhantomData }),
        Outcome::Failed(f) => Step::Failed(f),
        Outcome::Cancelled => Step::Cancelled,
        Outcome::Guarded(_) => unreachable!("the guarded loop goes on itself"),
    }
}

/// user code's panics caught at the run: `Failed`, never through the host
fn guarded<R, A: 'static>(go: impl FnOnce() -> Outcome) -> Step<R, A> {
    match catch_unwind(AssertUnwindSafe(go)) {
        Ok(o) => outcome(o),
        Err(p) => Step::Failed(Failure(p)),
    }
}

impl<R, A: 'static> Eff<R, A> {
    /// the program STEPPED: run until its value, a failure, or its first operation of the row outside it
    pub fn step(self) -> Step<R, A> {
        let node = self.node;
        guarded(move || machine::start(node))
    }
}
