//! A context built at run time (okay::dynamic): handlers nested as a configuration says, the same programs as under a
//! typed context; a sensitive effect a layer only under its interceptor; either, all, first over it
use std::cell::RefCell;

use okay::dynamic::{Dyn, With, all, either, first, root};
use okay::effects::{Reader, State, Writer};
use okay::typed::*;

okay::effect! { pub enum Pay: Sensitive { Transfer(to: String, n: i64) -> String = transfer; } }

struct Bank(RefCell<i64>);
impl Handles<Pay> for Bank {
    fn handle(&self, op: Pay) -> Step<'_> {
        match op {
            Pay::Transfer(to, n, r) => {
                *self.0.borrow_mut() -= n;
                Step::Now(r.give(format!("ok {to} {n}")))
            }
        }
    }
}

okay::needs! {
    async fn counted(cx: [Reader<i64>, State<i64>, Writer<String>], n: i64) -> i64 {
        let d = ask(cx).await;
        modify(cx, |s| s + d * n).await;
        let s = get(cx).await;
        tell(cx, format!("s {s}")).await;
        s
    }
}

#[test]
fn the_same_program_under_a_typed_and_a_dynamic_context() {
    let typed = Cx(AskWith(2i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
    assert_eq!(run(counted(&typed, 10)), Ok(21));
    // the handlers in an order read at run time
    let order = ["writer", "state", "reader"];
    let log = With::new(root(), Collect::<String>::new());
    let state = With::new(log.cx(), StateCell::new(1i64));
    let reader = With::new(state.cx(), AskWith(2i64));
    let cx = match order[2] {
        "reader" => reader.cx(),
        _ => unreachable!(),
    };
    assert_eq!(run(counted(&cx, 10)), Ok(21));
    assert_eq!(log.handler().take(), vec!["s 21".to_string()]);
}

/// handlers nested to a depth known only at run time: each level a reader adding one
fn nested<'a>(cx: Dyn<'a>, depth: i64, at: i64) -> std::pin::Pin<Box<dyn std::future::Future<Output = i64> + 'a>> {
    Box::pin(async move {
        if at == depth {
            return ask(&cx).await;
        }
        let outer: i64 = ask(&cx).await;
        let layer = With::new(cx, AskWith(outer + 1));
        nested(layer.cx(), depth, at + 1).await
    })
}

#[test]
fn handlers_nested_to_a_depth_known_at_run_time() {
    let base = With::new(root(), AskWith(0i64));
    for depth in [0, 1, 5, 50] {
        assert_eq!(run(nested(base.cx(), depth, 0)), Ok(depth));
    }
}

/// payments refused over a limit
struct Limit(i64);
impl<R> Interceptor<Pay, R, ()> for Limit {
    fn intercept<'a, H: Handles<Pay>>(&'a self, op: Pay, pass: Pass<'a, H>, _: &'a R) -> Step<'a> {
        match op {
            Pay::Transfer(to, n, r) if n > self.0 => Step::Now(r.give(format!("refused {to} {n}"))),
            Pay::Transfer(to, n, r) => {
                pass.perform(|r2| Pay::Transfer(to, n, r2)).map_or(Step::Later, |x| Step::Now(r.give(x)))
            }
        }
    }
}

#[test]
fn a_sensitive_effect_a_layer_under_its_interceptor() {
    let bank = With::audited(root(), Audited::new(Bank(RefCell::new(100)), Limit(50)));
    let cx = bank.cx();
    assert_eq!(run(transfer(&cx, "a".into(), 10)), Ok("ok a 10".to_string()));
    assert_eq!(run(transfer(&cx, "b".into(), 500)), Ok("refused b 500".to_string()));
    // rules set at run time, the same way
    let ps = Policies::new().with(|op, next| match op {
        Pay::Transfer(to, _, r) if to == "evil" => Step::Now(r.give("denied".to_string())),
        op => next(op),
    });
    let bank = With::audited(root(), Audited::new(Bank(RefCell::new(100)), ps));
    assert_eq!(run(transfer(&bank.cx(), "evil".into(), 1)), Ok("denied".to_string()));
}

#[test]
fn either_all_first_over_a_dynamic_context() {
    let log = With::new(root(), Collect::<String>::new());
    let got = run(either::<String, i64>(log.cx(), async |cx| {
        tell(&cx, "before".to_string()).await;
        raise::<String, i64, _, _>(&cx, "no".into()).await
    }));
    assert_eq!(got, Ok(Err("no".to_string())));
    let pairs = run(all(log.cx(), async |cx| {
        let a = flip(&cx).await;
        let b = among(&cx, &[1, 2]).await;
        (a, b)
    }));
    assert_eq!(pairs, Ok(vec![(true, 1), (true, 2), (false, 1), (false, 2)]));
    let odd = run(first(log.cx(), async |cx| {
        let n = among(&cx, &[2, 4, 5, 7]).await;
        if n % 2 == 0 { fail(&cx).await } else { n }
    }));
    assert_eq!(odd, Ok(Some(5)));
    assert_eq!(log.handler().take(), vec!["before".to_string()]);
}
