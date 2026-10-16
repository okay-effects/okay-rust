//! okay-core's KernelBenchmark lanes (src/jmh/scala/okay/one/bench/KernelBenchmark.scala), the same programs, the
//! same n, in okay-rs — so the two kernels are compared on one machine. A lane is measured as JMH's AverageTime:
//! warm-up, then rounds of a second each, µs per run of the program. No dependencies: `std::hint::black_box`.
//!
//! Where a lane cannot be the same: an effect is once in a row here, so `forwarded16`'s sixteen handlers between an
//! operation and its handler are sixteen effects `Skip<0>`…`Skip<15>`.
#![allow(clippy::type_complexity)]

use std::cell::{Cell, RefCell};
use std::hint::black_box;
use std::rc::Rc;
use std::time::{Duration, Instant};

use okay::effects::{all, among, Choose};

use okay::*;

const N: i64 = 1000;

// THE EFFECTS

#[derive(Clone, Debug)]
enum Tick {
    Value(i64, Reply<i64>),
}
impl Effect for Tick {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
struct Skip<const I: usize>(i64, Reply<i64>);
impl<const I: usize> Effect for Skip<I> {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
enum Ask {
    Get(Reply<i64>),
}
impl Effect for Ask {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
enum Tell {
    Put(i64, Reply<()>),
}
impl Effect for Tell {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
enum St {
    Get(Reply<i64>),
    Put(i64, Reply<()>),
}
impl Effect for St {
    type Kind = Plain;
}

#[derive(Clone, Debug)]
enum Throws {
    Raise(String, Reply<Never>),
}
impl Effect for Throws {
    type Kind = Plain;
}

// pure: isEven / isOdd over binds

fn is_even_p(i: i64) -> Eff<Nil, bool> {
    if i == 0 { pure(true) } else { pure::<Nil, ()>(()).flat_map(move |_| is_odd_p(i - 1)) }
}
fn is_odd_p(i: i64) -> Eff<Nil, bool> {
    if i == 0 { pure(false) } else { pure::<Nil, ()>(()).flat_map(move |_| is_even_p(i - 1)) }
}

// the ticking lanes

fn is_even<R: Member<Tick, I> + 'static, I: 'static>(i: i64) -> Eff<R, bool> {
    if i == 0 { pure(true) } else { perform::<R, Tick, I, i64>(|r| Tick::Value(i, r)).flat_map(move |_| is_odd::<R, I>(i - 1)) }
}
fn is_odd<R: Member<Tick, I> + 'static, I: 'static>(i: i64) -> Eff<R, bool> {
    if i == 0 { pure(false) } else { perform::<R, Tick, I, i64>(|r| Tick::Value(i, r)).flat_map(move |_| is_even::<R, I>(i - 1)) }
}

fn ticking<T, A: 'static>() -> Handler<Tick, T, A, A> {
    Handler::answering(|op| match op { Tick::Value(a, r) => r.give(a) }, |a| a)
}
fn ticking_k<T: 'static, A: 'static>() -> Handler<Tick, T, A, A> {
    Handler::deep(pure, |op, k| match op { Tick::Value(a, r) => r.resume(&k, a) })
}
fn skipping<const I: usize, T, A: 'static>() -> Handler<Skip<I>, T, A, A> {
    Handler::answering(|op: Skip<I>| op.1.give(op.0), |a| a)
}

type T1 = row![Tick];
type S1 = row![Skip<0>, Tick];
type S16 = row![Skip<15>, Skip<14>, Skip<13>, Skip<12>, Skip<11>, Skip<10>, Skip<9>, Skip<8>, Skip<7>, Skip<6>,
    Skip<5>, Skip<4>, Skip<3>, Skip<2>, Skip<1>, Skip<0>, Tick];

fn answered() -> bool { is_even::<T1, _>(N).handle(ticking()).run() }
fn general() -> bool { is_even::<T1, _>(N).handle(ticking_k()).run() }
fn forwarded() -> bool { is_even::<S1, _>(N).handle(skipping::<0, _, _>()).handle(ticking()).run() }
fn forwarded_k() -> bool { is_even::<S1, _>(N).handle(skipping::<0, _, _>()).handle(ticking_k()).run() }

macro_rules! skip16 {
    ($p:expr) => {
        $p.handle(skipping::<15, _, _>()).handle(skipping::<14, _, _>()).handle(skipping::<13, _, _>())
            .handle(skipping::<12, _, _>()).handle(skipping::<11, _, _>()).handle(skipping::<10, _, _>())
            .handle(skipping::<9, _, _>()).handle(skipping::<8, _, _>()).handle(skipping::<7, _, _>())
            .handle(skipping::<6, _, _>()).handle(skipping::<5, _, _>()).handle(skipping::<4, _, _>())
            .handle(skipping::<3, _, _>()).handle(skipping::<2, _, _>()).handle(skipping::<1, _, _>())
            .handle(skipping::<0, _, _>())
    };
}
fn forwarded16() -> bool { skip16!(is_even::<S16, _>(N)).handle(ticking()).run() }
fn forwarded_k16() -> bool { skip16!(is_even::<S16, _>(N)).handle(ticking_k()).run() }

// reads

type Rd = row![Ask];
fn asks(i: i64, acc: i64) -> Eff<Rd, i64> {
    if i == 0 { pure(acc) } else { perform::<Rd, Ask, _, i64>(Ask::Get).flat_map(move |v| asks(i - 1, acc + v)) }
}
fn reads() -> i64 {
    asks(N, 0).handle(Handler::answering(|op| match op { Ask::Get(r) => r.give(1) }, |a| a)).run()
}

// writer: the log in the handler, answered in place

type Wr = row![Tell];
fn telling(i: i64) -> Eff<Wr, ()> {
    if i == 0 { pure(()) } else { perform::<Wr, Tell, _, ()>(|r| Tell::Put(i, r)).flat_map(move |_| telling(i - 1)) }
}
fn writer() -> usize {
    let log = Rc::new(RefCell::new(Vec::new()));
    let at_end = log.clone();
    let h: Handler<Tell, Nil, (), (Vec<i64>, ())> = Handler::answering(
        move |op| match op { Tell::Put(i, r) => { log.borrow_mut().push(i); r.give(()) } },
        move |a| (at_end.borrow().clone(), a),
    );
    telling(N).handle(h).run().0.len()
}

// state: in a cell, and a handler's state (each path of a k its own)

type Sc = row![St];
fn counting(i: i64) -> Eff<Sc, i64> {
    if i == 0 {
        perform::<Sc, St, _, i64>(St::Get)
    } else {
        perform::<Sc, St, _, i64>(St::Get)
            .flat_map(|x| perform::<Sc, St, _, ()>(move |r| St::Put(x + 1, r)))
            .flat_map(move |_| counting(i - 1))
    }
}
fn state_cell() -> i64 {
    let s = Rc::new(Cell::new(0i64));
    let h: Handler<St, Nil, i64, i64> = Handler::answering(
        move |op| match op {
            St::Get(r) => r.give(s.get()),
            St::Put(i, r) => { s.set(i); r.give(()) }
        },
        |a| a,
    );
    counting(N).handle(h).run()
}
fn counter<T, A: 'static>(s0: i64) -> Handler<St, T, A, (i64, A)> {
    Handler::stateful(s0, |s: &mut i64, op| match op {
        St::Get(r) => r.give(*s),
        St::Put(i, r) => { *s = i; r.give(()) }
    }, |s, a| (s, a))
}
fn state() -> i64 { counting(N).handle(counter(0)).run().0 }

type Deep3 = row![St, Ask, Tell];
fn counting_deep(i: i64) -> Eff<Deep3, i64> {
    if i == 0 {
        perform::<Deep3, St, _, i64>(St::Get)
    } else {
        perform::<Deep3, St, _, i64>(St::Get)
            .flat_map(|x| perform::<Deep3, St, _, ()>(move |r| St::Put(x + 1, r)))
            .flat_map(move |_| counting_deep(i - 1))
    }
}
fn state_deep() -> i64 {
    let log = Rc::new(RefCell::new(Vec::<i64>::new()));
    let at_end = log.clone();
    let reader_at: Handler<Ask, row![Tell], (i64, i64), (i64, i64)> =
        Handler::answering(|op| match op { Ask::Get(r) => r.give(0) }, |a| a);
    let writer_at: Handler<Tell, Nil, (i64, i64), (Vec<i64>, (i64, i64))> = Handler::answering(
        move |op| match op { Tell::Put(i, r) => { log.borrow_mut().push(i); r.give(()) } },
        move |a| (at_end.borrow().clone(), a),
    );
    counting_deep(N).handle(counter(0)).handle(reader_at).handle(writer_at).run().1.0
}

// maps

fn left_maps() -> i64 {
    let mut p: Eff<Nil, i64> = pure(0);
    for _ in 0..N {
        p = p.map(|x| x + 1);
    }
    p.run()
}
fn right_mapping(i: i64) -> Eff<Nil, i64> {
    if i == 0 { pure(0) } else { defer(move || right_mapping(i - 1)).map(|x| x + 1) }
}
fn right_maps() -> i64 { right_mapping(N).run() }

// throwing: each raise handled where it is, a handler in the program, k not resumed

type Th = row![Throws];
fn catching() -> Handler<Throws, Nil, i64, Result<i64, String>> {
    Handler::aborting(|a| pure(Ok(a)), |op| match op { Throws::Raise(e, _) => pure(Err(e)) })
}
/// the same, a handler dropping `k` by hand
fn catching_k() -> Handler<Throws, Nil, i64, Result<i64, String>> {
    Handler::deep(|a| pure(Ok(a)), |op, _k| match op { Throws::Raise(e, _) => pure(Err(e)) })
}
fn raising(i: i64, acc: i64) -> Eff<Nil, i64> { raising_by(catching, i, acc) }
fn raising_by(h: fn() -> Handler<Throws, Nil, i64, Result<i64, String>>, i: i64, acc: i64) -> Eff<Nil, i64> {
    if i == 0 {
        pure(acc)
    } else {
        perform::<Th, Throws, _, Never>(|r| Throws::Raise("no".to_string(), r))
            .map(|n: Never| match n {})
            .map(|_: i64| 0i64)
            .handle(h())
            .flat_map(move |e: Result<i64, String>| raising_by(h, i - 1, acc + e.unwrap_or_else(|s| s.len() as i64)))
    }
}
fn throwing() -> i64 { raising(N, 0).run() }
fn throwing_k() -> i64 { raising_by(catching_k, N, 0).run() }

// choose: every value of a choice among n

fn choose() -> usize {
    let xs: Vec<i64> = (0..N).collect();
    among::<i64, row![Choose], _>(xs).map(|x| x + 1).handle(all()).run().len()
}

// LEVEL 1, direct style: the same programs, the effects passed down — an operation a call of the handler's method

mod direct {
    use std::hint::black_box;

