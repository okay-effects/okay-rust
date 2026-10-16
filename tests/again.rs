//! okay-core's TestAgain: a handler of an effect at any place in the row, the row kept (`intercept`) — its clauses'
//! own operations of the effect to the handler outside

use okay::effects::*;
use okay::*;

type R = row![Writer<String>, Reader<i64>];

fn body() -> Eff<R, i64> {
    eff! { data;
        a <- ask::<i64, _, _>();
        _ <- tell::<String, _, _>(format!("a={a}"));
        b <- ask::<i64, _, _>();
        yield a + b
    }
}

/// "an effect past the head handled again: the body's operations to the interceptor, its own outside"
#[test]
fn an_effect_past_the_head_handled_again() {
    let doubling = Handler::deep(pure, |op: Reader<i64>, k| match op {
        Reader::Ask(r) => ask::<i64, R, _>().flat_map(move |v| r.resume(&k, 2 * v)),
    });
    let p: Eff<R, i64> = body().intercept(doubling);
    assert_eq!(p.handle(collect()).handle(reader(5)).run(), (vec!["a=10".to_string()], 20));
}

/// "the same at the head: the writer intercepted, its tells prefixed and told outside"
#[test]
fn the_same_at_the_head() {
    let prefixing = Handler::deep(pure, |op: Writer<String>, k| match op {
        Writer::Tell(w, r) => tell::<String, R, _>(format!("> {w}")).flat_map(move |_| r.resume(&k, ())),
    });
    let p: Eff<R, i64> = body().intercept(prefixing);
    assert_eq!(p.handle(collect()).handle(reader(5)).run(), (vec!["> a=5".to_string()], 10));
}
