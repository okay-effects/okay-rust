//! THE REST TAKEN MANY TIMES, BY REPLAY (okay::typed): `all` and `first` run the body again for each path, its past
//! told it again — the choices made, the answers from further out — so a state further out is threaded through the
//! paths in order, as the machine's `all` under it; the machine's suite's programs, the same values

use std::cell::RefCell;

use okay::effects::{Choose, Reader};
use okay::typed::*;
use okay::{eff, Here, Loop};

/// "choose: every path, the state threaded" — as tests/std.rs on the machine
#[test]
fn choose_every_path_the_state_threaded() {
    let cx = Cx(StateCell::new(0i64), End);
    let r = run(all(&cx, async |cx| {
        eff! {
            b <- flip(cx);
            i <- among(cx, &[1i64, 2]);
            _ <- modify(cx, |s| s + i);
            s <- get(cx);
            yield (b, s)
        }
        .await
    }));
    assert_eq!(r, Ok(vec![(true, 1), (true, 3), (false, 4), (false, 6)]));
}

/// "a state read before the choice: each path resumed with what it read, not reading again" — the machine's k holds
/// the value read; a replay, its answer told again
#[test]
fn a_state_read_before_the_choice_read_once() {
    let cx = Cx(StateCell::new(0i64), End);
    let r = run(all(&cx, async |cx| eff! {
        s <- get(cx);
        b <- flip(cx);
        _ <- put(cx, s + 1);
        yield (b, s)
    }.await));
    assert_eq!(r, Ok(vec![(true, 0), (false, 0)]));
    assert_eq!(cx.0.take(), 1);
}

/// "a state inside the choice: each path its own"
#[test]
fn a_state_inside_each_path_its_own() {
    let r = run(all(&End, async |cx| {
        let st = Cx(StateCell::new(0i64), cx);
        eff! {
            i <- among(&st, &[1i64, 2, 3]);
            _ <- modify(&st, |s| s + i);
            s <- get(&st);
            yield s
        }
        .await
    }));
    assert_eq!(r, Ok(vec![1, 2, 3]));
}

/// "a writer further out: what was told before a choice told once"
#[test]
fn a_writer_further_out_told_once_before_the_choice() {
    let cx = Cx(Collect::<String>::new(), End);
    let r = run(all(&cx, async |cx| {
        eff! {
            tell(cx, "start".to_string());
            x <- among(cx, &['a', 'b']);
            tell(cx, format!("path {x}"));
            yield x
        }
        .await
    }));
    assert_eq!(r, Ok(vec!['a', 'b']));
    assert_eq!(cx.0.take(), vec!["start", "path a", "path b"]);
}

/// "fail: the path dropped" — the pythagorean triples under 20
#[test]
fn fail_drops_the_path() {
    let r = run(all(&End, async |cx| {
        let ns: Vec<i64> = (1..20).collect();
        let a = among(cx, &ns).await;
        let b = among(cx, &ns[a as usize - 1..]).await;
        let c = among(cx, &ns[b as usize - 1..]).await;
        if a * a + b * b != c * c {
            fail::<(), _, _>(cx).await;
        }
        (a, b, c)
    }));
    assert_eq!(r, Ok(vec![(3, 4, 5), (5, 12, 13), (6, 8, 10), (8, 15, 17), (9, 12, 15)]));
}

/// "first: the first path not failing, the rest not run"
#[test]
fn first_the_first_path_not_failing() {
    let runs = RefCell::new(0);
    let r = run(first(&End, async |cx| {
        *runs.borrow_mut() += 1;
        let x = among(cx, &[1i64, 2, 3, 4, 5]).await;
        if x * x < 10 {
            fail::<(), _, _>(cx).await;
        }
        x
    }));
    assert_eq!(r, Ok(Some(4)));
    assert_eq!(*runs.borrow(), 4);
    assert_eq!(run(first(&End, async |cx| fail::<i64, _, _>(cx).await)), Ok(None));
}

/// "all inside all: the inner paths for each outer one"
#[test]
fn all_inside_all() {
    let r = run(all(&End, async |cx| {
        let x = among(cx, &[1i64, 2]).await;
        let ys = all(cx, async |cx| {
            let y: i64 = among::<_, _, Here>(cx, &[10i64, 20]).await;
            x * y
        })
        .await;
        (x, ys)
    }));
    assert_eq!(r, Ok(vec![(1, vec![10, 20]), (2, vec![20, 40])]));
}

/// "eight queens: 92 ways"
#[test]
fn eight_queens() {
    okay::needs! {
        async fn queens(cx: [Choose], n: usize) -> Vec<usize> {
            let cols: Vec<usize> = (0..n).collect();
            let mut placed: Vec<usize> = Vec::new();
            for row in 0..n {
                let c = among(cx, &cols).await;
                if placed.iter().enumerate().any(|(r, &q)| q == c || row - r == c.abs_diff(q)) {
                    fail::<(), _, _>(cx).await;
                }
                placed.push(c);
            }
            placed
        }
    }
    let r = run(all(&End, async |cx| queens(cx, 8).await)).unwrap();
    assert_eq!(r.len(), 92);
    assert_eq!(r[0], vec![0, 4, 7, 5, 2, 6, 1, 3]);
}

