//! okay-core's TestStream on `generate`: a body made a lazy generator, `k` kept as the next step

use std::cell::Cell;
use std::rc::Rc;

use okay_machine::effects::*;
use okay_machine::*;

type E = row![Emit<i64>];

fn stream(body: Eff<E, ()>) -> Eff<Nil, Gen<i64, Nil>> {
    body.handle(generate())
}

fn take(g: Eff<Nil, Gen<i64, Nil>>, n: usize) -> Vec<i64> {
    let mut out = Vec::new();
    let mut g = g;
    while out.len() < n {
        match g.run() {
            Gen::Done => break,
            Gen::Next(w, rest) => {
                out.push(w);
                g = rest();
            }
        }
    }
    out
}

fn from<R: Member<Emit<i64>, Ix> + 'static, Ix: 'static>(i: i64) -> Eff<R, ()> {
    emit::<i64, R, Ix>(i).flat_map(move |_| defer(move || from::<R, Ix>(i + 1)))
}

#[test]
fn the_elements_in_order_an_infinite_body_pulled_as_far_as_asked() {
    assert_eq!(take(stream(from(0)), 5), vec![0, 1, 2, 3, 4]);
}

#[test]
fn a_body_that_ends_the_list_ends() {
    assert_eq!(take(stream(emit::<i64, E, _>(1).flat_map(|_| emit::<i64, E, _>(2))), 10), vec![1, 2]);
}

#[test]
fn lazy_the_body_runs_as_far_as_pulled_a_rest_pulled_again_run_again() {
    let ran = Rc::new(Cell::new(0));
    fn counted(i: i64, ran: Rc<Cell<i32>>) -> Eff<E, ()> {
        defer(move || {
            ran.set(ran.get() + 1);
            let r2 = ran.clone();
            emit::<i64, E, _>(i).flat_map(move |_| counted(i + 1, r2.clone()))
        })
    }
    let Gen::Next(w, rest) = stream(counted(0, ran.clone())).run() else { panic!("empty") };
    assert_eq!(ran.get(), 1);
    let mut got = vec![w];
    got.extend(take(rest(), 2));
    assert_eq!((got, ran.get()), (vec![0, 1, 2], 3));
    let mut again = vec![w];
    again.extend(take(rest(), 2));
    assert_eq!((again, ran.get()), (vec![0, 1, 2], 5));
}

#[test]
fn other_effects_handled_in_the_body_a_reader() {
    type R = row![Reader<i64>, Emit<i64>];
    let reading = ask::<i64, R, _>()
        .flat_map(emit::<i64, R, _>)
        .flat_map(|_| ask::<i64, R, _>().flat_map(|n| emit::<i64, R, _>(n * 2)));
    assert_eq!(take(stream(reading.handle(reader(5))), 10), vec![5, 10]);
}

fn counter<R: Member<State<i64>, I1> + Member<Emit<i64>, I2> + 'static, I1: 'static, I2: 'static>() -> Eff<R, ()> {
    get::<i64, R, I1>()
        .flat_map(|n| put::<i64, R, I1>(n + 1).flat_map(move |_| emit::<i64, R, I2>(n)))
        .flat_map(|_| defer(counter::<R, I1, I2>))
}

#[test]
fn a_cell_handled_in_the_body_lives_in_the_stream_one_every_pull_shares() {
    type R = row![State<i64>, Emit<i64>];
    let src = || {
        let s = Rc::new(Cell::new(0i64));
        stream(defer(move || {
            let s = s.clone();
            counter::<R, _, _>().handle(Handler::answering(move |op: State<i64>| match op {
                State::Get(r) => r.give(s.get()),
                State::Put(n, r) => { s.set(n); r.give(()) }
            }, |a| a))
        }))
    };
    let Gen::Next(w, rest) = src().run() else { panic!("empty") };
    let mut a = vec![w];
    a.extend(take(rest(), 3));
    assert_eq!(a, vec![0, 1, 2, 3]);
    // the head pulled once; the rest goes on from the cell
    let mut b = vec![w];
    b.extend(take(rest(), 3));
    assert_eq!(b, vec![0, 4, 5, 6]);
    // a source started again: a run of its own, a cell of its own
    assert_eq!(take(src(), 4), vec![0, 1, 2, 3]);
}

#[test]
fn a_state_handled_in_the_body_each_rest_its_state_as_it_was_taken() {
    type R = row![State<i64>, Emit<i64>];
    let Gen::Next(w, rest) = stream(counter::<R, _, _>().handle(state(0)).map(|_| ())).run() else { panic!("empty") };
    let mut a = vec![w];
    a.extend(take(rest(), 3));
    assert_eq!(a, vec![0, 1, 2, 3]);
    let mut b = vec![w];
    b.extend(take(rest(), 3));
    assert_eq!(b, vec![0, 1, 2, 3]);
}

#[test]
fn many_elements_pulled_in_constant_stack_collect_keeps_them_all() {
    assert_eq!(*take(stream(from(0)), 100_000).last().unwrap(), 99_999);
    fn up_to(i: i64, n: i64) -> Eff<E, ()> {
        if i == n { pure(()) } else { emit::<i64, E, _>(i).flat_map(move |_| up_to(i + 1, n)) }
    }
    assert_eq!(up_to(0, 100_000).handle(collect_emitted()).run().0.len(), 100_000);
}
