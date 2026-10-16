//! okay-one's TestAcceptance (okay-core's docs/one.md §4): the programs the kernel passes

use std::cell::Cell;
use std::rc::Rc;

use okay::effects::*;
use okay::*;

type S = State<i64>;

fn counted<R: Member<Choose, I1> + Member<S, I2> + 'static, I1: 'static, I2: 'static>() -> Eff<R, i64> {
    eff! { data;
        x <- among::<i64, R, I1>(vec![1, 2, 3]);
        s <- get::<i64, R, I2>();
        _ <- put::<i64, R, I2>(s + x);
        t <- get::<i64, R, I2>();
        yield t
    }
}

/// a cell: one for the run, the paths share it — answering in place, its state its own
fn cell<T, A: 'static>(s0: i64) -> Handler<S, T, A, (i64, A)> {
    let c = Rc::new(Cell::new(s0));
    let at_end = c.clone();
    Handler::answering(
        move |op| match op {
            State::Get(r) => r.give(c.get()),
            State::Put(s, r) => {
                c.set(s);
                r.give(())
            }
        },
        move |a| (at_end.get(), a),
    )
}

/// "D. state outside the choice: threaded through the paths"
#[test]
fn d_state_outside_the_choice_threaded_through_the_paths() {
    assert_eq!(counted::<row![Choose, S], _, _>().handle(all()).handle(state(0)).run(), (6, vec![1, 3, 6]));
}

/// "D. state inside the choice: each path its own"
#[test]
fn d_state_inside_the_choice_each_path_its_own() {
    assert_eq!(
        counted::<row![S, Choose], _, _>().handle(state(0)).handle(all()).run(),
        vec![(1, 1), (2, 2), (3, 3)]
    );
}

/// "D. a cell inside the choice: one for the run"
#[test]
fn d_a_cell_inside_the_choice_one_for_the_run() {
    assert_eq!(
        counted::<row![S, Choose], _, _>().handle(cell(0)).handle(all()).run(),
        vec![(1, 1), (3, 3), (6, 6)]
    );
}

#[derive(Clone, Debug)]
enum Two {
    Pick(Reply<i64>),
}
impl Effect for Two {
    type Kind = Plain;
}

/// "A/D. the rests made first, resumed later and in reverse: each its state as it was when captured"
#[test]
fn ad_the_rests_resumed_in_reverse_each_its_state_as_it_was() {
    fn twice<T: 'static>() -> Handler<Two, T, i64, Vec<i64>> {
        Handler::deep(|a| pure(vec![a]), |op, k| match op {
            Two::Pick(r) => {
                // the second first, then the first: each from the state as it was when the rest was taken
                r.resume(&k, 2).flat_map(move |two: Vec<i64>| {
                    r.resume(&k, 1).map(move |one: Vec<i64>| one.iter().chain(two.iter()).copied().collect())
                })
            }
        })
    }
    fn picked<R: Member<Two, I1> + Member<S, I2> + 'static, I1: 'static, I2: 'static>() -> Eff<R, i64> {
        eff! { data;
            s <- get::<i64, R, I2>();
            x <- perform::<R, Two, I1, i64>(Two::Pick);
            _ <- put::<i64, R, I2>(s + x);
            t <- get::<i64, R, I2>();
            yield t
        }
    }
    assert_eq!(
        picked::<row![S, Two], _, _>().handle(state(10)).map(|(_, t)| t).handle(twice()).run(),
        vec![11, 12]
    );
}

/// "F. flatMap and filter over 100 000 empties, in constant stack"
#[test]
fn f_flat_map_and_filter_over_100_000_empties_in_constant_stack() {
    type R = row![Choose];
    let p: Eff<R, i64> = among((1..=100_000).collect())
        .flat_map(|x: i64| if x % 1000 == 0 { pure(x) } else { fail() });
    assert_eq!(p.handle(all()).run().len(), 100);
}

/// "F. binds nested to the left, 100 000, in constant stack"
#[test]
fn f_binds_nested_to_the_left_100_000_in_constant_stack() {
    let mut p: Eff<Nil, i64> = pure(0);
    for _ in 0..100_000 {
        p = p.flat_map(|x| pure(x + 1));
    }
    assert_eq!(p.run(), 100_000);
}
