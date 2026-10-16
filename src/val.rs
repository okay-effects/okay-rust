//! A VALUE ERASED, without an allocation when it is small: up to 16 bytes, aligned to 8 at most, with nothing to drop
//! (`i64`, `bool`, `()`, a `Reply`, an operation of such) kept in place; else boxed. Taken back at the type it was made
//! at, checked by its `TypeId` — `Box<dyn Any>`'s `downcast`, for small values too.

use std::any::{Any, TypeId};
use std::mem::{align_of, needs_drop, size_of, MaybeUninit};

/// a value on its way, erased
pub(crate) struct Val(Repr);

enum Repr {
    Small { ty: TypeId, bits: MaybeUninit<[u64; 2]> },
    Boxed(Box<dyn Any>),
}

/// kept in place: it fits, and dropping it is nothing
const fn small<T>() -> bool {
    size_of::<T>() <= 16 && align_of::<T>() <= 8 && !needs_drop::<T>()
}

impl Val {
    #[inline(always)]
    pub(crate) fn new<T: 'static>(t: T) -> Val {
        if small::<T>() {
            let mut bits = MaybeUninit::<[u64; 2]>::uninit();
            // SAFETY: `T` fits in 16 bytes aligned to 8 (`small`), and needs no drop: the bits are its owner
            unsafe { std::ptr::write(bits.as_mut_ptr() as *mut T, t) };
            Val(Repr::Small { ty: TypeId::of::<T>(), bits })
        } else {
            Val(Repr::Boxed(Box::new(t)))
        }
    }

    /// the value at its type: the type it was made at — the typed API allows no other
    #[inline(always)]
    pub(crate) fn take<T: 'static>(self) -> T {
        match self.0 {
            Repr::Small { ty, bits } if ty == TypeId::of::<T>() => {
                // SAFETY: made by `new::<T>` (the same `TypeId`), written there whole; read once, `self` consumed
                unsafe { std::ptr::read(bits.as_ptr() as *const T) }
            }
            Repr::Boxed(b) => match b.downcast::<T>() {
                Ok(t) => *t,
                Err(_) => wrong(),
            },
            Repr::Small { .. } => wrong(),
        }
    }

    /// the value seen at its type, not taken
    pub(crate) fn peek<T: 'static>(&self) -> Option<&T> {
        match &self.0 {
            Repr::Small { ty, bits } if *ty == TypeId::of::<T>() => {
                // SAFETY: as `take`, a shared look
                Some(unsafe { &*(bits.as_ptr() as *const T) })
            }
            Repr::Boxed(b) => b.downcast_ref::<T>(),
            Repr::Small { .. } => None,
        }
    }
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
    }
}
