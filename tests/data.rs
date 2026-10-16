//! okay-core's TestData: `k` AS DATA — a clause's `k` kept in the handler's answer, resumed after the handler has
//! answered, anywhere, any number of times

use std::rc::Rc;

use okay::effects::*;
use okay::*;

type E = Emit<i64>;
type Thunk<T> = Rc<dyn Fn() -> Eff<T, Gen<i64, T>>>;

fn emits<R: Member<E, Ix> + 'static, Ix: 'static>(vs: Rc<Vec<i64>>, i: usize) -> Eff<R, ()> {
    if i == vs.len() { pure(()) } else { emit::<i64, R, Ix>(vs[i]).flat_map(move |_| emits::<R, Ix>(vs.clone(), i + 1)) }
}

/// two generators zipped: each resumed by turns, outside its handler
fn zip<T: 'static>(a: Thunk<T>, b: Thunk<T>) -> Eff<T, Vec<(i64, i64)>> {
    a().flat_map(move |ga| {
        b().flat_map(move |gb| match (ga.clone(), gb) {
            (Gen::Next(x, nx), Gen::Next(y, ny)) => zip(nx, ny).map(move |mut rest| {
                rest.insert(0, (x, y));
                rest
            }),
            _ => pure(Vec::new()),
        })
    })
}

#[test]
fn generators_zipped_each_k_kept_in_a_value_resumed_by_the_others_turn() {
    type R = row![E];
    let a: Thunk<Nil> = Rc::new(|| emits::<R, _>(Rc::new((1..=3).collect()), 0).handle(generate()));
    let b: Thunk<Nil> = Rc::new(|| emits::<R, _>(Rc::new((10..=50).step_by(10).collect()), 0).handle(generate()));
    assert_eq!(zip(a, b).run(), vec![(1, 10), (2, 20), (3, 30)]);
}

#[test]
fn a_kept_k_resumed_under_the_handlers_of_the_row_outside_a_state_they_share() {
    type T = row![State<i64>];
    type ET = row![E, State<i64>];
    fn counted(vs: Rc<Vec<i64>>, i: usize) -> Eff<ET, ()> {
        if i == vs.len() {
            return pure(());
        }
        get::<i64, ET, _>()
            .flat_map(|s| put::<i64, ET, _>(s + 1))
            .flat_map({ let vs = vs.clone(); move |_| emit::<i64, ET, _>(vs[i]) })
            .flat_map(move |_| counted(vs.clone(), i + 1))
    }
    let a: Thunk<T> = Rc::new(|| counted(Rc::new(vec![1, 2]), 0).handle(generate()));
    let b: Thunk<T> = Rc::new(|| counted(Rc::new((5..=9).collect()), 0).handle(generate()));
    assert_eq!(zip(a, b).handle(state(0)).run(), (5, vec![(1, 5), (2, 6)]));
}

#[test]
fn a_kept_k_resumed_twice() {
    type R = row![E];
    let g = emits::<R, _>(Rc::new(vec![1, 2]), 0).handle(generate::<i64, Nil, ()>()).run();
    let Gen::Next(1, next) = g else { panic!("not a first value: {g:?}") };
    let n2 = next.clone();
    let twice = next().flat_map(move |a| n2().map(move |b| (a.clone(), b)));
    match twice.run() {
        (Gen::Next(2, _), Gen::Next(2, _)) => {}
        other => panic!("not resumed twice: {other:?}"),
    }
}

#[test]
fn stack_safe_a_generator_of_many_values_drained() {
    type R = row![E];
    fn drain(g: Eff<Nil, Gen<i64, Nil>>, acc: i64) -> Eff<Nil, i64> {
        g.flat_map(move |g| match g {
            Gen::Next(v, next) => drain(next(), acc + v),
            Gen::Done => pure(acc),
        })
    }
    fn many(i: i64) -> Eff<R, ()> {
        if i == 0 { pure(()) } else { emit::<i64, R, _>(1).flat_map(move |_| many(i - 1)) }
    }
    assert_eq!(drain(many(100_000).handle(generate()), 0).run(), 100_000);
}
