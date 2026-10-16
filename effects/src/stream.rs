//! Streams: a producer emitting values through a capability, pulled as an `Iterator`.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Waker};

use crate::core::{Cap, Dead, Op, Run, Unpark};

/// One value out of a producer; answered `()` when the consumer takes the next.
pub struct Emit<T>(pub T);

impl<T: 'static> Op for Emit<T> {
    type Answer = ();
}

impl<T: 'static> Cap<Emit<T>> {
    /// `out.emit(x).await`: `x` to the consumer; resumes when the next value is wanted.
    pub async fn emit(&self, x: T) {
        self.perform(Emit(x)).await
    }
}

/// The values a producer emits, one per `next()`; dropping it drops the producer
/// where it is (its destructors run).
pub struct Stream<'a, T> {
    run: Run,
    id: u64,
    fut: Option<Pin<Box<dyn Future<Output = ()> + 'a>>>,
    waker: Waker,
    _dead: Dead,
    _t: PhantomData<fn() -> T>,
}

/// `producer` run lazily, a step per value: it gets its run (for handlers of its
/// own) and the capability to emit through. An operation it performs that none of
/// its handlers owns is a panic, as in `run`; a real async wait parks the thread.
pub fn stream<'a, T: 'static, F, Fut>(producer: F) -> Stream<'a, T>
where
    F: FnOnce(Run, Cap<Emit<T>>) -> Fut,
    Fut: Future<Output = ()> + 'a,
{
    let run = Run::new();
    let cap: Cap<Emit<T>> = Cap::new(&run);
    let (id, dead) = (cap.id, Dead(cap.live.clone()));
    let fut: Pin<Box<dyn Future<Output = ()> + 'a>> = Box::pin(producer(run.clone(), cap));
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    Stream { run, id, fut: Some(fut), waker, _dead: dead, _t: PhantomData }
}

impl<T: 'static> Iterator for Stream<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let fut = self.fut.as_mut()?;
        let mut cx = Context::from_waker(&self.waker);
        loop {
            if fut.as_mut().poll(&mut cx).is_ready() {
                self.fut = None;
                return None;
            }
            let mut mb = self.run.0.mailbox.borrow_mut();
            match mb.as_mut() {
                Some(p) if p.asked() && p.cap == self.id => {
                    let Emit(x) = *p.op.take().expect("asked").downcast::<Emit<T>>().unwrap_or_else(|_| unreachable!("a Cap<Emit<T>> carries an Emit<T>"));
                    p.answer = Some(Box::new(())); // taken when the producer is polled again
                    return Some(x);
                }
                Some(p) if p.asked() => panic!("okay: an operation (capability #{}) no live handler owns", p.cap),
                _ => {
                    drop(mb);
                    std::thread::park();
                }
            }
        }
    }
}
