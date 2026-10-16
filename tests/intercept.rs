//! INTERCEPTORS: control and audit of effects. A plain effect intercepted for a part of the program; a sensitive one
//! handled only under its audit — each operation told, passed on, changed or refused; an observer seeing every
//! operation of every effect on its way out, and each answer, as many times as the rest gets it.

use okay::audit::*;
use okay::effects::*;
use okay::*;

// A SENSITIVE EFFECT: payments

#[derive(Clone, Debug)]
enum Pay {
    /// money sent to an account: a receipt
    Transfer(String, i64, Reply<String>),
    /// the balance
    Balance(Reply<i64>),
}
impl Effect for Pay {
    type Kind = Sensitive;
}

fn transfer<R: Member<Pay, I>, I>(to: &str, amount: i64) -> Eff<R, String> {
    let to = to.to_string();
    perform(|r| Pay::Transfer(to, amount, r))
}

fn balance<R: Member<Pay, I>, I>() -> Eff<R, i64> {
    perform(Pay::Balance)
}

/// the bank: a balance, each transfer taken from it
fn bank<T, A: 'static>(funds: i64) -> Handler<Pay, T, A, A> {
    Handler::stateful(funds, |b: &mut i64, op| match op {
        Pay::Transfer(to, amount, r) => {
            *b -= amount;
            r.give(format!("ok:{to}:{amount}"))
        }
        Pay::Balance(r) => r.give(*b)
    }, |_, a| a)
}

/// an account as the trail shows it: its last four
fn masked(account: &str) -> String {
    let tail: String = account.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("****{tail}")
}

/// THE AUDIT OF PAYMENTS: a transfer over the limit refused, raised; the account written one way (changed before it
/// is passed on); each one told, its account masked, with its receipt; the balance told with its value
fn guarded<T, I1, I2, I3, A: 'static>(limit: i64) -> Interceptor<Pay, T, A, A>
where
    T: Member<Audited<Pay>, I1> + Member<Throws<String>, I2> + Member<Audit, I3> + 'static,
    I1: 'static,
    I2: 'static,
    I3: 'static,
{
    Interceptor::new(pure, move |op, k, pass| match op {
        Pay::Transfer(to, amount, r) => {
            let to = to.trim().to_uppercase();
            if amount > limit {
                let why = format!("refused: {amount} to {} over the limit {limit}", masked(&to));
                record::<T, I3>(why.clone()).flat_map(move |_| raise::<String, A, T, I2>(why.clone()))
            } else {
                let account = to.clone();
                pass.perform::<T, I1, String>(move |r2| Pay::Transfer(account, amount, r2)).flat_map(move |receipt: String| {
                    let k = k.clone();
                    let back = receipt.clone();
                    record::<T, I3>(format!("transfer {amount} to {} → {receipt}", masked(&to)))
                        .flat_map(move |_| r.resume(&k, back.clone()))
                })
            }
        }
        Pay::Balance(r) => pass.perform::<T, I1, i64>(Pay::Balance).flat_map(move |b: i64| {
            let k = k.clone();
            record::<T, I3>(format!("balance → {b}")).flat_map(move |_| r.resume(&k, b))
        }),
    })
}

type Payments = row![Pay, Throws<String>, Audit];

/// "a sensitive effect under its audit: passed on changed, told masked, refused over the limit"
#[test]
fn a_sensitive_effect_under_its_audit_passed_on_changed_told_masked_refused_over_the_limit() {
    let p: Eff<Payments, i64> = eff! { data;
        a <- transfer(" acct-1234 ", 100);
        b <- balance();
        _ <- transfer("acct-77", 5000);
        yield b
    };
    let (trail, result) = p
        .audit::<Pay, _, _>(guarded(1000))
        .handle_audited::<Pay, _, _>(bank(1000))
        .handle(either())
        .handle(trail())
        .run();
    assert_eq!(
        trail,
        vec![
            "transfer 100 to ****1234 → ok:ACCT-1234:100".to_string(),
            "balance → 900".to_string(),
            "refused: 5000 to ****T-77 over the limit 1000".to_string(),
        ]
    );
    assert_eq!(result, Err("refused: 5000 to ****T-77 over the limit 1000".to_string()));
}

