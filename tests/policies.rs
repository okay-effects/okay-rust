//! Interceptors set at run time (okay::typed `Policies`, watches in a `Vec`): rules read from a configuration, changed
//! while the program's context lives, a sensitive effect's audit among them — the type still says "audited"
use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

use okay::host::{Step as HostStep, start};
use okay::typed::*;

okay::effect! {
    pub enum Pay: Sensitive, wire {
        Transfer(to: String, amount: i64) -> String = transfer;
    }
}

struct Bank(RefCell<i64>);
impl Handles<Pay> for Bank {
    fn handle(&self, op: Pay) -> Step<'_> {
        match op {
            Pay::Transfer(to, n, r) => {
                *self.0.borrow_mut() -= n;
                Step::Now(r.give(format!("ok:{to}:{n}")))
            }
        }
    }
}

type Trail = Rc<RefCell<Vec<String>>>;

/// rules from a configuration, one a line: `limit N`, `normalize`, `deny ACCOUNT`, `trail`
fn rules(config: &str, trail: &Trail) -> Policies<Pay> {
    let ps = Policies::new();
    for line in config.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["limit", n] => {
                let limit: i64 = n.parse().unwrap();
                ps.add(move |op, next| match op {
                    Pay::Transfer(to, amount, r) if amount > limit => {
                        Step::Now(r.give(format!("refused: {amount} to {to} over {limit}")))
                    }
                    op => next(op),
                });
            }
            ["normalize"] => {
                ps.add(|op, next| match op {
                    Pay::Transfer(to, amount, r) => next(Pay::Transfer(to.trim().to_uppercase(), amount, r)),
                });
            }
            ["deny", who] => {
                let who = who.to_string();
                ps.add(move |op, next| match op {
                    Pay::Transfer(to, _, r) if to == who => Step::Now(r.give(format!("refused: {to} denied"))),
                    op => next(op),
                });
            }
            ["trail"] => {
                let t = trail.clone();
                ps.add(move |op, next| {
                    let seen = format!("{op:?}");
                    let step = next(op);
                    t.borrow_mut().push(seen);
                    step
                });
            }
            _ => panic!("no rule {line}"),
        }
    }
    ps
}

okay::needs! {
    async fn payments(cx: [Pay]) -> Vec<String> {
        vec![
            transfer(cx, " acct-1 ".into(), 100).await,
            transfer(cx, "evil".into(), 10).await,
            transfer(cx, "acct-2".into(), 5000).await,
        ]
    }
}

#[test]
fn rules_read_from_a_configuration() {
    let trail: Trail = Rc::default();
    let config = "trail\nlimit 1000\nnormalize\ndeny EVIL";
    let cx = Cx(Audited::new(Bank(RefCell::new(1000)), rules(config, &trail)), End);
    assert_eq!(
        run(payments(&cx)),
        Ok(vec![
            "ok:ACCT-1:100".to_string(),
            "refused: EVIL denied".to_string(),
            "refused: 5000 to acct-2 over 1000".to_string()
        ])
    );
    // the trail outermost: every operation as the program asked it
    assert_eq!(trail.borrow().len(), 3);
    assert!(trail.borrow()[0].contains("\" acct-1 \""), "{:?}", trail.borrow());
    let (bank, _) = cx.0.into_parts();
    assert_eq!(bank.0.into_inner(), 900);
}

#[test]
fn rules_changed_while_the_context_lives() {
    let cx = Cx(Audited::new(Bank(RefCell::new(0)), Policies::<Pay>::new()), End);
    // none: every operation passed on
    assert_eq!(run(transfer(&cx, "a".into(), 5000)), Ok("ok:a:5000".to_string()));
    // a limit set now, for the next programs
    cx.0.interceptor().add(|op, next| match op {
        Pay::Transfer(to, n, r) if n > 100 => Step::Now(r.give(format!("refused {to}"))),
        op => next(op),
    });
    assert_eq!(run(transfer(&cx, "a".into(), 5000)), Ok("refused a".to_string()));
    assert_eq!(run(transfer(&cx, "a".into(), 50)), Ok("ok:a:50".to_string()));
    cx.0.interceptor().clear();
    assert_eq!(run(transfer(&cx, "a".into(), 5000)), Ok("ok:a:5000".to_string()));
}

#[test]
fn a_host_behind_the_rules_sees_only_what_they_pass() {
    let mut c = start(|host| async move {
        let ps = Policies::new().with(|op, next| match op {
            Pay::Transfer(to, n, r) if n > 1000 => Step::Now(r.give(format!("refused {to}"))),
            op => next(op),
        });
        let cx = Cx(Audited::new(host.outside::<Pay>(), ps), End);
        (transfer(&cx, "big".into(), 5000).await, transfer(&cx, "small".into(), 5).await)
    });
    let mut asked = vec![];
    let mut s = c.step();
    while let HostStep::Yielded(a) = s {
        asked.push(a.name().to_string() + " " + &format!("{:?}", a.record()));
        s = c.resume("host ok".to_string());
    }
    assert!(matches!(s, HostStep::Done((ref a, ref b)) if a == "refused big" && b == "host ok"), "{s:?}");
    assert_eq!(asked.len(), 1, "{asked:?}");
}

/// a watch refusing an effect by its name
struct Forbid(&'static str);
impl Watch for Forbid {
    fn before(&self, effect: &str, _: &dyn Debug) -> Result<(), String> {
        if effect == self.0 { Err(format!("{effect} forbidden")) } else { Ok(()) }
    }
}
/// a watch telling each operation and its answer
struct Tell(Trail);
impl Watch for Tell {
    fn after(&self, effect: &str, op: &dyn Debug, answer: &dyn Debug) {
        self.0.borrow_mut().push(format!("{effect} {op:?} -> {answer:?}"));
    }
}

#[test]
fn watches_set_at_run_time() {
    use okay::effects::{Reader, Writer};
    okay::needs! {
        async fn both_effects(cx: [Reader<i64>, Writer<String>]) -> i64 {
            let n = ask(cx).await;
            tell(cx, format!("n {n}")).await;
            n
        }
    }
    let cx = Cx(AskWith(7i64), Cx(Collect::<String>::new(), End));
    let told: Trail = Rc::default();
    let mut watches: Vec<Box<dyn Watch>> = vec![Box::new(Tell(told.clone()))];
    assert_eq!(run(observe(&cx, &watches, async |cx| both_effects(cx).await)), Ok(Ok(7)));
    assert_eq!(told.borrow().len(), 2);
    watches.push(Box::new(Forbid("Writer<String>")));
    assert_eq!(
        run(observe(&cx, &watches, async |cx| both_effects(cx).await)),
        Ok(Err("Writer<String> forbidden".to_string()))
    );
}
