//! okay-core's TestCont: handlers of each form, k twice with a state inside and outside, effects in clauses, handlers
//! in the program, answer type modification around compiled programs, stack safety

use std::rc::Rc;

use okay_machine::cont::shift0;
use okay_machine::effects::*;
use okay_machine::*;

type Ask = Reader<i64>;
type Log = Writer<String>;
type St = State<i64>;

fn asked<R: Member<Ask, I1> + Member<Log, I2> + 'static, I1: 'static, I2: 'static>() -> Eff<R, i64> {
    ask::<i64, R, I1>().flat_map(|n| tell::<String, R, I2>(format!("got {n}")).map(move |_| n + 1))
}

fn ask_logged<T: Member<Log, I> + 'static, I: 'static, A: 'static>(n: i64) -> Handler<Ask, T, A, A> {
    Handler::deep(pure, move |op, k| match op {
        Reader::Ask(r) => tell::<String, T, I>(format!("asked {n}")).flat_map(move |_| r.resume(&k, n)),
    })
}

fn ask_state<T: Member<St, I1> + Member<Log, I2> + 'static, I1: 'static, I2: 'static, A: 'static>() -> Handler<Ask, T, A, A> {
    Handler::deep(pure, |op, k| match op {
        Reader::Ask(r) => get::<i64, T, I1>()
            .flat_map(|s| tell::<String, T, I2>(format!("state {s}")).map(move |_| s))
            .flat_map(move |s| r.resume(&k, s)),
    })
}

#[test]
fn answering_stateful_the_handlers_registered_in_order() {
    type R = row![Ask, Log];
    assert_eq!(asked::<R, _, _>().handle(reader(41)).handle(collect()).run(), (vec!["got 41".to_string()], 42));
}

#[test]
fn a_handler_of_an_effect_from_inside_the_row_log_handled_first_past_ask() {
    type R = row![Ask, Log];
    assert_eq!(asked::<R, _, _>().handle(collect()).handle(reader(41)).run(), (vec!["got 41".to_string()], 42));
}

#[test]
fn aborting_the_rest_dropped() {
    type R = row![Throws<String>, Log];
    fn p() -> Eff<R, i64> {
        tell::<String, R, _>("before".into())
            .flat_map(|_| raise::<String, (), R, _>("no".into()))
            .flat_map(|_| tell::<String, R, _>("after".into()))
            .map(|_| 1)
    }
    assert_eq!(p().handle(either()).handle(collect()).run(), (vec!["before".to_string()], Err("no".to_string())));
    assert_eq!(p().handle(collect()).handle(either()).run(), Err("no".to_string()));
}

fn flips<R: Member<Choose, I1> + Member<St, I2> + 'static, I1: 'static, I2: 'static>() -> Eff<R, i64> {
    flip::<R, I1>().flat_map(|_| get::<i64, R, I2>()).flat_map(|s| put::<i64, R, I2>(s + 1).map(move |_| s))
}

#[test]
fn deep_k_twice_a_state_inside_each_path_its_own() {
    type R = row![Choose, St];
    assert_eq!(flips::<R, _, _>().handle(state(0)).handle(all()).run(), vec![(1, 0), (1, 0)]);
}

#[test]
fn deep_k_twice_a_state_outside_one_for_both_paths() {
    type R = row![Choose, St];
    assert_eq!(flips::<R, _, _>().handle(all()).handle(state(0)).run(), (2, vec![0, 1]));
}

#[test]
fn deep_k_dropped() {
    type R = row![Choose];
    let p = flip::<R, _>().flat_map(|b| if b { pure(1) } else { fail::<i64, R, _>() });
    assert_eq!(p.handle(all()).run(), vec![1]);
}

#[test]
fn an_effect_in_a_clause_the_handler_of_ask_tells() {
    type R = row![Ask, Log];
    assert_eq!(
        asked::<R, _, _>().handle(ask_logged(41)).handle(collect()).run(),
        (vec!["asked 41".to_string(), "got 41".to_string()], 42)
    );
}

#[test]
fn a_program_of_effects_in_a_clause_ask_answered_from_the_state_outside() {
    type R = row![Ask, Log, St];
    assert_eq!(
        asked::<R, _, _>().handle(ask_state()).handle(collect()).handle(state(7)).run(),
        (7, (vec!["state 7".to_string(), "got 7".to_string()], 8))
    );
}

#[test]
fn a_compiled_program_run_twice_its_state_each_run_its_own() {
    type R = row![St];
    let c = || get::<i64, R, _>().flat_map(|s| put::<i64, R, _>(s + 1).map(move |_| s)).handle(state(0));
    assert_eq!(c().run(), (1, 0));
    assert_eq!(c().run(), (1, 0));
}

