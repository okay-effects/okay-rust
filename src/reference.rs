//! THE MACHINE AGAINST ONE REFERENCE — okay-core's TestReference: random programs run by the machine, with all it does
//! for speed, and by their meaning — normal forms, each node by its rule, nothing clever: the same answers required.
//! Rows are types here, so a random program's handlers are installed by `intercept` (the row kept, the effect
//! handled), over one row of four effects. The programs of control (shift0, `$`) wait for stage 2.

use std::rc::Rc;

use crate::eff::{Frame, Kind, Node, Op, Val};
use crate::effects::*;
use crate::machine::{K0, Level, StatefulRt};
use crate::{row, Eff, Handler, Step};

// THE MEANING

/// a program's normal form: its value, or its first operation not handled in it, with its rest
enum Norm {
    Done(Val),
    Op(Op, Rc<dyn Fn(Val) -> Norm>),
}

fn shared(mut f: Frame) -> Rc<Frame> {
    f.make_shared();
    Rc::new(f)
}

fn norm(n: Node) -> Norm {
    match n.0 {
        Kind::Return(v) => Norm::Done(v),
        Kind::Bind(m, f) => {
            let f = shared(f);
            bind(norm(*m), Rc::new(move |x| norm(f.copy_shared().call(x))))
        }
        Kind::Then(v, f) => norm(f.call(v)),
        Kind::Inject(op, f) => {
            let f = shared(f);
            Norm::Op(op, Rc::new(move |y| norm(f.copy_shared().call(y))))
        }
        Kind::Perform(op) => Norm::Op(op, Rc::new(Norm::Done)),
        Kind::Handle(body, level) => handled(level, norm(*body)),
        Kind::Resume(k, v) => norm(k.call_fn(v)),
        Kind::Defer(p) => norm(p()),
        _ => panic!("a node no program written here makes"),
    }
}

fn bind(n: Norm, f: Rc<dyn Fn(Val) -> Norm>) -> Norm {
    match n {
        Norm::Done(x) => f(x),
        Norm::Op(op, k) => Norm::Op(op, Rc::new(move |y| bind(k(y), f.clone()))),
    }
}

/// a normal form a program again
fn reify(n: Norm) -> Node {
    match n {
        Norm::Done(v) => Node(Kind::Return(v)),
        Norm::Op(op, k) => Node(Kind::Inject(op, Frame::new(move |x| reify(k(x))))),
    }
}

/// a handler the textbook's: its body's normal form, an operation of its own given to it with the rest under it again,
/// one of another's going out past it
fn handled(level: Level, n: Norm) -> Norm {
    if let Level::Stateful(e, rt, s) = level {
        return stated(e, rt, s, n);
    }
    match n {
        Norm::Done(v) => match &level {
            Level::Answering(_, rt) => Norm::Done((rt.ret)(v)),
            Level::Deep(_, rt) => norm((rt.ret)(v)),
            Level::Aborting(_, rt) => norm((rt.ret)(v)),
            Level::Guard(_) | Level::Mask(_) => Norm::Done(v),
            _ => panic!("a level no program written here makes"),
        },
        Norm::Op(op, k) => {
            let mine = match &level {
                Level::Answering(e, _) | Level::Deep(e, _) | Level::Aborting(e, _) => *e == op.effect,
                _ => false,
            };
            if !mine {
                return Norm::Op(op, Rc::new(move |x| handled(level.clone(), k(x))));
            }
            match &level {
                Level::Answering(_, rt) => handled(level.clone(), k((rt.value)(op.value))),
                Level::Aborting(_, rt) => norm((rt.abort)(op.value)),
                Level::Deep(_, rt) => {
                    let again = level.clone();
                    norm((rt.apply)(op.value, K0::of_fn(move |x| reify(handled(again.clone(), k(x))))))
                }
                _ => unreachable!(),
            }
        }
    }
}

/// a state's handler: the state as it is, moved by each of its operations
fn stated(e: std::any::TypeId, rt: Rc<StatefulRt>, s: Rc<dyn std::any::Any>, n: Norm) -> Norm {
    match n {
        Norm::Done(v) => Norm::Done((rt.ret)(s, v)),
        Norm::Op(op, k) if op.effect == e => {
            let (s2, v) = (rt.step)(s, op.value);
            stated(e, rt, s2, k(v))
        }
        Norm::Op(op, k) => Norm::Op(op, Rc::new(move |x| stated(e, rt.clone(), s.clone(), k(x)))),
    }
}

/// a program with no effect left: its value
fn value<A: 'static>(p: Eff<crate::Nil, A>) -> A {
    match norm(p.node) {
        Norm::Done(v) => v.take::<A>(),
        Norm::Op(..) => panic!("an operation not handled"),
    }
}

// THE PROGRAMS: a random tree first (every random choice made there, none in the closures), then a program of it — made
// again each time a rest needs it (a program as data is consumed by its run)