    use okay::direct::*;

    use super::N;

    pub trait Tick {
        fn tick(&mut self, i: i64) -> i64;
    }
    pub struct Ticking;
    impl Tick for Ticking {
        #[inline]
        fn tick(&mut self, i: i64) -> i64 {
            i
        }
    }

    fn is_even(t: &mut impl Tick, i: i64) -> bool {
        if i == 0 { true } else { black_box(t.tick(i)); is_odd(t, i - 1) }
    }
    fn is_odd(t: &mut impl Tick, i: i64) -> bool {
        if i == 0 { false } else { black_box(t.tick(i)); is_even(t, i - 1) }
    }
    pub fn answered() -> bool { is_even(&mut Ticking, black_box(N)) }

    pub fn reads() -> i64 {
        let mut r = Const(black_box(1i64));
        let mut acc = 0;
        for _ in 0..black_box(N) {
            acc += r.ask();
        }
        acc
    }

    pub fn writer() -> usize {
        let mut w = Log::new();
        for i in (1..=black_box(N)).rev() {
            w.tell(i);
        }
        w.0.len()
    }

    fn counting(st: &mut impl State<i64>, n: i64) -> i64 {
        for _ in 0..n {
            let x = st.get();
            st.put(x + 1);
        }
        st.get()
    }
    pub fn state() -> i64 { counting(&mut Cell(black_box(0i64)), black_box(N)) }

