//! ONE API COMPILED BY THE HANDLER'S TYPE (okay::typed): programs written once with `eff! { … }`, run under
//! handlers answering in place, dropping the rest, taking it once; a sensitive effect only under its audit

use std::cell::RefCell;

use okay::eff;
use okay::effects::{Reader, State, Throws, Writer};
use okay::typed::*;

okay::needs! {
    async fn counted(cx: [Reader<i64>, State<i64>, Writer<String>], n: i64) -> i64 {
        eff! {
            d <- ask(cx);
            _ <- modify(cx, |s| s + d * n);
            s <- get(cx);
            _ <- tell(cx, format!("s {s}"));
            yield s
        }
        .await
    }
}

/// a writer of another type: nothing kept — the same program compiled for it
struct Silent;
impl Handles<Writer<String>> for Silent {
    fn handle(&self, op: Writer<String>) -> Step<'_> {
        match op {
            Writer::Tell(_, r) => Step::Now(r.give(())),
        }
    }
}

/// "one program, two contexts: compiled for each"
#[test]
fn one_program_two_contexts_compiled_for_each() {
    let kept = Cx(AskWith(2i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
    assert_eq!(run(counted(&kept, 10)), Ok(21));
    assert_eq!(kept.1.1.0.take(), vec!["s 21".to_string()]);
    let silent = Cx(AskWith(5i64), Cx(StateCell::new(0i64), Cx(Silent, End)));
    assert_eq!(run(counted(&silent, 2)), Ok(10));
}

/// "either: the error raised, the rest of the body dropped"
#[test]
fn either_the_error_raised_the_rest_of_the_body_dropped() {
    let cx = Cx(StateCell::new(0i64), End);
    let after = RefCell::new(false);
    let r: Result<Result<i64, String>, Stuck> = run(either(&cx, async |cx| {
        put(cx, 1).await;
        let _: () = raise(cx, "no".to_string()).await;
        *after.borrow_mut() = true;
        7
    }));
    assert_eq!(r, Ok(Err("no".to_string())));
    assert!(!*after.borrow(), "the rest after the raise ran");
    assert_eq!(cx.0.take(), 1, "the state before the raise kept");
}

/// "a raise in a loop, each under its own either: the loop goes on"
#[test]
fn a_raise_in_a_loop_each_under_its_own_either() {
    let cx = End;
    let total = run(async {
        let mut acc = 0;
        for i in 0..10 {
            let r: Result<i64, String> = either(&cx, async |cx| {
                if i % 2 == 0 { raise(cx, "even".to_string()).await } else { i }
            })
            .await;
            acc += r.unwrap_or(100);
        }
        acc
    });
    assert_eq!(total, Ok(5 * 100 + (1 + 3 + 5 + 7 + 9)));
}

/// "a generator: each value given as it comes, the body suspended at each emit and resumed once — a state in it"
#[test]
fn a_generator_each_value_as_it_comes_the_body_resumed_once() {
    let cx = Cx(StateCell::new(0i64), End);
    let seen = RefCell::new(Vec::new());
    let done = run(generate(&cx, |v: i64| seen.borrow_mut().push(v), async |cx| {
        for i in 1..=5 {
            modify(cx, |s| s + i).await;
            let s = get(cx).await;
            emit(cx, s).await;
        }
        "done"
    }));
    assert_eq!(done, Ok("done"));
    assert_eq!(*seen.borrow(), vec![1, 3, 6, 10, 15]);
}

/// "a raise reaches the nearest either: the types find it"
#[test]
fn throws_found_by_type() {
    fn _typed<C: Has<Throws<String>, I>, I>(_: &C) {}
    let cx = End;
    let _: Result<Result<i32, String>, Stuck> = run(either(&cx, async |cx| {
        _typed(cx);
        0
    }));
}

okay::needs! {
    /// a value asked twice, of any type: the function's own type parameter beside the context's
    async fn twice<V: Clone + std::fmt::Debug>(cx: [Reader<V>]) -> (V, V) {
        let a = ask(cx).await;
        let b = ask(cx).await;
        (a, b)
    }
}

/// "needs!: the effects listed, the context's type and indices made"
#[test]
fn needs_the_effects_listed() {
    assert_eq!(run(twice(&Cx(AskWith("x".to_string()), End))), Ok(("x".to_string(), "x".to_string())));
    assert_eq!(run(twice(&Cx(StateCell::new(0u8), Cx(AskWith(7i64), End)))), Ok((7, 7)));
}
