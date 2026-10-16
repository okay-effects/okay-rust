//! okay-one's TestStd: the standard effects, each by its handler

use okay::effects::*;
use okay::*;

/// "reader, state, writer, throws: each by its handler, any order of the row"
#[test]
fn reader_state_writer_throws_each_by_its_handler_any_order_of_the_row() {
    type R = row![Reader<i64>, State<i64>, Writer<String>, Throws<String>];
    fn p() -> Eff<R, i64> {
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
        p().handle(reader(5)).handle(state(1)).handle(collect()).handle(either()).run(),
        Ok((vec!["s 6".to_string()], (6, 12)))
    );
    assert_eq!(
        p().handle(either()).handle(reader(500)).handle(state(1)).handle(collect()).run(),
        (vec!["s 501".to_string()], (501, Err("big".to_string())))
    );
}

/// "choose: every path, a state inside each its own" — the state outside the choice here, threaded through the paths
/// as okay-one's test has it
#[test]
fn choose_every_path_the_state_threaded() {
    type R = row![Choose, State<i64>];
    let p: Eff<R, (bool, i64)> = eff! { data;
        b <- flip();
        i <- among(vec![1i64, 2]);
        _ <- modify::<i64, _, _>(move |s| s + i);
        s <- get::<i64, _, _>();
        yield (b, s)
    };
    assert_eq!(p.handle(all()).handle(state(0)).run().1, vec![(true, 1), (true, 3), (false, 4), (false, 6)]);
}
