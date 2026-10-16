//! okay-core's TestFused, its behaviour: many maps in constant stack, a panic in the middle of a chain stops the maps
//! after it (how maps are kept is the machine's own; the test of a shift under maps waits for stage 2's control)

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use okay::*;

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
