use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay_effects::{answering, handle, op, run, Cap, Op, Reply};

struct Ask;
impl Op for Ask {
    type Answer = i64;
}
struct AskStr;
impl Op for AskStr {
    type Answer = String;
}
op!(pub struct Tick {} -> ());

fn message(p: Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

#[test]
fn two_instances_of_one_effect() {
    let got = run(|r| async move {
        let r = &r;
        answering(r, |_: Ask| 1, |outer| answering(r, |_: Ask| 2, move |inner| async move {
            outer.perform(Ask).await * 10 + inner.perform(Ask).await
        }))
        .await
    });
    assert_eq!(got, 12);
}

#[test]
fn different_types() {
    let got = run(|r| async move {
        let r = &r;
        answering(r, |_: Ask| 3, |n| answering(r, |_: AskStr| "ab".to_string(), move |s| async move {
            s.perform(AskStr).await.repeat(n.perform(Ask).await as usize)
        }))
        .await
    });
    assert_eq!(got, "ababab");
}

#[test]
fn an_escaped_capability_is_refused() {
    let kept: Rc<Cell<Option<Cap<Ask>>>> = Rc::default();
    let k = kept.clone();
    run(|r| async move { answering(&r, |_: Ask| 1, |a| async move { k.set(Some(a)); 0 }).await });
    let cap = kept.take().unwrap();
    let p = catch_unwind(AssertUnwindSafe(|| run(|_| async move { cap.perform(Ask).await })));
    assert!(message(p.unwrap_err()).contains("escaped its scope"));
}

struct Cleanup(Rc<Cell<bool>>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn abort_ends_the_body_and_its_destructors_run() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let got = run(|r| async move {
        handle(&r, |_: Ask| std::future::ready(Reply::Abort("aborted")), move |a| async move {
            let _guard = Cleanup(c);
            a.perform(Ask).await;
            "done"
        })
        .await
    });
    assert_eq!(got, "aborted");
    assert!(cleaned.get());
}

#[test]
fn raise_at_the_operation() {
    let p = catch_unwind(AssertUnwindSafe(|| {
        run(|r| async move {
            handle(&r, |_: Ask| std::future::ready(Reply::<i64, i64>::Raise(Box::new("refused".to_string()))), |a| async move {
                a.perform(Ask).await
            })
            .await
        })
    }));
    assert_eq!(message(p.unwrap_err()), "refused");
}

#[test]
fn a_handler_answering_through_an_outer_capability() {
    // translating: Ask answered by performing Tick outside, counted there
    let ticks = Rc::new(Cell::new(0));
    let t = ticks.clone();
    let got = run(|r| async move {
        let r = &r;
        answering(r, move |_: Tick| t.set(t.get() + 1), |tick| handle(r, move |_: Ask| {
            let tick = tick.clone();
            async move { tick.perform(Tick {}).await; Reply::Resume(7) }
        }, |a| async move { a.perform(Ask).await + a.perform(Ask).await }))
        .await
    });
    assert_eq!((got, ticks.get()), (14, 2));
}

#[test]
fn a_million_operations() {
    let got = run(|r| async move {
        answering(&r, |_: Ask| 1, |a| async move {
            let mut s = 0;
            for _ in 0..1_000_000 {
                s += a.perform(Ask).await;
            }
            s
        })
        .await
    });
    assert_eq!(got, 1_000_000);
}
