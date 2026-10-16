//! INTERCEPTORS, ONE API COMPILED BY THE HANDLER'S TYPE (okay::typed): tests/intercept.rs's programs and trails. A
//! sensitive effect handled only under its interceptor — each operation told, passed on changed, refused; a plain one
//! intercepted for a part of the program; a watch seeing every operation of every effect, and each answer

use std::cell::RefCell;
use std::fmt::Debug;

use okay::audit::Audit;
use okay::effects::{Throws, Writer};
use okay::typed::*;
use okay::{eff, effect};

// A SENSITIVE EFFECT: payments

effect! {
    enum Pay: Sensitive {
        /// money sent to an account: a receipt
        Transfer(to: String, amount: i64) -> String = transfer;
        /// the balance
        Balance() -> i64 = balance;
    }
}

/// the bank: a balance, each transfer taken from it
struct Bank(RefCell<i64>);
impl Handles<Pay> for Bank {
    fn handle(&self, op: Pay) -> Step<'_> {
        match op {
            Pay::Transfer(to, amount, r) => {
                *self.0.borrow_mut() -= amount;
                Step::Now(r.give(format!("ok:{to}:{amount}")))
            }
            Pay::Balance(r) => Step::Now(r.give(*self.0.borrow())),
        }
    }
}

/// an account as the trail shows it: its last four
fn masked(account: &str) -> String {
    let tail: String = account.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("****{tail}")
}

