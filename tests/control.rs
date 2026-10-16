//! okay-core's TestControl: CONTROL IN THE ROW — a `$` in the row (`Delim<T>`), a shift0 to the nearest; the operations
//! of the row outside going out past the `$` to their handlers; handlers inside the `$`, in the `k` of a shift0 past
//! them

use std::rc::Rc;

use okay::cont::{reset, shift0};
use okay::effects::*;
use okay::*;

/// a shift0 under the `$` over `R`, its rest a program of effects over `R`
fn sh<R: 'static, X: 'static, O: Over<R>>(f: impl Fn(K1<X, Eff<R, i64>>) -> O + 'static) -> Cont<Delim<R>, Eff<R, i64>, O, X> {
    shift0::<Delim<R>, R, _, X, Eff<R, i64>, O>(f)
}

fn twice<T: 'static>(k: K1<i64, Eff<T, i64>>, a: i64, b: i64) -> Eff<T, i64> {
    let k2 = k.clone();
    k(a).flat_map(move |x| k2(b).map(move |y| x + y))
}

#[test]
fn an_operation_under_a_dollar_and_a_shift0_resumed_twice_out_of_the_dollar_to_its_handler() {
    type R = row![Reader<i64>];
    // `$ (ask + shift0(k => k(1) + k(2)))`, `ask` 10
    let body = || sh::<R, i64, Eff<R, i64>>(|k| twice(k, 1, 2));
    let p: Eff<R, i64> = dollar(ask::<i64, Delim<R>, _>().cont::<Eff<R, i64>>().flat_map(move |n| body().map(move |x| n + x)), pure);
    assert_eq!(p.handle(reader(10)).run(), 23);
}

#[test]
fn a_deep_handlers_k_taken_past_two_dollars_resumed_twice_the_dollars_put_back_their_shift0s_working() {
    type R = row![Choose];
    type L = Eff<R, i64>;
    type M = Cont<Delim<R>, L, L, i64>;
    // `$ ($ (flip >>= b => shift0(k => k(b ? 1 : 2) + k(10))) * 100)`: the inner `$` under the outer, a row deeper
    let inner: Cont<Delim<Delim<R>>, M, M, i64> = flip::<Delim<Delim<R>>, _>().cont::<M>().flat_map(|b: bool| {
        shift0::<Delim<Delim<R>>, Delim<R>, _, i64, M, M>(move |k| {
            let k2 = k.clone();
            k(if b { 1 } else { 2 }).flat_map(move |a| k2(10).map(move |c| a + c))
        })
    });
    let outer: M = dollar(inner, |a| pure_c(a * 100));
    let p: L = dollar(outer, pure);
    assert_eq!(p.handle(all()).run(), vec![1100, 1200]);
}

#[test]
fn printf_under_choose_the_answer_moved_to_a_function_one_a_path() {
    type R = row![Choose];
    type K = Eff<R, String>;
    type S = Rc<dyn Fn(i64) -> K>;
    // `$ (flip >>= b => "got " + b + shift0(k => pure(x => k(x))))`: the answer `String` turned `Int => K`
    let body = || shift0::<Delim<R>, R, _, i64, K, Eff<R, S>>(|k| pure(Rc::new(move |x| k(x)) as S));
    let p: Eff<R, S> = dollar(
        flip::<Delim<R>, _>().cont::<Eff<R, S>>().flat_map(move |b| body().map(move |n| format!("{b} {n}"))),
        |s| pure(format!("got {s}")),
    );
    let fs = p.handle(all()).run();
    let got: Vec<Vec<String>> = fs.iter().map(|f| f(42).handle(all()).run()).collect();
    assert_eq!(got, vec![vec!["got true 42".to_string()], vec!["got false 42".to_string()]]);
}

#[test]
fn a_deep_handlers_program_under_the_dollar_its_operations_of_others_out_of_it() {
    type R = row![Reader<i64>];
    type Q = Cons<Choose, Delim<R>>;
    let p: Eff<Q, i64> = flip::<Q, _>().flat_map(|b| ask::<i64, Q, _>().map(move |n| if b { n } else { -n }));
    assert_eq!(reset(p.handle(all()).cont()).handle(reader(3)).run(), vec![3, -3]);
}

#[test]
fn stack_safe_a_loop_of_operations_under_a_dollar_out_of_it() {
    type R = row![Reader<i64>];
    fn lp(i: i64, acc: i64) -> Cont<Delim<R>, Eff<R, i64>, Eff<R, i64>, i64> {
        if i == 0 { pure_c(acc) } else { ask::<i64, Delim<R>, _>().cont().flat_map(move |n| lp(i - 1, acc + n)) }
    }
    assert_eq!(reset(lp(100_000, 0)).handle(reader(1)).run(), 100_000);
}

#[test]
fn stack_safe_many_dollars_in_a_program_of_effects_each_a_shift0_resumed_in_it() {
    type R = row![Reader<i64>];
    fn one() -> Eff<R, i64> {
        dollar(sh::<R, i64, Eff<R, i64>>(|k| k(1).map(|a| a + 1)), pure)
    }
    fn many(i: i64, acc: i64) -> Eff<R, i64> {
        if i == 0 { pure(acc) } else { one().flat_map(move |x| many(i - 1, acc + x)) }
    }
    assert_eq!(many(100_000, 0).handle(reader(0)).run(), 200_000);
}

// HANDLERS INSIDE THE $

/// a cell counting its reads: a state, each `Get` the count so far
fn counter<T, A: 'static>(s0: i64) -> Passing<State<i64>, T, A, A> {
    Passing::stateful(s0, |s: &mut i64, op| match op {
        State::Get(r) => { let v = *s; *s += 1; r.give(v) }
        State::Put(n, r) => { *s = n; r.give(()) }
    }, |_, a| a)
}

