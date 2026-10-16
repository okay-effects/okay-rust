use std::cell::{Cell, RefCell};
use std::rc::Rc;

use okay_effects::{answering, handle, op, run, stream, Emit, Reply};

op!(pub struct Ask {} -> i64);

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn a_producer_pulled_as_an_iterator() {
    let squares: Vec<i64> = stream(|_, out| async move {
        for i in 1..=5 {
            out.emit(i * i).await;
        }
    })
    .collect();
    assert_eq!(squares, vec![1, 4, 9, 16, 25]);
}

#[test]
fn lazily_one_step_per_value() {
    let made = Rc::new(Cell::new(0));
    let m = made.clone();
    let mut s = stream(move |_, out| async move {
        for i in 0.. {
            m.set(i + 1);
            out.emit(i).await;
        }
    });
    assert_eq!(made.get(), 0);
    assert_eq!(s.next(), Some(0));
    assert_eq!(s.next(), Some(1));
    assert_eq!(made.get(), 2);
}

#[test]
fn breaking_out_drops_the_producer() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let mut seen = vec![];
    for x in stream(move |_, out| async move {
        let _guard = Cleanup(c);
        for i in 0.. {
            out.emit(i).await;
        }
    }) {
        if x == 3 {
            break;
        }
        seen.push(x);
    }
    assert_eq!(seen, vec![0, 1, 2]);
    assert!(cleaned.get(), "the producer's destructors ran");
}

#[test]
fn a_producer_with_handlers_of_its_own() {
    let got: Vec<i64> = stream(|r, out| async move {
        answering(&r, |_: Ask| 10, |ask| async move {
            for i in 0..3 {
                out.emit(ask.perform(Ask {}).await + i).await;
            }
        })
        .await
    })
    .collect();
    assert_eq!(got, vec![10, 11, 12]);
}

#[test]
fn consumed_inside_an_effectful_program() {
    // Emit handled like any operation: each value to an effectful consumer, which may stop the producer
    let log = Rc::new(RefCell::new(vec![]));
    let l = log.clone();
    let total = run(|r| async move {
        let r = &r;
        answering(r, |_: Ask| 100, |ask| async move {
            let sum = Rc::new(Cell::new(0));
            let s = sum.clone();
            handle(r, move |Emit(x): Emit<i64>| {
                let (ask, s) = (ask.clone(), s.clone());
                async move {
                    if x > 2 {
                        return Reply::Abort(s.get()); // enough: the producer is dropped
                    }
                    s.set(s.get() + x + ask.perform(Ask {}).await);
                    Reply::Resume(())
                }
            }, move |out| async move {
                for i in 0.. {
                    l.borrow_mut().push(i);
                    out.emit(i).await;
                }
                unreachable!()
            })
            .await
        })
        .await
    });
    assert_eq!(total, 303);
    assert_eq!(*log.borrow(), vec![0, 1, 2, 3]);
}

#[test]
fn a_producer_waiting_on_the_world() {
    // a future pending once, waking itself: the iterator parks until woken
    async fn yield_now() {
        let mut once = false;
        std::future::poll_fn(|cx| {
            if once {
                return std::task::Poll::Ready(());
            }
            once = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        })
        .await
    }
    let got: Vec<i64> = stream(|_, out| async move {
        for i in 0..3 {
            yield_now().await;
            out.emit(i).await;
        }
    })
    .collect();
    assert_eq!(got, vec![0, 1, 2]);
}
