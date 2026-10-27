//! The services as effects (okay::services): a test answers them at once, production with the real ones
use std::cell::RefCell;
use std::fmt::Debug;
use std::time::Instant;

use okay::services::*;
use okay::typed::*;

okay::needs! {
    async fn stamp(cx: [Clock, Fresh, Log]) -> (u64, u64) {
        let id = fresh(cx).await;
        sleep_ms(cx, 250).await;
        let t = now_ms(cx).await;
        debug(cx, format!("id {id}")).await;
        info(cx, format!("{id} at {t}")).await;
        (id, t)
    }
}

#[test]
fn virtual_time_at_once_the_system_clock_waiting() {
    let cx = Cx(VirtualClock::at(10_000), Cx(FreshFrom::new(7), Cx(LogCollect::at_least(Level::Info), End)));
    let started = Instant::now();
    assert_eq!(run(stamp(&cx)), Ok((7, 10_250)));
    assert!(started.elapsed().as_millis() < 100, "virtual: no real wait");
    assert_eq!(cx.1.1.0.lines(), vec![(Level::Info, "7 at 10250".to_string())]);

    let cx = Cx(SystemClock, Cx(FreshFrom::new(1), Cx(LogCollect::at_least(Level::Debug), End)));
    let started = Instant::now();
    let (id, _) = block_on(stamp(&cx));
    assert_eq!(id, 1);
    assert!(started.elapsed().as_millis() >= 240, "the system's clock waits");
    assert_eq!(cx.1.1.0.lines().len(), 2);
}

#[test]
fn seeded_randomness_the_same_each_run() {
    okay::needs! {
        async fn draws(cx: [Random]) -> Vec<u64> {
            let mut v = vec![];
            for _ in 0..5 {
                v.push(random_below(cx, 100).await);
            }
            v
        }
    }
    let a = run(draws(&Cx(SplitMix::seeded(42), End))).unwrap();
    let b = run(draws(&Cx(SplitMix::seeded(42), End))).unwrap();
    let c = run(draws(&Cx(SplitMix::seeded(43), End))).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(a.iter().all(|n| *n < 100));
    let f = run(random_f64(&Cx(SplitMix::seeded(1), End))).unwrap();
    assert!((0.0..1.0).contains(&f));
}

#[test]
fn a_log_to_a_sink() {
    let seen = RefCell::new(vec![]);
    let cx = Cx(LogTo::new(Level::Warn, |l, s: &str| seen.borrow_mut().push(format!("{l:?} {s}"))), End);
    run(async {
        info(&cx, "quiet".into()).await;
        warn(&cx, "loud".into()).await;
        error(&cx, "louder".into()).await;
    })
    .unwrap();
    assert_eq!(*seen.borrow(), vec!["Warn loud", "Error louder"]);
}

/// every read of a secret seen, a secret not on the list refused
struct SecretsAudit(RefCell<Vec<String>>, &'static [&'static str]);
impl<R> Interceptor<Secrets, R, ()> for SecretsAudit {
    fn intercept<'a, H: Handles<Secrets>>(&'a self, op: Secrets, pass: Pass<'a, H>, _: &'a R) -> Step<'a> {
        match op {
            Secrets::Read(name, r) => {
                self.0.borrow_mut().push(name.clone());
                if !self.1.contains(&name.as_str()) {
                    return Step::Now(r.give(None));
                }
                pass.perform(|r2| Secrets::Read(name, r2)).map_or(Step::Later, |s| Step::Now(r.give(s)))
            }
        }
    }
}

#[test]
fn the_environment_plain_its_secrets_only_under_an_audit() {
    okay::needs! {
        async fn connect(cx: [Env, Secrets]) -> String {
            let host = variable(cx, "DB_HOST".into()).await.unwrap_or_default();
            let pass = secret(cx, "DB_PASS".into()).await;
            let other = secret(cx, "ROOT_KEY".into()).await;
            format!("{host} {:?} {:?} {}", pass, other, pass.as_ref().map_or("", |s| s.expose()))
        }
    }
    let env = EnvOf::new([("DB_HOST", "db.local")], [("DB_PASS", "hunter2"), ("ROOT_KEY", "k")]);
    let audit = SecretsAudit(RefCell::new(vec![]), &["DB_PASS"]);
    let cx = Cx(&env, Cx(Audited::new(&env, audit), End));
    let got = run(connect(&cx)).unwrap();
    // printed, a secret says nothing; refused, none
    assert_eq!(got, "db.local Some(Secret(***)) None hunter2");
    assert_eq!(*cx.1.0.interceptor().0.borrow(), vec!["DB_PASS".to_string(), "ROOT_KEY".to_string()]);
}

#[test]
fn a_store_in_memory() {
    okay::needs! {
        async fn count<K: Clone + Debug + Eq + std::hash::Hash>(cx: [Store<K, i64>], k: K) -> i64 {
            let n = load(cx, k.clone()).await.unwrap_or(0) + 1;
            save(cx, k, n).await;
            n
        }
    }
    let cx = Cx(InMemory::of([("a".to_string(), 10i64)]), End);
    assert_eq!(run(count(&cx, "a".to_string())), Ok(11));
    assert_eq!(run(count(&cx, "b".to_string())), Ok(1));
    assert_eq!(run(remove::<String, i64, _, _>(&cx, "a".to_string())), Ok(Some(11)));
    assert_eq!(cx.0.into_map().len(), 1);
}
