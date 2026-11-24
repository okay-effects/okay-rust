//! okay-one's TestStd: the standard effects, each by its handler

use okay_machine::effects::*;
use okay_machine::*;

/// "reader, state, writer, throws: each by its handler, any order of the row"
#[test]
fn reader_state_writer_throws_each_by_its_handler_any_order_of_the_row() {
    type R = row![Reader<i64>, State<i64>, Writer<String>, Throws<String>];
    fn p() -> Prog<i64, R> {
        eff! { data;
            n <- ask::<i64, _, _>();
            _ <- modify::<i64, _, _>(move |s| s + n);
            s <- get::<i64, _, _>();
            _ <- tell::<String, _, _>(format!("s {s}"));
            _ <- if s > 100 { raise::<String, (), _, _>("big".to_string()) } else { pure(()) };
            yield s * 2
        }
    }
    assert_eq!(
        p().handle(Reader::run(5)).handle(State::run(1)).handle(Writer::run()).handle(Throws::run()).run(),
        Ok((vec!["s 6".to_string()], (6, 12)))
    );
    assert_eq!(
        p().handle(Throws::run()).handle(Reader::run(500)).handle(State::run(1)).handle(Writer::run()).run(),
        (vec!["s 501".to_string()], (501, Err("big".to_string())))
    );
}

/// "choose: every path, a state inside each its own" — the state outside the choice here, threaded through the paths
/// as okay-one's test has it
#[test]
fn choose_every_path_the_state_threaded() {
    type R = row![Choose, State<i64>];
    let p: Prog<(bool, i64), R> = eff! { data;
        b <- flip();
        i <- among(vec![1i64, 2]);
        _ <- modify::<i64, _, _>(move |s| s + i);
        s <- get::<i64, _, _>();
        yield (b, s)
    };
    assert_eq!(
        p.handle(Choose::all()).handle(State::run(0)).run().1,
        vec![(true, 1), (true, 3), (false, 4), (false, 6)]
    );
}

/// "choose first: the paths tried in order, the first value kept, the rest of the paths never run" — the rest `k`
/// resumed again for each path tried (a real `k`, no replay)
#[test]
fn choose_first_the_first_path_with_a_value() {
    type R = row![Choose, Writer<String>];
    let p: Prog<i64, R> = eff! { data;
        i <- among(vec![1i64, 2, 3, 4]);
        _ <- tell::<String, _, _>(format!("try {i}"));
        x <- if i % 3 == 0 { pure(i * 10) } else { fail() };
        yield x
    };
    assert_eq!(
        p.handle(Choose::first()).handle(Writer::run()).run(),
        (vec!["try 1".to_string(), "try 2".to_string(), "try 3".to_string()], Some(30))
    );
    let none: Prog<i64, row![Choose]> = among(vec![1i64, 2]).flat_map(|_| fail());
    assert_eq!(none.handle(Choose::first()).run(), None);
}