    fn counting_deep(st: &mut impl State<i64>, _r: &mut impl Reader<i64>, _w: &mut impl Writer<i64>, n: i64) -> i64 {
        counting(st, n)
    }
    pub fn state_deep() -> i64 {
        counting_deep(&mut Cell(black_box(0i64)), &mut Const(0i64), &mut Log::new(), black_box(N))
    }

    /// the state behind `dyn`: each operation a call through a table, nothing inlined — the cost of a call
    #[inline(never)]
    fn counting_dyn(st: &mut dyn State<i64>, n: i64) -> i64 {
        for _ in 0..n {
            let x = st.get();
            st.put(x + 1);
        }
        st.get()
    }
    pub fn state_dyn() -> i64 {
        let mut cell = Cell(black_box(0i64));
        // the handler opaque: its type unknown to the loop, each operation a call through its table
        let st: &mut dyn State<i64> = black_box(&mut cell);
        counting_dyn(st, black_box(N))
    }

    /// the reader behind `dyn`, opaque: an ask a call
    #[inline(never)]
    fn asks_dyn(r: &mut dyn Reader<i64>, n: i64) -> i64 {
        let mut acc = 0;
        for _ in 0..n {
            acc += r.ask();
        }
        acc
    }
    pub fn reads_dyn() -> i64 {
        let mut c = Const(black_box(1i64));
        let r: &mut dyn Reader<i64> = black_box(&mut c);
        asks_dyn(r, black_box(N))
    }
}

// ONE API COMPILED BY THE HANDLER'S TYPE (okay::typed): the same programs, eff! { … }

mod typed {
    use std::cell::RefCell;
    use std::hint::black_box;

