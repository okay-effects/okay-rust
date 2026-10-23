//! okay-effects' capabilities on the one API: a capability is a context — the one a program passes picks the
//! handler, so two instances of one effect coexist; it cannot escape its handler's scope (a borrow); an error drops
//! the body, its destructors run; a handler's panic is raised where the operation was performed
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay::effect;
use okay::typed::*;

effect! { enum Ask: Plain { Ask() -> i64 = ask_i; } }
effect! { enum AskStr: Plain { AskStr() -> String = ask_s; } }

struct Is<T>(T);
impl Handles<Ask> for Is<i64> {
    fn handle(&self, op: Ask) -> Step<'_> {
        match op {
            Ask::Ask(r) => Step::Now(r.give(self.0)),
        }
    }
}
impl Handles<AskStr> for Is<&'static str> {
    fn handle(&self, op: AskStr) -> Step<'_> {
        match op {
            AskStr::AskStr(r) => Step::Now(r.give(self.0.to_string())),
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
fn two_instances_of_one_effect() {
    // each its own context: the one passed answers
    let (outer, inner) = (Cx(Is(1i64), End), Cx(Is(2i64), End));
    let got = run(async { ask_i(&outer).await * 10 + ask_i(&inner).await });
    assert_eq!(got, Ok(12));
    // one over the other: the index says which — the nearest, `Here`; the outer, `There<Here>`
    let nested = Cx(Is(2i64), &outer);
    let got = run(async {
        ask_i::<_, okay::There<okay::Here>>(&nested).await * 10 + ask_i::<_, okay::Here>(&nested).await
    });
    assert_eq!(got, Ok(12));
}

#[test]
fn different_types() {
    let cx = Cx(Is(3i64), Cx(Is("ab"), End));
    let got = run(async { ask_s(&cx).await.repeat(ask_i(&cx).await as usize) });
    assert_eq!(got, Ok("ababab".to_string()));
}

#[test]
fn an_error_ends_the_body_and_its_destructors_run() {
    let cleaned = Rc::new(Cell::new(false));
    let c = cleaned.clone();
    let got = run(either(&Cx(Is(1i64), End), async |cx| {
        let _guard = Cleanup(c);
        if ask_i(cx).await == 1 {
            raise::<&str, (), _, _>(cx, "aborted").await;
        }
        "done"
    }));
    assert_eq!(got, Ok(Err("aborted")));
    assert!(cleaned.get());
}

struct Refuse;
impl Handles<Ask> for Refuse {
    fn handle(&self, _: Ask) -> Step<'_> {
        std::panic::resume_unwind(Box::new("refused".to_string()))
    }
}

#[test]
fn raised_at_the_operation() {
    let cx = Cx(Refuse, End);
    let p = catch_unwind(AssertUnwindSafe(|| run(ask_i(&cx))));
    assert_eq!(p.unwrap_err().downcast_ref::<String>().map(|s| s.as_str()), Some("refused"));
}

#[test]
fn a_million_operations() {
    let cx = Cx(Is(1i64), End);
    let got = run(async {
        let mut s = 0;
        for _ in 0..1_000_000 {
            s += ask_i(&cx).await;
        }
        s
    });
    assert_eq!(got, Ok(1_000_000));
}