#[test]
fn atm_around_compiled_programs_a_shift0_turning_string_into_a_function_its_k_resumed_twice() {
    type S = Rc<dyn Fn(i64) -> String>;
    type Q = row![Ask, Log, Control<Nil>];
    let body: Cont<Reset, Eff<Nil, String>, Eff<Nil, S>, String> =
        shift0::<Reset, Nil, _, i64, Eff<Nil, String>, Eff<Nil, S>>(|k| {
            pure(Rc::new(move |x| format!("{} / {}", k(x).run(), k(x + 1).run())) as S)
        })
        .flat_map(|x| {
            asked::<Q, _, _>().handle(reader(x)).handle(collect()).cont::<Eff<Nil, String>>().map(|v| format!("{v:?}"))
        });
    let p: Eff<Nil, S> = dollar(body, |s| pure(format!("got {s}")));
    assert_eq!(p.run()(41), r#"got (["got 41"], 42) / got (["got 42"], 43)"#);
}

#[test]
fn a_handler_in_the_program_its_parameter_a_value_of_the_program() {
    type R = row![Ask, Log];
    type Inner = row![Reader<i32>, Ask, Log];
    let p = ask::<i64, R, _>().flat_map(|n| {
        ask::<i32, Inner, _>().map(|m| m as i64 * 10).handle(reader((n + 1) as i32)).map(move |m| n + m)
    });
    assert_eq!(p.handle(reader(1)).handle(collect::<String, _, _>()).run(), (Vec::<String>::new(), 21));
}

#[test]
fn a_handler_in_the_program_a_raise_caught_the_program_going_on() {
    type R = row![Log];
    type T = Cons<Throws<String>, R>;
    fn step(i: i64) -> Eff<T, i64> {
        if i % 2 == 0 { raise::<String, i64, T, _>(format!("odd {i}")) } else { pure(i) }
    }
    let p = step(1).handle(either()).flat_map(|a| {
        let a = a.clone();
        step(2).handle(either()).flat_map(move |b| {
            let sum = a.clone().unwrap_or(0) + b.clone().unwrap_or(0);
            tell::<String, R, _>(format!("{a:?} {b:?}")).map(move |_| sum)
        })
    });
    assert_eq!(p.handle(collect()).run(), (vec![r#"Ok(1) Err("odd 2")"#.to_string()], 1));
}

#[test]
fn a_handler_in_the_program_its_effect_not_seen_outside_its_state_each_time_its_own() {
    type R = row![Log];
    type SR = Cons<St, R>;
    fn go(i: i64) -> Eff<SR, ()> {
        if i == 0 { pure(()) } else { get::<i64, SR, _>().flat_map(|s| put::<i64, SR, _>(s + 1)).flat_map(move |_| go(i - 1)) }
    }
    fn counted(n: i64) -> Eff<R, i64> {
        go(n).handle(state(0)).map(|(s, _)| s)
    }
    let p = counted(3).flat_map(|a| counted(4).flat_map(move |b| tell::<String, R, _>(format!("{a} {b}")).map(move |_| a + b)));
    assert_eq!(p.handle(collect()).run(), (vec!["3 4".to_string()], 7));
}

#[test]
fn a_handler_in_the_program_under_a_deep_one_outside_each_path_of_k_with_its_own() {
    type R = row![Choose];
    type SR = Cons<St, R>;
    let p = flip::<R, _>().flat_map(|b| {
        get::<i64, SR, _>()
            .flat_map(|s| flip::<SR, _>().flat_map(move |c| put::<i64, SR, _>(s + if c { 1 } else { 2 })))
            .handle(state(if b { 10 } else { 20 }))
            .map(|(s, _)| s)
    });
    assert_eq!(p.handle(all()).run(), vec![11, 12, 21, 22]);
}

const N: i64 = 100_000;

#[test]
fn stack_safe_a_loop_of_handlers_in_the_program_each_its_operations_forwarded_out() {
    type R = row![Log];
    type AR = Cons<Ask, R>;
    fn lp(n: i64, acc: i64) -> Eff<R, i64> {
        if n == 0 { pure(acc) } else { asked::<AR, _, _>().handle(reader(n % 2)).flat_map(move |v| lp(n - 1, acc + v)) }
    }
    assert_eq!(lp(N, 0).handle(collect()).run().1, N + N / 2);
}

#[test]
fn stack_safe_a_loop_of_operations_each_forwarded_past_deeps_dollar_and_a_state() {
    type R = row![Choose, St, Ask];
    fn lp(n: i64, acc: i64) -> Eff<R, i64> {
        if n == 0 { pure(acc) } else { ask::<i64, R, _>().flat_map(move |x| lp(n - 1, acc + x)) }
    }
    assert_eq!(lp(N, 0).handle(all()).handle(state(0)).handle(reader(1)).run(), (0, vec![N]));
}

#[test]
fn stack_safe_binds_nested_to_the_left_maps() {
    type R = row![Ask];
    let p = (0..N).fold(pure::<R, i64>(0), |p, _| p.flat_map(|a| ask::<i64, R, _>().map(move |x| a + x)));
    assert_eq!(p.handle(reader(1)).run(), N);
}

#[test]
fn stack_safe_a_recursion_under_maps_deferred() {
    fn down(n: i64) -> Eff<Nil, i64> {
        if n == 0 { pure(0) } else { defer(move || down(n - 1)).map(|x| x + 1) }
    }
    assert_eq!(down(N).run(), N);
}