    use okay::eff;
    use okay::effects::{Reader, State, Writer};
    use okay::typed::*;
    use okay::{Effect, Plain, Reply};

    use super::N;

    #[derive(Clone, Debug)]
    pub enum Tick {
        Value(i64, Reply<i64>),
    }
    impl Effect for Tick {
        type Kind = Plain;
    }
    pub struct Ticking;
    impl Handles<Tick> for Ticking {
        #[inline(always)]
        fn handle(&self, op: Tick) -> Step<'_> {
            match op {
                Tick::Value(i, r) => Step::Now(r.give(i)),
            }
        }
    }

    async fn is_even<C: Has<Tick, I>, I>(cx: &C, n: i64) -> bool {
        // isEven/isOdd as one loop: an async recursion is boxed at each call
        let (mut i, mut even) = (n, true);
        while i > 0 {
            black_box(eff! { perform::<_, Tick, _, i64>(cx, |r| Tick::Value(i, r)) }.await);
            i -= 1;
            even = !even;
        }
        even
    }
    pub fn answered() -> bool { run(is_even(&Cx(Ticking, End), black_box(N))).unwrap_or(false) }

    async fn asks<C: Has<Reader<i64>, I>, I>(cx: &C, n: i64) -> i64 {
        let mut acc = 0;
        for _ in 0..n {
            acc += eff! { ask(cx) }.await;
        }
        acc
    }
    pub fn reads() -> i64 { run(asks(&Cx(AskWith(black_box(1i64)), End), black_box(N))).unwrap_or(0) }

    async fn telling<C: Has<Writer<i64>, I>, I>(cx: &C, n: i64) {
        for i in (1..=n).rev() {
            eff! { tell(cx, i) }.await;
        }
    }
    pub fn writer() -> usize {
        let cx = Cx(Collect::<i64>::new(), End);
        let _ = run(telling(&cx, black_box(N)));
        cx.0.take().len()
    }

