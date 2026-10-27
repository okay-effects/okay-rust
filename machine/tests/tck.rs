//! okay-kernel's TCK digest: its thousand random programs — handlers of logs, states, failures and choices nested in any
//! order, each effect twice (two identities), their operations, binds — run here; the digest of their outcomes must
//! be the reference machine's, the legacy kernel's and okay-java's: the same programs, the same seed
//! (okay-kernel tck/src/main/scala/okay/kernel/tck/Tck.scala). A program's values are Java's `int`s (wrapping); an
//! effect's two identities two types here (`State<i64>` and `State<i32>`: an effect's identity is its type). The
//! TCK's state is its `Handlers.state`: a CELL, made when the handler is installed, one for every resumption of a rest
//! taken past it (not the machine's register, each resumption from the state as it was) — so here.

use std::rc::Rc;

use okay_machine::effects::*;
use okay_machine::*;

/// java.util.Random, under scala.util.Random
struct JRandom(u64);

impl JRandom {
    fn new(seed: i64) -> JRandom {
        JRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & -bound == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}

enum T {
    Pure(i32),
    Tell(i32, i32),
    Get(i32),
    Put(i32, i32),
    Among(Vec<i32>),
    Raise(i32, i32),
    Seq(Rc<T>, Rc<T>),
    Add(Rc<T>, Rc<T>),
    WriterH(i32, Rc<T>),
    StateH(i32, i32, Rc<T>),
    ThrowsH(i32, Rc<T>),
    ChooseH(Rc<T>),
}

fn random(r: &mut JRandom, depth: i32) -> Rc<T> {
    Rc::new(if depth <= 0 || r.next_int(4) == 0 {
        match r.next_int(6) {
            0 => T::Pure(r.next_int(10)),
            1 => { let e = r.next_int(2); T::Tell(e, r.next_int(10)) }
            2 => T::Get(r.next_int(2)),
            3 => { let e = r.next_int(2); T::Put(e, r.next_int(10)) }
            4 => { let n = 1 + r.next_int(2); T::Among((0..n).map(|_| r.next_int(10)).collect()) }
            _ => { let e = r.next_int(2); T::Raise(e, r.next_int(10)) }
        }
    } else {
        match r.next_int(6) {
            0 => { let a = random(r, depth - 1); T::Seq(a, random(r, depth - 1)) }
            1 => { let a = random(r, depth - 1); T::Add(a, random(r, depth - 1)) }
            2 => { let e = r.next_int(2); T::WriterH(e, random(r, depth - 1)) }
            3 => { let e = r.next_int(2); let s0 = r.next_int(10); T::StateH(e, s0, random(r, depth - 1)) }
            4 => { let e = r.next_int(2); T::ThrowsH(e, random(r, depth - 1)) }
            _ => T::ChooseH(random(r, depth - 1)),
        }
    })
}

/// one row with every effect, each twice; a handler in a program installed by `intercept` (the row kept)
type R = row![Choose, Throws<i64>, Throws<i32>, State<i64>, State<i32>, Writer<i64>, Writer<i32>];

fn writer_value(log: Vec<i32>, v: i32) -> i32 {
    v.wrapping_mul(100).wrapping_add(log.iter().fold(0i32, |s, w| s.wrapping_add(*w)))
}

fn build(t: &Rc<T>) -> Eff<R, i32> {
    match &**t {
        T::Pure(n) => pure(*n),
        T::Tell(0, n) => { let n = *n; tell::<i64, R, _>(n as i64).map(move |_| n) }
        T::Tell(_, n) => { let n = *n; tell::<i32, R, _>(n).map(move |_| n) }
        T::Get(0) => get::<i64, R, _>().map(|s| s as i32),
        T::Get(_) => get::<i32, R, _>(),
        T::Put(0, n) => { let n = *n; put::<i64, R, _>(n as i64).map(move |_| n) }
        T::Put(_, n) => { let n = *n; put::<i32, R, _>(n).map(move |_| n) }
        T::Among(ns) => among::<i32, R, _>(ns.clone()),
        T::Raise(0, n) => raise::<i64, i32, R, _>(*n as i64),
        T::Raise(_, n) => raise::<i32, i32, R, _>(*n),
        T::Seq(a, b) => { let b = b.clone(); build(a).flat_map(move |_| build(&b)) }
        T::Add(a, b) => { let b = b.clone(); build(a).flat_map(move |i| build(&b).map(move |j| i.wrapping_add(j))) }
        T::WriterH(0, body) => build(body)
            .intercept(collect::<i64, R, i32>())
            .map(|(log, v)| writer_value(log.into_iter().map(|w| w as i32).collect(), v)),
        T::WriterH(_, body) => build(body).intercept(collect::<i32, R, i32>()).map(|(log, v)| writer_value(log, v)),
        T::StateH(0, s0, body) => build(body).intercept(cell::<i64>(*s0 as i64)).map(|(s, v)| v.wrapping_mul(10).wrapping_add(s as i32)),
        T::StateH(_, s0, body) => build(body).intercept(cell::<i32>(*s0)).map(|(s, v)| v.wrapping_mul(10).wrapping_add(s)),
        T::ThrowsH(0, body) => build(body).intercept(either::<i64, R, i32>()).map(|r| r.unwrap_or_else(|e| -1 - e as i32)),
        T::ThrowsH(_, body) => build(body).intercept(either::<i32, R, i32>()).map(|r| r.unwrap_or_else(|e| -1 - e)),
        T::ChooseH(body) => build(body).intercept(all::<R, i32>()).map(|vs| vs.iter().fold(0i32, |s, v| s.wrapping_add(*v))),
    }
}

/// the TCK's state: a cell, answered in place — every resumption sees what the last left
fn cell<S: Copy + std::fmt::Debug + 'static>(s0: S) -> Handler<State<S>, R, i32, (S, i32)> {
    let s = Rc::new(std::cell::Cell::new(s0));
    let at_end = s.clone();
    Handler::answering(move |op: State<S>| match op {
        State::Get(r) => r.give(s.get()),
        State::Put(n, r) => { s.set(n); r.give(()) }
    }, move |v| (at_end.get(), v))
}

