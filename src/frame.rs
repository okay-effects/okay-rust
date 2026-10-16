//! A FRAME, the rest of a bind: a closure from the value to the next node. Small closures — up to 24 bytes, aligned
//! to 8 at most, the most a bind captures (an index, a counter, a `K`, an `Rc`) — kept in place, no allocation; larger
//! ones shared behind an `Rc`. A frame taken into a rest `k` is moved behind an `Rc` there (`share`), once: a `k`
//! resumed many times copies only the `Rc` — a closure is never cloned, so it need not be `Clone`.

use std::mem::{align_of, size_of, MaybeUninit};
use std::rc::Rc;

use crate::eff::{Node, Val};

type Bits = MaybeUninit<[u64; 3]>;

pub(crate) enum Frame {
    Inline { bits: Bits, vt: &'static Vt },
    Shared(Rc<dyn Fn(Val) -> Node>),
}

/// a frame shared: in a rest `k`
pub(crate) type Shared = Rc<dyn Fn(Val) -> Node>;

/// the closure's own functions, one table for each closure type
pub(crate) struct Vt {
    call: unsafe fn(*const u8, Val) -> Node,
    share: unsafe fn(*const u8) -> Shared,
    drop: unsafe fn(*mut u8),
}

struct VtOf<F>(F);

impl<F: Fn(Val) -> Node + 'static> VtOf<F> {
    const VT: Vt = Vt { call: call::<F>, share: share::<F>, drop: drop::<F> };
}

// SAFETY (the three): `p` points at an `F` written by `Frame::new::<F>` (the table is `F`'s, kept beside it)
unsafe fn call<F: Fn(Val) -> Node>(p: *const u8, v: Val) -> Node {
    unsafe { (*(p as *const F))(v) }
}
unsafe fn share<F: Fn(Val) -> Node + 'static>(p: *const u8) -> Shared {
    // the closure moved out: its bits are not dropped after (`Frame::share` forgets the frame)
    Rc::new(unsafe { std::ptr::read(p as *const F) })
}
unsafe fn drop<F>(p: *mut u8) {
    unsafe { std::ptr::drop_in_place(p as *mut F) }
}

impl Frame {
    #[inline(always)]
    pub(crate) fn new<F: Fn(Val) -> Node + 'static>(f: F) -> Frame {
        if size_of::<F>() <= size_of::<Bits>() && align_of::<F>() <= align_of::<Bits>() {
            let mut bits = Bits::uninit();
            // SAFETY: `F` fits (checked above); the bits own it from here, dropped by its table
            unsafe { std::ptr::write(bits.as_mut_ptr() as *mut F, f) };
            Frame::Inline { bits, vt: &VtOf::<F>::VT }
        } else {
            Frame::Shared(Rc::new(f))
        }
    }

    /// the rest given the value; the frame dropped after
    #[inline(always)]
    pub(crate) fn call(self, v: Val) -> Node {
        match &self {
            // SAFETY: the bits hold the closure of this table
            Frame::Inline { bits, vt } => unsafe { (vt.call)(bits.as_ptr() as *const u8, v) },
            Frame::Shared(f) => f(v),
        }
    }
}

impl Frame {
    /// the frame made shared where it is, for a rest `k`: an inline closure moved behind an `Rc`, once
    #[inline]
    pub(crate) fn make_shared(&mut self) {
        if let Frame::Inline { .. } = self {
            // SAFETY: `self` read out and written back whole, nothing between them that can unwind but an allocation
            // (an abort); the frame read is consumed by `share`, never dropped twice
            unsafe {
                let me = std::ptr::read(self);
                std::ptr::write(self, Frame::Shared(me.share()));
            }
        }
    }

    /// a copy of a shared frame — every frame of a rest taken is (`make_shared`)
    #[inline]
    pub(crate) fn copy_shared(&self) -> Frame {
        match self {
            Frame::Shared(f) => Frame::Shared(f.clone()),
            Frame::Inline { .. } => unreachable!("okay: a frame of a rest taken not shared"),
        }
    }

    /// the frame shared, for a rest `k`: an inline closure moved behind an `Rc`, once
    pub(crate) fn share(self) -> Shared {
        let me = std::mem::ManuallyDrop::new(self);
        match &*me {
            // SAFETY: the bits hold the closure of this table, moved out here; `me` is not dropped
            Frame::Inline { bits, vt } => unsafe { (vt.share)(bits.as_ptr() as *const u8) },
            // SAFETY: the `Rc` taken out of `me`, which is not dropped: one owner still
            Frame::Shared(f) => unsafe { std::ptr::read(f) },
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if let Frame::Inline { bits, vt } = self {
            // SAFETY: the bits hold the closure of this table, dropped once — here
            unsafe { (vt.drop)(bits.as_mut_ptr() as *mut u8) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eff::Kind;
    use std::cell::Cell;

    #[test]
    fn small_and_large_closures_called_cloned_dropped() {
        let n = 41i64;
        let f = Frame::new(move |v: Val| Node(Kind::Return(Val::new(v.take::<i64>() + n))));
        assert!(matches!(f, Frame::Inline { .. }));
        let g = f.share();
        assert_eq!(super::super::machine::run(g(Val::new(1i64))).take::<i64>(), 42);
        assert_eq!(super::super::machine::run(Frame::Shared(g.clone()).call(Val::new(2i64))).take::<i64>(), 43);
        let big = [7i64; 8];
        let h = Frame::new(move |_: Val| Node(Kind::Return(Val::new(big[0]))));
        assert!(matches!(h, Frame::Shared(_)));
        assert_eq!(super::super::machine::run(h.call(Val::new(()))).take::<i64>(), 7);
        // a capture with a destructor: dropped once — inline, or moved behind an `Rc`
        let drops = Rc::new(Cell::new(0));
        struct D(Rc<Cell<i32>>);
        impl Drop for D { fn drop(&mut self) { self.0.set(self.0.get() + 1) } }
        let d = D(drops.clone());
        let k = Frame::new(move |_: Val| { let _ = &d; Node(Kind::Return(Val::new(0i64))) });
        std::mem::drop(k);
        assert_eq!(drops.get(), 1);
        let d2 = D(drops.clone());
        let k2 = Frame::new(move |_: Val| { let _ = &d2; Node(Kind::Return(Val::new(0i64))) }).share();
        let k3 = k2.clone();
        std::mem::drop(k2);
        assert_eq!(drops.get(), 1);
        std::mem::drop(k3);
        assert_eq!(drops.get(), 2);
    }
}