    async fn counting<C: Has<State<i64>, I>, I>(cx: &C, n: i64) -> i64 {
        for _ in 0..n {
            eff! {
                x <- get(cx);
                put(cx, x + 1)
            }
            .await;
        }
        get(cx).await
    }
    pub fn state() -> i64 { run(counting(&Cx(StateCell::new(black_box(0i64)), End), black_box(N))).unwrap_or(0) }
    pub fn state_deep() -> i64 {
        let cx = Cx(StateCell::new(black_box(0i64)), Cx(AskWith(0i64), Cx(Collect::<i64>::new(), End)));
        run(counting(&cx, black_box(N))).unwrap_or(0)
    }
    /// the context opaque to the loop (behind `black_box`): each get and put a load and a store of the state, not
    /// folded away
    pub fn state_opaque() -> i64 {
        let cx = Cx(StateCell::new(0i64), End);
        run(counting(black_box(&cx), black_box(N))).unwrap_or(0)
    }
    pub fn reads_opaque() -> i64 {
        let cx = Cx(AskWith(1i64), End);
        run(asks(black_box(&cx), black_box(N))).unwrap_or(0)
    }

    async fn raising<C>(cx: &C, n: i64) -> i64 {
        let mut acc = 0;
        for _ in 0..n {
            let e: Result<i64, String> =
                either(cx, async |cx| raise(cx, "no".to_string()).await).await;
            acc += e.unwrap_or_else(|s| s.len() as i64);
        }
        acc
    }
    pub fn throwing() -> i64 { run(raising(&End, black_box(N))).unwrap_or(0) }

    /// a generator pulled: 1000 values, each the body suspended and resumed once
    pub fn generate_() -> i64 {
        let sum = RefCell::new(0i64);
        let _ = run(generate(&End, |v: i64| *sum.borrow_mut() += v, async |cx| {
            for i in 0..black_box(N) {
                emit(cx, i).await;
            }
        }));
        sum.into_inner()
    }

    /// an interceptor passing each operation on: `answered` through it
    pub struct Passing;
    impl<R> Interceptor<Tick, R, ()> for Passing {
        #[inline(always)]
        fn intercept<H: Handles<Tick>>(&self, op: Tick, pass: Pass<'_, H>, _: &R) -> Step<'_> {
            match op {
                Tick::Value(i, r) => pass.perform(|r2| Tick::Value(i, r2)).map_or(Step::Later, |v| Step::Now(r.give(v))),
            }
        }
    }
    pub fn intercepted() -> bool { run(is_even(&Cx(Audited::new(Ticking, Passing), End), black_box(N))).unwrap_or(false) }

    /// a watch seeing every operation, its effect named, keeping none: `answered` under it
    pub struct Counting(std::cell::Cell<u64>);
    impl Watch for Counting {
        #[inline]
        fn after(&self, _: &str, _: &dyn std::fmt::Debug, _: &dyn std::fmt::Debug) {
            self.0.set(self.0.get() + 1);
        }
    }
    pub fn observed() -> bool {
        let cx = Cx(Ticking, End);
        run(observe(&cx, Counting(std::cell::Cell::new(0)), async |cx| is_even(cx, black_box(N)).await))
            .map(|r| r.unwrap_or(false))
            .unwrap_or(false)
    }

