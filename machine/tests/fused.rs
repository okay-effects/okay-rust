//! okay-core's TestFused, its behaviour: many maps in constant stack, a panic in the middle of a chain stops the maps
//! after it, a shift under many maps (how maps are kept is the machine's own)

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay_machine::*;

#[test]
fn many_maps_of_a_value_in_constant_stack() {
    let mut p: Eff<Nil, i64> = pure(0);
    for _ in 0..100_000 {
        p = p.map(|x| x + 1);
    }
    assert_eq!(p.run(), 100_000);
}

#[test]
fn a_panic_in_the_middle_of_a_chain_out_of_the_run_the_maps_after_it_not_run() {
    let after = Rc::new(Cell::new(false));
    let a = after.clone();
    let p = defer(|| pure::<Nil, i64>(1))
        .map(|x| x + 1)
        .map(|x| if x == 2 { panic!("two") } else { x })
        .map(move |x| { a.set(true); x });
    assert!(catch_unwind(AssertUnwindSafe(|| p.run())).is_err());
    assert!(!after.get());
}

#[test]
fn a_shift_under_many_maps_k_the_maps_each_resumption_all_of_them() {
    use okay_machine::effects::{reset, shift, Shifts};
    type SR = row![Shifts<i64, Nil>];
    let mut p: Eff<SR, i64> = shift::<i64, Nil, i64, SR, _>(|k| {
        let k2 = k.clone();
        k(1).flat_map(move |a| k2(10).map(move |b| a + b)).widen()
    });
    for _ in 0..50 {
        p = p.map(|x| x + 1);
    }
    assert_eq!(p.handle(reset()).run(), (1 + 50) + (10 + 50));
}