/// THE AUDIT OF PAYMENTS: a transfer over the limit refused, raised; the account written one way (changed before it
/// is passed on); each one told, its account masked, with its receipt; the balance told with its value
struct Guarded {
    limit: i64,
}
impl<R, I1, I2> Interceptor<Pay, R, (I1, I2)> for Guarded
where
    R: Has<Throws<String>, I1> + Has<Audit, I2>,
{
    fn intercept<H: Handles<Pay>>(&self, op: Pay, pass: Pass<'_, H>, cx: &R) -> Step<'_> {
        match op {
            Pay::Transfer(to, amount, r) => {
                let to = to.trim().to_uppercase();
                if amount > self.limit {
                    let why = format!("refused: {amount} to {} over the limit {}", masked(&to), self.limit);
                    now(record(cx, why.clone()));
                    let _: Option<()> = now(raise(cx, why));
                    return Step::Later;
                }
                let account = to.clone();
                let Some(receipt) = pass.perform(|r2| Pay::Transfer(account, amount, r2)) else { return Step::Later };
                now(record(cx, format!("transfer {amount} to {} → {receipt}", masked(&to))));
                Step::Now(r.give(receipt))
            }
            Pay::Balance(r) => {
                let Some(b) = pass.perform(Pay::Balance) else { return Step::Later };
                now(record(cx, format!("balance → {b}")));
                Step::Now(r.give(b))
            }
        }
    }
}

okay::needs! {
    async fn payments(cx: [Pay]) -> i64 {
        eff! {
            _ <- transfer(cx, " acct-1234 ".into(), 100);
            b <- balance(cx);
            _ <- transfer(cx, "acct-77".into(), 5000);
            yield b
        }
        .await
    }
}

/// "a sensitive effect under its audit: passed on changed, told masked, refused over the limit"
#[test]
fn a_sensitive_effect_under_its_audit_passed_on_changed_told_masked_refused_over_the_limit() {
    let cx = Cx(Audited::new(Bank(RefCell::new(1000)), Guarded { limit: 1000 }), Cx(Raised::new(), Cx(Trail::new(), End)));
    let result = run(catch(&cx.1.0, payments(&cx)));
    assert_eq!(
        cx.1.1.0.take(),
        vec![
            "transfer 100 to ****1234 → ok:ACCT-1234:100".to_string(),
            "balance → 900".to_string(),
            "refused: 5000 to ****T-77 over the limit 1000".to_string(),
        ]
    );
    assert_eq!(result, Ok(Err("refused: 5000 to ****T-77 over the limit 1000".to_string())));
    let (bank, _) = cx.0.into_parts();
    assert_eq!(bank.0.into_inner(), 900);
}

/// "a plain effect intercepted for a part of the program: its own operation reaches the handler" — okay-core's
/// guide, `shouting`: the part run with its own context, the handler shared
#[test]
fn a_plain_effect_intercepted_for_a_part_of_the_program() {
    struct Shouting;
    impl<R> Interceptor<Writer<String>, R, ()> for Shouting {
        fn intercept<H: Handles<Writer<String>>>(&self, op: Writer<String>, pass: Pass<'_, H>, _: &R) -> Step<'_> {
            match op {
                Writer::Tell(s, r) => match pass.perform(|r2| Writer::Tell(s.to_uppercase(), r2)) {
                    Some(()) => Step::Now(r.give(())),
                    None => Step::Later,
                },
            }
        }
    }
    let collected = Collect::<String>::new();
    let cx = Cx(&collected, End);
    let loud = Cx(Audited::new(&collected, Shouting), End);
    let r = run(eff! {
        tell(&cx, "a".to_string());
        tell(&loud, "b".to_string())
    });
    assert_eq!(r, Ok(()));
    assert_eq!(collected.take(), vec!["a", "B"]);
}

/// passes every payment on as it is
struct Passing;
impl<R> Interceptor<Pay, R, ()> for Passing {
    fn intercept<H: Handles<Pay>>(&self, op: Pay, pass: Pass<'_, H>, _: &R) -> Step<'_> {
        match op {
            Pay::Transfer(to, n, r) => pass.perform(|r2| Pay::Transfer(to, n, r2)).map_or(Step::Later, |s| Step::Now(r.give(s))),
            Pay::Balance(r) => pass.perform(Pay::Balance).map_or(Step::Later, |b| Step::Now(r.give(b))),
        }
    }
}

/// a watch telling every operation and its answer to a trail
struct Telling<'t>(&'t Trail);
impl Watch for Telling<'_> {
    fn before(&self, effect: &str, op: &dyn Debug) -> Result<(), String> {
        self.0.push(format!("{effect} {op:?}"));
        Ok(())
    }
    fn after(&self, _: &str, _: &dyn Debug, answer: &dyn Debug) {
        self.0.push(format!("  → {answer:?}"));
    }
}
/// "an observer sees every operation of every effect on its way out, and its answer; the sensitive ones audited"
#[test]
fn an_observer_sees_every_operation_of_every_effect_and_its_answer() {
    let cx = Cx(AskWith(5i64), Cx(StateCell::new(1i64), Cx(Audited::new(Bank(RefCell::new(100)), Passing), End)));
    let trail = Trail::new();
    let receipt = run(observe(&cx, Telling(&trail), async |cx| {
        eff! {
            n <- ask(cx);
            _ <- modify(cx, |s| s + n);
            s <- get(cx);
            transfer(cx, "acct".into(), s)
        }
        .await
    }));
    assert_eq!(receipt, Ok(Ok("ok:acct:6".to_string())));
    assert_eq!(
        trail.take(),
        vec![
            "Reader<i64> Ask(_)", "  → 5",
            "State<i64> Get(_)", "  → 1",
            "State<i64> Put(6, _)", "  → ()",
            "State<i64> Get(_)", "  → 6",
            "Pay Transfer(\"acct\", 6, _)", "  → \"ok:acct:6\"",
        ]
    );
}

/// "an observer refusing a sensitive effect: a read-only run"
#[test]
fn an_observer_refusing_a_sensitive_effect_a_read_only_run() {
    struct ReadOnly<'t>(&'t Trail);
    impl Watch for ReadOnly<'_> {
        fn before(&self, effect: &str, op: &dyn Debug) -> Result<(), String> {
            if effect != "Pay" {
                return Ok(());
            }
            let why = format!("read-only: {op:?} refused");
            self.0.push(why.clone());
            Err(why)
        }
    }
    let cx = Cx(AskWith(5i64), Cx(Audited::new(Bank(RefCell::new(100)), Passing), End));
    let trail = Trail::new();
    let result = run(observe(&cx, ReadOnly(&trail), async |cx| {
        eff! {
            n <- ask(cx);
            transfer(cx, "acct".into(), n)
        }
        .await
    }));
    assert_eq!(trail.take(), vec!["read-only: Transfer(\"acct\", 5, _) refused"]);
    assert_eq!(result, Ok(Err("read-only: Transfer(\"acct\", 5, _) refused".to_string())));
}

/// "an answer told each time the rest gets it: a choice resumed twice, two answers" — by replay the path runs twice,
/// a watch inside the choice sees the operation each time, with its answer
#[test]
fn an_answer_told_each_time_the_rest_gets_it() {
    let trail = Trail::new();
    let paths = run(all(&End, async |cx| observe(cx, Telling(&trail), async |cx| flip(cx).await).await.unwrap()));
    assert_eq!(paths, Ok(vec![true, false]));
    assert_eq!(trail.take(), vec!["Choose Among(2, _)", "  → 0", "Choose Among(2, _)", "  → 1"]);
}

/// "a plain effect beside an audited one" — what does not compile (a bare bank, a bank under no interceptor of it,
/// the bank reached past its interceptor): okay::typed's compile_fail doc tests
#[test]
fn a_plain_effect_beside_an_audited_one() {
    let cx = Cx(StateCell::new(1i64), Cx(Audited::new(Bank(RefCell::new(10)), Passing), End));
    let r = run(eff! {
        s <- get(&cx);
        transfer(&cx, "a".into(), s)
    });
    assert_eq!(r, Ok("ok:a:1".to_string()));
}
