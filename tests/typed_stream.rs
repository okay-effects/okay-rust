//! A body emitting values, pulled as an `Iterator` (okay::typed `stream`): okay-effects' streams on the one API
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use okay::effect;
use okay::typed::*;

effect! { enum Ask: Plain { Ask() -> i64 = ask_i; } }

struct Ten;
impl Handles<Ask> for Ten {
    fn handle(&self, op: Ask) -> Step<'_> {
        match op {
            Ask::Ask(r) => Step::Now(r.give(10)),
        }
    }
}

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn a_body_pulled_as_an_iterator() {
    let squares: Vec<i64> = stream(&End, async |cx| {
        for i in 1..=5i64 {
            emit(&cx, i * i).await;
        }
    })
    .collect();
    assert_eq!(squares, vec![1, 4, 9, 16, 25]);
}

#[test]
fn lazily_one_step_per_value() {
    let made = Rc::new(Cell::new(0));
    let m = made.clone();
    let mut s = stream(&End, async move |cx| {
        for i in 0.. {
            m.set(i + 1);
            emit(&cx, i).await;
        }
    });
    assert_eq!(made.get(), 0);
    assert_eq!(s.next(), Some(0));
    assert_eq!(s.next(), Some(1));
    assert_eq!(made.get(), 2);
}

#[test]
fn breaking_out_drops_the_body() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let mut seen = vec![];
    for x in stream(&End, async move |cx| {
        let _guard = Cleanup(c);
        for i in 0.. {
            emit(&cx, i).await;
        }
    }) {
        if x == 3 {
            break;
        }
        seen.push(x);
    }
    assert_eq!(seen, vec![0, 1, 2]);
    assert!(cleaned.get(), "the body's destructors ran");
}

#[test]
fn a_body_with_the_handlers_of_its_context() {
    let cx = Cx(Ten, End);
    let got: Vec<i64> = stream(&cx, async |cx| {
        for i in 0..3 {
            let n = ask_i(&cx).await;
            emit(&cx, n + i).await;
        }
    })
    .collect();
    assert_eq!(got, vec![10, 11, 12]);
}

#[test]
fn consumed_inside_an_effectful_program() {
    // inside a program, the values pushed instead: `generate`, the body suspended at each, an error stopping it
    let log = RefCell::new(vec![]);
    let cx = Cx(Ten, End);
    let got = run(either(&cx, async |cx| {
        let mut sum = 0;
        generate(
            cx,
            |x: i64| sum += x,
            async |g| {
                for i in 0.. {
                    log.borrow_mut().push(i);
                    if i == 3 {
                        raise::<String, (), _, _>(g, "enough".into()).await;
                    }
                    emit(g, i).await;
                }
            },
        )
        .await;
        sum + ask_i(cx).await
    }));
    assert_eq!(got, Ok(Err("enough".to_string())));
    assert_eq!(*log.borrow(), vec![0, 1, 2, 3]);
}

#[test]
fn a_body_waiting_on_the_world() {
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
    let got: Vec<i64> = stream(&End, async |cx| {
        for i in 0..3 {
            yield_now().await;
            emit(&cx, i).await;
        }
    })
    .collect();
    assert_eq!(got, vec![0, 1, 2]);
}