/// "a plain effect intercepted for a part of the program: its clause runs outside it, its own operation reaches the
/// handler" — okay-core's guide, `shouting`
#[test]
fn a_plain_effect_intercepted_for_a_part_of_the_program() {
    type R = row![Writer<String>];
    fn shouting<A: 'static>() -> Handler<Writer<String>, R, A, A> {
        Handler::deep(pure, |op: Writer<String>, k| match op {
            Writer::Tell(s, r) => tell::<String, R, _>(s.to_uppercase()).flat_map(move |_| r.resume(&k, ())),
        })
    }
    let p: Eff<R, ()> = eff! { data;
        tell("a".to_string());
        tell::<String, R, _>("b".to_string()).intercept(shouting())
    };
    assert_eq!(p.handle(collect()).run(), (vec!["a".to_string(), "B".to_string()], ()));
}

/// an observer telling every operation and its answer to the trail
fn telling<T: Member<Audit, I> + 'static, I: 'static, A: 'static>() -> Observer<T, A, A> {
    Observer::new(pure, |op, k| {
        let line = format!("{} {}", op.effect(), op.describe());
        record::<T, I>(line).flat_map(move |_| op.clone().proceed_then(&k, |answer| record::<T, I>(format!("  → {answer}"))))
    })
}

/// "an observer sees every operation of every effect on its way out, and its answer; the sensitive ones audited"
#[test]
fn an_observer_sees_every_operation_of_every_effect_and_its_answer() {
    type R = row![Reader<i64>, State<i64>, Pay, Audit];
    let p: Eff<R, String> = eff! { data;
        n <- ask::<i64, _, _>();
        _ <- modify::<i64, _, _>(move |s| s + n);
        s <- get::<i64, _, _>();
        transfer("acct", s)
    };
    let (trail, (_, receipt)) = p
        .observe(telling())
        .handle(reader(5))
        .handle(state(1))
        .handle_audited::<Pay, _, _>(bank(100))
        .handle(trail())
        .run();
    assert_eq!(receipt, "ok:acct:6");
    assert_eq!(
        trail,
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
    type R = row![Reader<i64>, Pay, Throws<String>, Audit];
    fn read_only<T, I1, I2, A: 'static>() -> Observer<T, A, A>
    where
        T: Member<Throws<String>, I1> + Member<Audit, I2> + 'static,
        I1: 'static,
        I2: 'static,
    {
        Observer::new(pure, |op, k| {
            if op.effect() == "Pay" {
                let why = format!("read-only: {} refused", op.describe());
                record::<T, I2>(why.clone()).flat_map(move |_| raise::<String, A, T, I1>(why.clone()))
            } else {
                op.proceed(&k)
            }
        })
    }
    let p: Eff<R, String> = eff! { data;
        n <- ask::<i64, _, _>();
        transfer("acct", n)
    };
    let (trail, result) = p
        .observe(read_only())
        .handle(reader(5))
        .handle_audited::<Pay, _, _>(bank(100))
        .handle(either())
        .handle(trail())
        .run();
    assert_eq!(trail, vec!["read-only: Transfer(\"acct\", 5, _) refused"]);
    assert_eq!(result, Err("read-only: Transfer(\"acct\", 5, _) refused".to_string()));
}

/// "an answer told each time the rest gets it: a choice resumed twice, two answers"
#[test]
fn an_answer_told_each_time_the_rest_gets_it() {
    type R = row![Choose, Audit];
    let p: Eff<R, bool> = flip();
    let (trail, paths) = p.observe(telling()).handle(all()).handle(trail()).run();
    assert_eq!(paths, vec![true, false]);
    assert_eq!(trail, vec!["Choose Among(2, _)", "  → 0", "  → 1"]);
}
