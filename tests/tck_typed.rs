//! okay-kernel's TCK digest on the ONE API: the same thousand random programs (tests/tck.rs), interpreted by an
//! `async` function over a DYNAMIC context (`okay::dynamic`) — each handler of a program a layer put on at run time,
//! nested in the order the program says. The digest must be the machine's, the reference machine's, okay-java's. The
//! TCK's state is a cell every resumption of a rest sees (`Shared`: one object for every path of the replay, its
//! past told again, not done again); its writer the machine's, each resumption from the log as it was (`With`).

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use okay::dynamic::{Dyn, Shared, With, all, either, root};
use okay::effects::State;
use okay::typed::*;

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
            1 => {
                let e = r.next_int(2);
                T::Tell(e, r.next_int(10))
            }
            2 => T::Get(r.next_int(2)),
            3 => {
                let e = r.next_int(2);
                T::Put(e, r.next_int(10))
            }
            4 => {
                let n = 1 + r.next_int(2);
                T::Among((0..n).map(|_| r.next_int(10)).collect())
            }
            _ => {
                let e = r.next_int(2);
                T::Raise(e, r.next_int(10))
            }
        }
    } else {
        match r.next_int(6) {
            0 => {
                let a = random(r, depth - 1);
                T::Seq(a, random(r, depth - 1))
            }
            1 => {
                let a = random(r, depth - 1);
                T::Add(a, random(r, depth - 1))
            }
            2 => {
                let e = r.next_int(2);
                T::WriterH(e, random(r, depth - 1))
            }
            3 => {
                let e = r.next_int(2);
                let s0 = r.next_int(10);
                T::StateH(e, s0, random(r, depth - 1))
            }
            4 => {
                let e = r.next_int(2);
                T::ThrowsH(e, random(r, depth - 1))
            }
            _ => T::ChooseH(random(r, depth - 1)),
        }
    })
}

fn writer_value(log: Vec<i32>, v: i32) -> i32 {
    v.wrapping_mul(100).wrapping_add(log.iter().fold(0i32, |s, w| s.wrapping_add(*w)))
}

/// the TCK's state: a cell answered in place
struct CellOf<S>(std::cell::Cell<S>);
impl<S: Copy + std::fmt::Debug + 'static> Handles<State<S>> for CellOf<S> {
    fn handle(&self, op: State<S>) -> Step<'_> {
        match op {
            State::Get(r) => Step::Now(r.give(self.0.get())),
            State::Put(n, r) => {
                self.0.set(n);
                Step::Now(r.give(()))
            }
        }
    }
}

fn eval<'a>(cx: Dyn<'a>, t: &'a Rc<T>) -> Pin<Box<dyn Future<Output = i32> + 'a>> {
    Box::pin(async move {
        match &**t {
            T::Pure(n) => *n,
            T::Tell(0, n) => {
                tell::<i64, _, _>(&cx, *n as i64).await;
                *n
            }
            T::Tell(_, n) => {
                tell::<i32, _, _>(&cx, *n).await;
                *n
            }
            T::Get(0) => get::<i64, _, _>(&cx).await as i32,
            T::Get(_) => get::<i32, _, _>(&cx).await,
            T::Put(0, n) => {
                put::<i64, _, _>(&cx, *n as i64).await;
                *n
            }
            T::Put(_, n) => {
                put::<i32, _, _>(&cx, *n).await;
                *n
            }
            T::Among(ns) => among(&cx, ns).await,
            T::Raise(0, n) => raise::<i64, i32, _, _>(&cx, *n as i64).await,
            T::Raise(_, n) => raise::<i32, i32, _, _>(&cx, *n).await,
            T::Seq(a, b) => {
                eval(cx, a).await;
                eval(cx, b).await
            }
            T::Add(a, b) => {
                let i = eval(cx, a).await;
                i.wrapping_add(eval(cx, b).await)
            }
            T::WriterH(0, body) => {
                let w = With::new(cx, Collect::<i64>::new());
                let v = eval(w.cx(), body).await;
                writer_value(w.handler().take().into_iter().map(|x| x as i32).collect(), v)
            }
            T::WriterH(_, body) => {
                let w = With::new(cx, Collect::<i32>::new());
                let v = eval(w.cx(), body).await;
                writer_value(w.handler().take(), v)
            }
            T::StateH(0, s0, body) => {
                let s = Shared::new(cx, || CellOf(std::cell::Cell::new(*s0 as i64)));
                let v = eval(s.cx(), body).await;
                v.wrapping_mul(10).wrapping_add(s.handler().0.get() as i32)
            }
            T::StateH(_, s0, body) => {
                let s = Shared::new(cx, || CellOf(std::cell::Cell::new(*s0)));
                let v = eval(s.cx(), body).await;
                v.wrapping_mul(10).wrapping_add(s.handler().0.get())
            }
            T::ThrowsH(0, body) => {
                either::<i64, i32>(cx, async |cx| eval(cx, body).await).await.unwrap_or_else(|e| -1 - e as i32)
            }
            T::ThrowsH(_, body) => {
                either::<i32, i32>(cx, async |cx| eval(cx, body).await).await.unwrap_or_else(|e| -1 - e)
            }
            T::ChooseH(body) => {
                let vs = all(cx, async |cx| eval(cx, body).await).await;
                vs.iter().fold(0i32, |s, v| s.wrapping_add(*v))
            }
        }
    })
}

/// okay-kernel's Tck.Digest
const DIGEST: i64 = -8623788747993822161;

#[test]
fn a_thousand_random_programs_the_reference_machines_digest() {
    let mut r = JRandom::new(20261010);
    let mut d: i64 = 0;
    for _ in 0..1000 {
        let mut t = random(&mut r, 6);
        t = Rc::new(T::WriterH(1, t));
        t = Rc::new(T::WriterH(0, t));
        t = Rc::new(T::StateH(1, 0, t));
        t = Rc::new(T::StateH(0, 0, t));
        t = Rc::new(T::ThrowsH(1, t));
        t = Rc::new(T::ThrowsH(0, t));
        t = Rc::new(T::ChooseH(t));
        let v = run(eval(root(), &t)).expect("every operation answered in place");
        d = d.wrapping_mul(31).wrapping_add(v as i64);
    }
    assert_eq!(d, DIGEST);
}