fn reading<T, A: 'static>(v: i64) -> Passing<Reader<i64>, T, A, A> {
    Passing::answering(move |op| match op { Reader::Ask(r) => r.give(v) }, |a| a)
}

#[test]
fn printf_with_a_state_inside_the_dollar_taken_into_k_each_resumption_from_the_state_as_it_was() {
    type S = Rc<dyn Fn(i64) -> String>;
    type P = Eff<Nil, String>;
    type Q = Cons<State<i64>, Reset>;
    // `$ (handle_counter(get ; x <- shift0(k => pure(x => k(x))) ; get ; s"$n $x $m"))`
    let body: Cont<Q, P, Eff<Nil, S>, String> = get::<i64, Q, _>().cont::<Eff<Nil, S>>().flat_map(|n| {
        shift0::<Q, Nil, _, i64, P, Eff<Nil, S>>(|k| pure(Rc::new(move |x| k(x).run()) as S))
            .flat_map(move |x| get::<i64, Q, _>().cont::<P>().map(move |m| format!("{n} {x} {m}")))
    });
    let f: S = dollar(body.handle(counter(0)), |s| pure(format!("<{s}>"))).run();
    assert_eq!(f(42), "<0 42 1>");
    assert_eq!(f(7), "<0 7 1>");
}

#[test]
fn a_for_operations_of_the_row_and_a_shift0_moving_the_answer() {
    type T = row![Reader<i64>];
    type Q = Delim<T>;
    type S = Rc<dyn Fn(i64) -> String>;
    type P = Eff<T, String>;
    let body: Cont<Q, P, Eff<T, S>, String> = ask::<i64, Q, _>().cont::<Eff<T, S>>().flat_map(|n| {
        shift0::<Q, T, _, i64, P, Eff<T, S>>(move |k| pure(Rc::new(move |x| k(x).handle(reader(n)).run()) as S))
            .flat_map(move |x| ask::<i64, Q, _>().cont::<P>().map(move |m| format!("{n} {x} {m}")))
    });
    let f: S = dollar(body, |s| pure(format!("<{s}>"))).handle(reader(5)).run();
    assert_eq!(f(42), "<5 42 5>");
}

#[test]
fn a_program_of_effects_under_a_dollar_beside_a_shift0_lifted() {
    type R = Cons<Reader<i64>, Reset>;
    fn under() -> Eff<R, i64> {
        ask::<i64, R, _>().map(|n| n + 1)
    }
    let body: Cont<R, Eff<Nil, i64>, Eff<Nil, i64>, i64> = under().flat_map(|a| {
        shift0::<R, Nil, _, i64, Eff<Nil, i64>, Eff<Nil, i64>>(move |k| {
            let k2 = k.clone();
            k(a).flat_map(move |b| k2(b))
        })
        .map(move |x| a + x)
    });
    assert_eq!(dollar(body.handle(reading(41)), pure).run(), 126);
    assert_eq!(reset(under().handle(reader(41)).cont()).run(), 42);
}

#[test]
fn an_answering_handler_inside_the_dollar_under_a_shift0_resumed_twice_in_k_both_times() {
    type R = Cons<Reader<i64>, Reset>;
    let body: Cont<R, Eff<Nil, i64>, Eff<Nil, i64>, i64> = shift0::<R, Nil, _, i64, Eff<Nil, i64>, Eff<Nil, i64>>(|k| twice(k, 1, 2))
        .flat_map(|x| ask::<i64, R, _>().cont::<Eff<Nil, i64>>().map(move |n| n + x));
    assert_eq!(dollar(body.handle(reading(10)), pure).run(), 23);
}

#[test]
fn a_translating_handler_inside_the_dollar_each_operation_a_program_over_the_row_outside() {
    type O = row![Reader<i64>];
    type R = Cons<Writer<String>, Delim<O>>;
    let tell_out: Passing<Writer<String>, Delim<O>, i64, i64> = Passing::translating(|op| match op {
        Writer::Tell(_, r) => ask::<i64, Delim<O>, _>().map(move |_| r.give(())),
    });
    let body: Cont<R, Eff<O, i64>, Eff<O, i64>, i64> = tell::<String, R, _>("a".into()).cont().flat_map(|_| {
        shift0::<R, O, _, i64, Eff<O, i64>, Eff<O, i64>>(|k| k(1)).map(|x| x + 1)
    });
    assert_eq!(dollar(body.handle(tell_out), pure).handle(reader(5)).run(), 2);
}

#[test]
fn a_handler_of_an_effect_outside_the_dollar_put_inside_it() {
    type R = Cons<Reader<i64>, Delim<row![Writer<String>]>>;
    let p: Eff<R, i64> = ask::<i64, R, _>().flat_map(|n| tell::<String, R, _>(format!("n {n}")).map(move |_| n));
    let q = reset(p.handle(reader(4)).cont()).handle(collect());
    assert_eq!(q.run(), (vec!["n 4".to_string()], 4));
}

#[test]
fn atm_in_a_for_three_lines_the_dollar_makes_it_a_program_of_effects() {
    type S = Rc<dyn Fn(i64) -> String>;
    type R = Cons<Reader<i64>, Reset>;
    let body = eff! { data;
        n <- ask::<i64, R, _>();
        x <- shift0::<R, Nil, _, i64, Eff<Nil, String>, Eff<Nil, S>>(|k| pure(Rc::new(move |x| k(x).run()) as S));
        m <- ask::<i64, R, _>();
        yield format!("{n} {x} {m}")
    };
    let p: Eff<Nil, S> = dollar(body.handle(reading(5)), |s| pure(format!("<{s}>")));
    assert_eq!(p.run()(42), "<5 42 5>");
}