type R4 = row![Reader<i64>, State<i64>, Choose, Throws<String>];

enum P {
    Pure(i64),
    Ask(Rc<P>),
    Get(Rc<P>),
    Put(i64, Rc<P>),
    Flip(Rc<P>, Rc<P>),
    Raise(i64, Rc<P>),
    Seq(i64, Rc<P>, Rc<P>),
    Defer(i64, Rc<P>),
    Reader(i64, Rc<P>),
    State(i64, Rc<P>),
    StateSnd(i64, Rc<P>),
    Choose(Rc<P>),
    Attempt(Rc<P>),
    AskSt(i64, bool, Rc<P>),
    AskFromSt(i64, Rc<P>),
    Keep(i64, Rc<P>),
}

/// xorshift64*: the same programs on every machine
struct Rand(u64);
impl Rand {
    fn next(&mut self, n: u64) -> i64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as i64 % n as i64
    }
}

fn random(r: &mut Rand, depth: u32) -> Rc<P> {
    let c = r.next(5);
    if depth == 0 {
        return Rc::new(P::Pure(c));
    }
    let d = depth - 1;
    Rc::new(match r.next(17) {
        0 => P::Pure(c),
        1 => P::Ask(random(r, d)),
        2 => P::Get(random(r, d)),
        3 => P::Put(c, random(r, d)),
        4 => P::Flip(random(r, d), random(r, d)),
        5 => P::Raise(c, random(r, d)),
        6 => P::Seq(c, random(r, d), random(r, d)),
        7 => P::Defer(c, random(r, d)),
        8 => P::Reader(c, random(r, d)),
        9 => P::State(c, random(r, d)),
        10 => P::Choose(random(r, d)),
        11 => P::Attempt(random(r, d)),
        12 => P::AskSt(c, r.next(2) == 0, random(r, d)),
        13 => P::Reader(c + 1, Rc::new(P::StateSnd(c, random(r, d)))),
        14 => P::StateSnd(c, Rc::new(P::Choose(random(r, d)))),
        15 => P::AskFromSt(c, random(r, d)),
        _ => P::Keep(c, random(r, d)),
    })
}

/// an answer holding a kept rest
#[derive(Clone)]
enum Kept {
    Value(i64),
    Rest(Rc<dyn Fn(i64) -> Eff<R4, Kept>>),
}

fn drained(c: i64, kept: Kept) -> Eff<R4, i64> {
    match kept {
        Kept::Value(v) => crate::pure(v),
        Kept::Rest(k) => put::<i64, R4, _>(c).flat_map(move |_| k(c)).flat_map(move |next| drained(c + 1, next)),
    }
}

fn build(p: &Rc<P>) -> Eff<R4, i64> {
    let b = |p: &Rc<P>| -> Rc<P> { p.clone() };
    match &**p {
        P::Pure(c) => crate::pure(*c),
        P::Ask(rest) => { let rest = b(rest); ask::<i64, R4, _>().flat_map(move |n| build(&rest).map(move |x| x + n)) }
        P::Get(rest) => { let rest = b(rest); get::<i64, R4, _>().flat_map(move |s| build(&rest).map(move |x| x * 2 + s)) }
        P::Put(c, rest) => { let rest = b(rest); put::<i64, R4, _>(*c).flat_map(move |_| build(&rest)) }
        P::Flip(l, rr) => {
            let (l, rr) = (b(l), b(rr));
            flip::<R4, _>().flat_map(move |t| if t { build(&l) } else { build(&rr).map(|x| x + 1) })
        }
        P::Raise(c, rest) => {
            let rest = b(rest);
            if c % 2 == 0 {
                raise::<String, (), R4, _>("e".repeat(*c as usize)).flat_map(move |_| build(&rest))
            } else {
                build(&rest)
            }
        }
        P::Seq(c, l, rr) => {
            let (c, rr) = (*c, b(rr));
            build(l).flat_map(move |x| if x % 2 == 0 { build(&rr).map(move |y| y + x) } else { crate::pure(x - c) })
        }
        P::Defer(c, inner) => { let (c, inner) = (*c, b(inner)); crate::defer(move || build(&inner)).map(move |x| x + c) }
        P::Reader(c, body) => build(body).intercept(reader(*c)),
        P::State(c, body) => build(body).intercept(state(*c)).map(|(s, a)| s + a),
        P::StateSnd(c, body) => build(body).intercept(state(*c)).map(|(_, a)| a),
        P::Choose(body) => build(body).intercept(all()).map(|xs: Vec<i64>| xs.iter().sum()),
        P::Attempt(body) => build(body).intercept(either()).map(|e: Result<i64, String>| e.unwrap_or_else(|s| s.len() as i64)),
        P::AskSt(c, twice, body) => {
            let (n, twice) = (*c, *twice);
            build(body).intercept(Handler::deep(|a: i64| crate::pure(a * 2), move |op: Reader<i64>, k| match op {
                Reader::Ask(r) => get::<i64, R4, _>().flat_map(move |s| {
                    if twice {
                        let k2 = k.clone();
                        r.resume(&k, s + n).flat_map(move |a| r.resume(&k2, a % 7).map(move |x| x + a))
                    } else {
                        r.resume(&k, s + n)
                    }
                }),
            }))
        }
        P::AskFromSt(c, body) => {
            let n = *c;
            build(body).intercept(Handler::translating(move |op: Reader<i64>| match op {
                Reader::Ask(r) => get::<i64, R4, _>()
                    .flat_map(|s| put::<i64, R4, _>(s + 1).map(move |_| s))
                    .map(move |s| r.give(s + n)),
            }))
        }
        P::Keep(c, body) => {
            let c = *c;
            build(body)
                .intercept(Handler::deep(|a: i64| crate::pure(Kept::Value(a)), |op: Reader<i64>, k| match op {
                    Reader::Ask(r) => crate::pure(Kept::Rest(Rc::new(move |x| r.resume(&k, x)))),
                }))
                .flat_map(move |kept| drained(c, kept))
        }
    }
}

