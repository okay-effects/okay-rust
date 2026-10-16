//! okay-core's TestLaws: the monad's laws, seen by what a program gives when run — with effects of each kind
//! (the law in a `$` changing its answer waits for stage 2's control)

use okay::effects::*;
use okay::*;

type R = row![Reader<i64>, Writer<String>, State<i64>, Choose];

fn run_r<A: Clone + 'static>(p: Eff<R, A>) -> Vec<(i64, (Vec<String>, A))> {
    p.handle(reader(7)).handle(collect()).handle(state(0)).handle(all()).run()
}

/// a program of each effect, its value from `x`
fn f(x: i64) -> Eff<R, i64> {
    eff! { data;
        r <- ask::<i64, _, _>();
        _ <- tell::<String, _, _>(format!("f {x}"));
        s <- get::<i64, _, _>();
        _ <- put::<i64, _, _>(s + x);
        b <- flip();
        yield if b { x + r } else { x * r }
    }
}

fn g(x: i64) -> Eff<R, i64> {
    tell::<String, R, _>(format!("g {x}")).flat_map(|_| get::<i64, R, _>()).map(move |s| s + x)
}

fn m() -> Eff<R, i64> {
    f(1)
}

#[test]
fn left_identity() {
    assert_eq!(run_r(pure::<R, i64>(3).flat_map(f)), run_r(f(3)));
}

#[test]
fn right_identity() {
    assert_eq!(run_r(m().flat_map(pure::<R, i64>)), run_r(m()));
}

#[test]
fn associativity() {
    assert_eq!(run_r(m().flat_map(f).flat_map(g)), run_r(m().flat_map(|x| f(x).flat_map(g))));
}

#[test]
fn map_by_identity_by_two_functions_and_by_flat_map() {
    let h = |x: i64| x * 10;
    let i = |x: i64| x - 1;
    assert_eq!(run_r(m().map(|x| x)), run_r(m()));
    assert_eq!(run_r(m().map(h).map(i)), run_r(m().map(move |x| i(h(x)))));
    assert_eq!(run_r(m().map(h)), run_r(m().flat_map(move |x| pure::<R, i64>(h(x)))));
}