/// a program of the row closed: its effects' handlers outside, never reached (every operation is the wrapper's)
fn closed(p: Eff<R, i32>) -> i32 {
    p.handle(collect::<i32, _, i32>()).map(|(_, v)| v)
        .handle(collect::<i64, _, i32>()).map(|(_, v)| v)
        .handle(state::<i32, _, i32>(0)).map(|(_, v)| v)
        .handle(state::<i64, _, i32>(0)).map(|(_, v)| v)
        .handle(either::<i32, _, i32>()).map(|r| r.unwrap_or(i32::MIN))
        .handle(either::<i64, _, i32>()).map(|r| r.unwrap_or(i32::MIN))
        .handle(all::<_, i32>()).map(|vs| vs[0])
        .run()
}

/// okay-kernel's Tck.Digest
const DIGEST: i64 = -8623788747993822161;

#[test]
fn a_thousand_random_programs_the_reference_machines_digest() {
    let mut r = JRandom::new(20261010);
    let mut d: i64 = 0;
    for _ in 0..1000 {
        // a handler of each kind around: ChooseH(ThrowsH(0, ThrowsH(1, StateH(0, 0, StateH(1, 0, WriterH(0, WriterH(1, gen)))))))
        let mut t = random(&mut r, 6);
        t = Rc::new(T::WriterH(1, t));
        t = Rc::new(T::WriterH(0, t));
        t = Rc::new(T::StateH(1, 0, t));
        t = Rc::new(T::StateH(0, 0, t));
        t = Rc::new(T::ThrowsH(1, t));
        t = Rc::new(T::ThrowsH(0, t));
        t = Rc::new(T::ChooseH(t));
        let v = closed(build(&t));
        d = d.wrapping_mul(31).wrapping_add(v as i64);
    }
    assert_eq!(d, DIGEST);
}

/// the generator as Java's: `new java.util.Random(20261010L)`, `nextInt` of these bounds, as Java printed them
#[test]
fn java_random_as_java() {
    let mut r = JRandom::new(20261010);
    let b = [4, 6, 10, 2, 6, 10];
    let got: Vec<i32> = (0..18).map(|i| r.next_int(b[i % 6])).collect();
    assert_eq!(got, vec![2, 4, 9, 1, 0, 8, 0, 4, 4, 0, 2, 7, 0, 3, 6, 1, 1, 1]);
}