    /// every value of a choice among n: n paths, each the body run again, its choice told it
    pub fn choose() -> usize {
        let xs: Vec<i64> = (0..N).collect();
        run(all(&End, async |cx| among(cx, &xs).await + 1)).map(|v| v.len()).unwrap_or(0)
    }
}

// THE HARNESS: JMH's AverageTime — warm-up, then rounds of a second, µs a run

fn lane(name: &str, f: &dyn Fn() -> u64) {
    let second = Duration::from_secs(1);
    for _ in 0..3 {
        let t = Instant::now();
        while t.elapsed() < second {
            black_box(f());
        }
    }
    let mut rounds = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let mut n = 0u64;
        while t.elapsed() < second {
            black_box(f());
            n += 1;
        }
        rounds.push(t.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
    let mean = rounds.iter().sum::<f64>() / rounds.len() as f64;
    let sd = (rounds.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (rounds.len() - 1) as f64).sqrt();
    println!("{name:14} {mean:10.3} ± {sd:7.3} us/op");
}

fn main() {
    let only: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let lanes: Vec<(&str, Box<dyn Fn() -> u64>)> = vec![
        ("pure", Box::new(|| is_even_p(N).run() as u64)),
        ("answered", Box::new(|| answered() as u64)),
        ("general", Box::new(|| general() as u64)),
        ("forwarded", Box::new(|| forwarded() as u64)),
        ("forwarded16", Box::new(|| forwarded16() as u64)),
        ("forwardedK", Box::new(|| forwarded_k() as u64)),
        ("forwardedK16", Box::new(|| forwarded_k16() as u64)),
        ("reads", Box::new(|| reads() as u64)),
        ("writer", Box::new(|| writer() as u64)),
        ("stateCell", Box::new(|| state_cell() as u64)),
        ("state", Box::new(|| state() as u64)),
        ("stateDeep", Box::new(|| state_deep() as u64)),
        ("leftMaps", Box::new(|| left_maps() as u64)),
        ("rightMaps", Box::new(|| right_maps() as u64)),
        ("throwing", Box::new(|| throwing() as u64)),
        ("throwingK", Box::new(|| throwing_k() as u64)),
        ("choose", Box::new(|| choose() as u64)),
        ("D:answered", Box::new(|| direct::answered() as u64)),
        ("D:reads", Box::new(|| direct::reads() as u64)),
        ("D:writer", Box::new(|| direct::writer() as u64)),
        ("D:state", Box::new(|| direct::state() as u64)),
        ("D:stateDeep", Box::new(|| direct::state_deep() as u64)),
        ("D:stateDyn", Box::new(|| direct::state_dyn() as u64)),
        ("D:readsDyn", Box::new(|| direct::reads_dyn() as u64)),
        ("T:answered", Box::new(|| typed::answered() as u64)),
        ("T:reads", Box::new(|| typed::reads() as u64)),
        ("T:writer", Box::new(|| typed::writer() as u64)),
        ("T:state", Box::new(|| typed::state() as u64)),
        ("T:stateDeep", Box::new(|| typed::state_deep() as u64)),
        ("T:stateOpaque", Box::new(|| typed::state_opaque() as u64)),
        ("T:readsOpaque", Box::new(|| typed::reads_opaque() as u64)),
        ("T:throwing", Box::new(|| typed::throwing() as u64)),
        ("T:generate", Box::new(|| typed::generate_() as u64)),
        ("T:choose", Box::new(|| typed::choose() as u64)),
        ("T:intercepted", Box::new(|| typed::intercepted() as u64)),
        ("T:observed", Box::new(|| typed::observed() as u64)),
    ];
    // each lane's answer, the Scala lanes' own
    assert!(answered() && general() && forwarded() && forwarded16() && forwarded_k() && forwarded_k16());
    assert_eq!((reads(), writer(), state_cell(), state(), state_deep()), (N, N as usize, N, N, N));
    assert_eq!((left_maps(), right_maps(), throwing(), throwing_k(), choose()), (N, N, 2 * N, 2 * N, N as usize));
    assert!(direct::answered());
    assert!(typed::answered() && typed::intercepted() && typed::observed());
    assert_eq!((typed::reads(), typed::writer(), typed::state(), typed::state_deep(), typed::throwing(), typed::generate_()),
        (N, N as usize, N, N, 2 * N, N * (N - 1) / 2));
    assert_eq!((typed::state_opaque(), typed::reads_opaque(), typed::choose()), (N, N, N as usize));
    assert_eq!((direct::reads(), direct::writer(), direct::state(), direct::state_deep(), direct::state_dyn(), direct::reads_dyn()), (N, N as usize, N, N, N, N));
    for (name, f) in &lanes {
        if only.is_empty() || only.iter().any(|o| o == name) {
            lane(name, f.as_ref());
        }
    }
}
