//! A VALUE ERASED, without an allocation when it is small: up to 16 bytes, aligned to 8 at most, with nothing to drop
//! (`i64`, `bool`, `()`, a `Reply`, an operation of such) kept in place; else boxed. Taken back at the type it was made
//! at, checked by its `TypeId` — in place beside a small value, at the head of a boxed one: a load, never a call through
//! the box's vtable. A take has nothing to clean up if it unwinds, so a program taking an answer inlines into its
//! handler (with a cleanup there — `downcast` calls `type_id` — a resumable choice ran 6.7× slower).

use std::any::{Any, TypeId};
use std::mem::{align_of, needs_drop, size_of, MaybeUninit};

/// a value on its way, erased
pub struct Val(Repr);

enum Repr {
    Small { ty: TypeId, bits: MaybeUninit<[u64; 2]> },
    Boxed(Box<Tagged<dyn Any>>),
}

/// a boxed value and its type, at the head: read without a call
struct Tagged<T: ?Sized> {
    ty: TypeId,
    v: T,
}

/// kept in place: it fits, and dropping it is nothing
const fn small<T>() -> bool {
    size_of::<T>() <= 16 && align_of::<T>() <= 8 && !needs_drop::<T>()
}

impl Val {
    #[inline(always)]
    pub fn new<T: 'static>(t: T) -> Val {
        if small::<T>() {
            let mut bits = MaybeUninit::<[u64; 2]>::uninit();
            // SAFETY: `T` fits in 16 bytes aligned to 8 (`small`), and needs no drop: the bits are its owner
            unsafe { std::ptr::write(bits.as_mut_ptr() as *mut T, t) };
            Val(Repr::Small { ty: TypeId::of::<T>(), bits })
        } else {
            Val(Repr::Boxed(Box::new(Tagged { ty: TypeId::of::<T>(), v: t })))
        }
    }

    /// the value at its type: the type it was made at — the typed API allows no other
    #[inline(always)]
    pub fn take<T: 'static>(self) -> T {
        match self.0 {
            Repr::Small { ty, bits } if ty == TypeId::of::<T>() => {
                // SAFETY: made by `new::<T>` (the same `TypeId`), written there whole; read once, `self` consumed
                unsafe { std::ptr::read(bits.as_ptr() as *const T) }
            }
            Repr::Boxed(b) if b.ty == TypeId::of::<T>() => {
                // SAFETY: made by `new::<T>` as a `Box<Tagged<T>>` (the same `TypeId`)
                unsafe { Box::from_raw(Box::into_raw(b) as *mut Tagged<T>) }.v
            }
            // dropped before the panic: nothing left to clean up as it unwinds
            Repr::Boxed(b) => mismatch(b),
            Repr::Small { .. } => wrong(),
        }
    }

    /// the value seen at its type, not taken
    pub fn peek<T: 'static>(&self) -> Option<&T> {
        match &self.0 {
            Repr::Small { ty, bits } if *ty == TypeId::of::<T>() => {
                // SAFETY: as `take`, a shared look
                Some(unsafe { &*(bits.as_ptr() as *const T) })
            }
            // SAFETY: as `take`, a shared look
            Repr::Boxed(b) if b.ty == TypeId::of::<T>() => Some(unsafe { &*(&b.v as *const dyn Any as *const T) }),
            _ => None,
        }
    }
}

#[cold]
#[inline(never)]
fn mismatch(b: Box<Tagged<dyn Any>>) -> ! {
    drop(b);
    wrong()
}

#[cold]
fn wrong() -> ! {
    panic!("okay: a value taken at another type than it was made at — a reply given to another operation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_and_boxed_values_taken_back() {
        assert_eq!(Val::new(42i64).take::<i64>(), 42);
        assert_eq!(Val::new((1u8, true)).take::<(u8, bool)>(), (1, true));
        assert_eq!(Val::new(String::from("x")).take::<String>(), "x");
        assert_eq!(Val::new(vec![1, 2, 3]).peek::<Vec<i32>>(), Some(&vec![1, 2, 3]));
        assert_eq!(Val::new(7u32).peek::<i64>(), None);
        assert_eq!(Val::new(String::from("y")).peek::<String>().map(|s| s.as_str()), Some("y"));
        assert_eq!(Val::new(String::from("y")).peek::<Vec<u8>>(), None);
    }

    #[test]
    fn a_boxed_value_not_taken_is_dropped_once() {
        let rc = std::rc::Rc::new(());
        let v = Val::new(rc.clone());
        assert_eq!(std::rc::Rc::strong_count(&rc), 2);
        drop(v);
        assert_eq!(std::rc::Rc::strong_count(&rc), 1);
        let taken: std::rc::Rc<()> = Val::new(rc.clone()).take();
        assert_eq!(std::rc::Rc::strong_count(&rc), 2);
        drop(taken);
        assert_eq!(std::rc::Rc::strong_count(&rc), 1);
    }

    #[test]
    #[should_panic(expected = "another type")]
    fn taken_at_another_type() {
        let _: u32 = Val::new(String::from("x")).take();
    }
}