type Answers = (i64, Vec<Result<i64, String>>);

fn closed(p: &Rc<P>) -> Eff<crate::Nil, Answers> {
    build(p).handle(either()).handle(all()).handle(state(1)).handle(reader(2))
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// the answers of 1000 random programs: changed only knowingly
const DIGEST: u64 = 16067519562359843393;

#[test]
fn effects_random_programs_handlers_of_each_kind_in_them_and_around_them_the_machine_as_the_meaning() {
    let mut r = Rand(17);
    for i in 1..=500 {
        let p = random(&mut r, 4);
        assert_eq!(closed(&p).run(), value(closed(&p)), "program {i}");
    }
}

#[test]
fn effects_random_programs_the_choice_outermost_each_path_its_own_state() {
    let mut r = Rand(19);
    for i in 1..=500 {
        let p = random(&mut r, 4);
        let q = || build(&p).handle(reader(3)).handle(either()).handle(state(0)).handle(all());
        assert_eq!(q().run(), value(q()), "program {i}");
    }
}

#[test]
fn step_random_programs_stepped_their_asks_answered_by_the_driver_as_the_meaning_with_a_reader() {
    let mut r = Rand(37);
    for i in 1..=300 {
        let p = random(&mut r, 4);
        let open = || build(&p).handle(either()).handle(all()).handle(state(1));
        let mut s = open().step();
        let driven = loop {
            s = match s {
                Step::Done(a) => break a,
                Step::Yield(y) => match y.op::<Reader<i64>>() {
                    Some(Reader::Ask(r)) => y.resume(r.give(2)),
                    None => panic!("not an ask"),
                },
                Step::Failed(e) => panic!("{}", e.message()),
                Step::Cancelled => panic!("cancelled"),
            }
        };
        assert_eq!(driven, value(open().handle(reader(2))), "program {i}");
    }
}

#[test]
fn the_semantics_pinned_the_answers_of_random_programs_their_digest() {
    let mut r = Rand(29);
    let answers: Vec<Answers> = (0..1000).map(|_| closed(&random(&mut r, 4)).run()).collect();
    let digest = fnv(&format!("{answers:?}"));
    if DIGEST == 0 {
        panic!("pin the digest: {digest}");
    }
    assert_eq!(digest, DIGEST);
}

#[test]
fn the_random_programs_are_not_trivial() {
    fn size(p: &P) -> usize {
        1 + match p {
            P::Pure(_) => 0,
            P::Ask(x) | P::Get(x) | P::Put(_, x) | P::Raise(_, x) | P::Defer(_, x) | P::Reader(_, x) | P::State(_, x)
            | P::StateSnd(_, x) | P::Choose(x) | P::Attempt(x) | P::AskSt(_, _, x) | P::AskFromSt(_, x) | P::Keep(_, x) => size(x),
            P::Flip(a, b) | P::Seq(_, a, b) => size(a) + size(b),
        }
    }
    let mut r = Rand(29);
    let ps: Vec<Rc<P>> = (0..1000).map(|_| random(&mut r, 4)).collect();
    let answers: std::collections::HashSet<String> = ps.iter().map(|p| format!("{:?}", closed(p).run())).collect();
    let mean = ps.iter().map(|p| size(p)).sum::<usize>() as f64 / ps.len() as f64;
    eprintln!("{} distinct answers of 1000, mean size {mean:.1}", answers.len());
    assert!(answers.len() > 100 && mean > 4.0);
}

#[test]
fn deeper_random_programs_the_machine_as_the_meaning() {
    let mut r = Rand(41);
    for i in 1..=300 {
        let p = random(&mut r, 6);
        assert_eq!(closed(&p).run(), value(closed(&p)), "program {i}");
    }
}
