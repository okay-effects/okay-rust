//! okay-core's TestNodes: `$`s nested in the resumed rests, in constant stack; answer type modification (printf)

use std::rc::Rc;

use okay_machine::cont::shift0;
use okay_machine::*;

/// `n` `$`s, each in the rest its outer `$`'s shift0 resumed (here each over no effect, widened under the outer: a row
/// a type, a recursion a row deeper each step would be one type per step)
fn nest(n: i64) -> Eff<Nil, i64> {
    if n == 0 {
        return pure(0);
    }
    let body: Cont<Reset, Eff<Nil, i64>, Eff<Nil, i64>, i64> = shift0::<Reset, Nil, _, (), Eff<Nil, i64>, Eff<Nil, i64>>(|k| k(()))
        .flat_map(move |_| nest(n - 1).widen::<Control<Nil>>().cont::<Eff<Nil, i64>>());
    dollar(body, |a| pure(a + 1))
}

#[test]
fn stack_safe_dollars_nested_in_the_resumed_rests() {
    assert_eq!(nest(100_000).run(), 100_000);
}

#[test]
fn answer_type_modification_the_shift0_turns_the_answer_string_into_a_function_printf() {
    type S = Rc<dyn Fn(i64) -> String>;
    // `$ ("Hello " + shift0(k => x => k(x)))`
    let body: Cont<Reset, Eff<Nil, String>, Eff<Nil, S>, i64> =
        shift0::<Reset, Nil, _, i64, Eff<Nil, String>, Eff<Nil, S>>(|k| pure(Rc::new(move |x| k(x).run()) as S));
    let p: Eff<Nil, S> = dollar(body.map(|n| n.to_string()), |s| pure(format!("Hello {s}")));
    assert_eq!(p.run()(42), "Hello 42");
}