/// "a reader and a state further out, asked once per path's past: the reader asked once"
#[test]
fn answers_further_out_asked_once() {
    struct Counted(RefCell<i64>);
    impl Handles<Reader<i64>> for Counted {
        fn handle(&self, op: Reader<i64>) -> Step<'_> {
            match op {
                Reader::Ask(r) => {
                    *self.0.borrow_mut() += 1;
                    Step::Now(r.give(10))
                }
            }
        }
    }
    let cx = Cx(Counted(RefCell::new(0)), End);
    let r = run(all(&cx, async |cx| {
        eff! {
            d <- ask(cx);
            x <- among(cx, &[1i64, 2, 3]);
            yield d * x
        }
        .await
    }));
    assert_eq!(r, Ok(vec![10, 20, 30]));
    assert_eq!(*cx.0.0.borrow(), 1);
}

// RECURSION

/// "isEven/isOdd, a million calls, mutual: constant stack" — as tests/recursion.rs on the machine
#[test]
fn mutual_recursion_a_million_calls() {
    enum Call {
        Even(i64),
        Odd(i64),
    }
    let cx = Cx(StateCell::new(0i64), End);
    let r = run(tail_rec(Call::Even(1_000_000), async |call| {
        modify(&cx, |s| s + 1).await;
        match call {
            Call::Even(0) => Loop::Done(true),
            Call::Odd(0) => Loop::Done(false),
            Call::Even(i) => Loop::Next(Call::Odd(i - 1)),
            Call::Odd(i) => Loop::Next(Call::Even(i - 1)),
        }
    }));
    assert_eq!(r, Ok(true));
    assert_eq!(cx.0.take(), 1_000_001);
}

// EVERY OPERATION OF EVERY EFFECT

struct Trail(RefCell<Vec<String>>);
impl Watch for Trail {
    fn after(&self, effect: &str, op: &dyn std::fmt::Debug, answer: &dyn std::fmt::Debug) {
        self.0.borrow_mut().push(format!("{effect} {op:?} → {answer:?}"));
    }
}

/// "an observer sees every operation of every effect, with its answer"
#[test]
fn an_observer_sees_every_operation() {
    let cx = Cx(AskWith(5i64), Cx(StateCell::new(1i64), Cx(Collect::<String>::new(), End)));
    let trail = Trail(RefCell::new(vec![]));
    let r = run(observe(&cx, &trail, async |cx| {
        eff! {
            n <- ask(cx);
            _ <- modify(cx, |s| s + n);
            s <- get(cx);
            _ <- tell(cx, format!("s {s}"));
            yield s * 2
        }
        .await
    }));
    assert_eq!(r, Ok(Ok(12)));
    assert_eq!(
        trail.0.into_inner(),
        vec![
            "Reader<i64> Ask(_) → 5",
            "State<i64> Get(_) → 1",
            "State<i64> Put(6, _) → ()",
            "State<i64> Get(_) → 6",
            "Writer<String> Tell(\"s 6\", _) → ()",
        ]
    );
}

/// "a read-only observer: a write refused, the body dropped there"
#[test]
fn a_read_only_observer_refuses_writes() {
    struct ReadOnly;
    impl Watch for ReadOnly {
        fn before(&self, _: &str, op: &dyn std::fmt::Debug) -> Result<(), String> {
            let op = format!("{op:?}");
            if op.starts_with("Put") { Err(format!("read-only: {op}")) } else { Ok(()) }
        }
    }
    let cx = Cx(StateCell::new(1i64), End);
    let r = run(observe(&cx, ReadOnly, async |cx| {
        let s = get(cx).await;
        put(cx, s + 1).await;
        s
    }));
    assert_eq!(r, Ok(Err("read-only: Put(2, _)".to_string())));
    assert_eq!(cx.0.take(), 1);
}

/// "an observer outside the choice sees an operation further out once; inside it, each time the path runs"
#[test]
fn an_observer_and_the_replay() {
    let cx = Cx(AskWith(1i64), End);
    let outside = Trail(RefCell::new(vec![]));
    let _ = run(observe(&cx, &outside, async |cx| {
        all(cx, async |cx| {
            let d: i64 = ask(cx).await;
            let x: i64 = among(cx, &[1i64, 2]).await;
            d + x
        })
        .await
    }));
    assert_eq!(outside.0.borrow().len(), 1, "{:?}", outside.0.borrow());
    let inside = Trail(RefCell::new(vec![]));
    let _ = run(all(&cx, async |cx| {
        observe(cx, &inside, async |cx| {
            let d: i64 = ask(cx).await;
            let x: i64 = among(cx, &[1i64, 2]).await;
            d + x
        })
        .await
    }));
    assert_eq!(inside.0.borrow().len(), 4, "{:?}", inside.0.borrow());
}
