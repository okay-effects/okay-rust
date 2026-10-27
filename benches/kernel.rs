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
    /// rules set at run time: each a call through a pointer, passing the operation on
    fn policies(n: usize) -> Policies<Tick> {
        let ps = Policies::new();
        for _ in 0..n {
            ps.add(|op, next| next(op));
        }
        ps
    }
    pub fn dyn0() -> bool { run(is_even(&Cx(Audited::new(Ticking, policies(0)), End), black_box(N))).unwrap_or(false) }
    pub fn dyn1() -> bool { run(is_even(&Cx(Audited::new(Ticking, policies(1)), End), black_box(N))).unwrap_or(false) }
    pub fn dyn4() -> bool { run(is_even(&Cx(Audited::new(Ticking, policies(4)), End), black_box(N))).unwrap_or(false) }
    /// a context built at run time: the operation boxed, one layer passed through a pointer
    pub fn dyn_cx() -> bool {
        let layer = okay::dynamic::With::new(okay::dynamic::root(), Ticking);
        run(is_even(&layer.cx(), black_box(N))).unwrap_or(false)
    }
    /// the same, the handler four layers out
    pub fn dyn_cx4() -> bool {
        use okay::dynamic::{root, With};
        let t = With::new(root(), Ticking);
        let a = With::new(t.cx(), AskWith(1i64));
        let b = With::new(a.cx(), StateCell::new(0i64));
        let c = With::new(b.cx(), Collect::<String>::new());
        run(is_even(&c.cx(), black_box(N))).unwrap_or(false)
    }
    okay::effect! { pub enum Hop: Plain, wire { Hop(i: i64) -> i64 = hop; } }
    /// a program handed out: each of its 1000 operations stops it, the host answers it typed (`resume`)
    pub fn hosted() -> i64 {
        let mut c = okay::host::start(|h| async move {
            let cx = Cx(h.outside::<Hop>(), End);
            let mut s = 0;
            for i in 0..N {
                s += hop(&cx, i).await;
            }
            s
        });
        let mut st = c.step();
        loop {
            match st {
                okay::host::Step::Yielded(a) => {
                    let Hop::Hop(i, _) = a.get::<Hop>().unwrap();
                    st = c.resume(*i);
                }
                okay::host::Step::Done(s) => return s,
                _ => return -1,
            }
        }
    }
    /// the same, answered as data through a route (`drive`): each operation a record out, a plain value in
    pub fn driven() -> i64 {
        let c = okay::host::start(|h| async move {
            let cx = Cx(h.outside::<Hop>(), End);
            let mut s = 0;
            for i in 0..N {
                s += hop(&cx, i).await;
            }
            s
        });
        let routes = okay::host::Routes::new().on::<Hop>(|op| match op {
            Hop::Hop(i, r) => Ok(r.give(i)),
        });
        okay::host::drive(c, routes.answerer()).unwrap_or(-1)
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

#[cfg(all(feature = "count-allocs", feature = "mimalloc"))]
compile_error!("count-allocs counts the system allocator: not with mimalloc");

/// `--features count-allocs`: the system allocator under a counter; each lane prints its allocations per run
#[cfg(feature = "count-allocs")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static BYTES: AtomicU64 = AtomicU64::new(0);

    pub struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(l.size() as u64, Relaxed);
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(n as u64, Relaxed);
            unsafe { System.realloc(p, l, n) }
        }
    }
    #[global_allocator]
    static GLOBAL: Counting = Counting;

    /// allocations and bytes of one run, after a warm one
    pub fn per_run(f: &dyn Fn() -> u64) -> (u64, u64) {
        std::hint::black_box(f());
        let (a, b) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
        std::hint::black_box(f());
        (ALLOCS.load(Relaxed) - a, BYTES.load(Relaxed) - b)
    }
}

fn lane(name: &str, f: &dyn Fn() -> u64) {
    #[cfg(feature = "count-allocs")]
    {
        let (a, b) = counting::per_run(f);
        println!("{name:14} {a:10} allocs {b:12} bytes per run");
    }
    #[cfg(not(feature = "count-allocs"))]
    timed(name, f);
}

fn timed(name: &str, f: &dyn Fn() -> u64) {
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
        ("T:dyn0", Box::new(|| typed::dyn0() as u64)),
        ("T:dyn1", Box::new(|| typed::dyn1() as u64)),
        ("T:dyn4", Box::new(|| typed::dyn4() as u64)),
        ("T:dynCx", Box::new(|| typed::dyn_cx() as u64)),
        ("H:hosted", Box::new(|| typed::hosted() as u64)),
        ("H:driven", Box::new(|| typed::driven() as u64)),
        ("T:dynCx4", Box::new(|| typed::dyn_cx4() as u64)),
    ];
    // each lane's answer, the Scala lanes' own
    assert!(answered() && general() && forwarded() && forwarded16() && forwarded_k() && forwarded_k16());
    assert_eq!((reads(), writer(), state_cell(), state(), state_deep()), (N, N as usize, N, N, N));
    assert_eq!((left_maps(), right_maps(), throwing(), throwing_k(), choose()), (N, N, 2 * N, 2 * N, N as usize));
    assert!(typed::answered() && typed::intercepted() && typed::observed());
    assert_eq!((typed::hosted(), typed::driven()), (N * (N - 1) / 2, N * (N - 1) / 2));
    assert!(typed::dyn0() && typed::dyn1() && typed::dyn4() && typed::dyn_cx() && typed::dyn_cx4());
    assert_eq!((typed::reads(), typed::writer(), typed::state(), typed::state_deep(), typed::throwing(), typed::generate_()),
        (N, N as usize, N, N, 2 * N, N * (N - 1) / 2));
    assert_eq!((typed::state_opaque(), typed::reads_opaque(), typed::choose()), (N, N, N as usize));
    for (name, f) in &lanes {
        if only.is_empty() || only.iter().any(|o| o == name) {
            lane(name, f.as_ref());
        }
    }
}
