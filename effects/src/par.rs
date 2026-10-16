//! Parallel branches in one run, on one thread: `all`, `par`, `race`.
//!
//! The branches are polled round-robin with the run's waker, so their real
//! async waits (a timer, a socket) overlap. A branch performs through any
//! capability it can see — its own handlers', or a parent's, captured as any
//! value is (no lending: the borrow checker already keeps a capability inside
//! the scope of its handler). One mailbox serves them all: an operation answered
//! inside a branch is taken and answered within the branch's own poll; one that
//! goes past the branches stays in the mailbox across polls, and while it does
//! only the branch that performed it is polled — the parent's handlers answer
//! the branches' operations one at a time.

use std::future::{poll_fn, Future};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::Poll;

use crate::core::Run;

type Branch<'a> = Option<Pin<Box<dyn Future<Output = ()> + 'a>>>;

/// Poll `bs` to the end (all) or to the first done (`first`), whose index is
/// returned. A branch's panic drops the others, then goes on unwinding.
async fn poll_branches(r: &Run, mut bs: Vec<Branch<'_>>, first: bool) -> Option<usize> {
    let mut left = bs.len();
    let mut owner: Option<usize> = None; // the branch whose operation is out past us
    if left == 0 {
        return None;
    }
    poll_fn(|cx| {
        let n = bs.len();
        let locked = owner;
        for i in locked.into_iter().chain((0..n).filter(|&i| Some(i) != locked)) {
            let Some(b) = bs[i].as_mut() else { continue };
            match catch_unwind(AssertUnwindSafe(|| b.as_mut().poll(cx))) {
                Err(p) => {
                    bs.iter_mut().for_each(|b| *b = None); // the others dropped: their destructors run
                    resume_unwind(p)
                }
                Ok(Poll::Ready(())) => {
                    bs[i] = None;
                    left -= 1;
                    if first || left == 0 {
                        bs.iter_mut().for_each(|b| *b = None);
                        return Poll::Ready(Some(i));
                    }
                }
                Ok(Poll::Pending) => {}
            }
            if matches!(r.0.mailbox.borrow().as_ref(), Some(p) if p.asked()) {
                owner = Some(i); // outward, and this branch alone until it has its answer
                return Poll::Pending;
            }
            owner = None;
        }
        Poll::Pending
    })
    .await
}

/// Each branch writing its value into its slot.
fn slotted<'a, T, Fut: Future<Output = T> + 'a>(slots: &'a mut [Option<T>], futs: Vec<Fut>) -> Vec<Branch<'a>> {
    let box_ = |(slot, f): (&'a mut Option<T>, Fut)| -> Branch<'a> { Some(Box::pin(async move { *slot = Some(f.await) })) };
    slots.iter_mut().zip(futs).map(box_).collect()
}

/// Every branch run, their values in order. A branch that panics drops the
/// others and the panic goes on from here.
pub async fn all<T, Fut>(r: &Run, branches: impl IntoIterator<Item = Fut>) -> Vec<T>
where
    Fut: Future<Output = T>,
{
    let futs: Vec<Fut> = branches.into_iter().collect();
    let mut slots: Vec<Option<T>> = futs.iter().map(|_| None).collect();
    let bs = slotted(&mut slots, futs);
    poll_branches(r, bs, false).await;
    slots.into_iter().map(|s| s.expect("every branch done")).collect()
}

/// Two branches of different types, both run.
pub async fn par<A, B>(r: &Run, a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let (mut x, mut y) = (None, None);
    let bs: Vec<Branch<'_>> = vec![
        Some(Box::pin(async { x = Some(a.await) })),
        Some(Box::pin(async { y = Some(b.await) })),
    ];
    poll_branches(r, bs, false).await;
    (x.expect("done"), y.expect("done"))
}

/// The first branch's value; the others are dropped where they are, their
/// destructors run. Panics if there are no branches.
pub async fn race<T, Fut>(r: &Run, branches: impl IntoIterator<Item = Fut>) -> T
where
    Fut: Future<Output = T>,
{
    let futs: Vec<Fut> = branches.into_iter().collect();
    let mut slots: Vec<Option<T>> = futs.iter().map(|_| None).collect();
    let bs = slotted(&mut slots, futs);
    let won = poll_branches(r, bs, true).await.expect("okay: a race of no branches");
    slots.swap_remove(won).expect("the winner's value")
}
