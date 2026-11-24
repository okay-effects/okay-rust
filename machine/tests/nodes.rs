//! okay-core's TestNodes: `$`s nested in the resumed rests, in constant stack; answer type modification (printf)

use std::rc::Rc;

use okay_machine::cont::shift0;
use okay_machine::*;

/// `n` `$`s, each in the rest its outer `$`'s shift0 resumed (here each over no effect, widened under the outer: a row
/// a type, a recursion a row deeper each step would be one type per step)
fn nest(n: i64) -> Prog<i64, Nil> {
    if n == 0 {
        return pure(0);
    }
    let body: Cont<Reset, Prog<i64, Nil>, Prog<i64, Nil>, i64> = shift0::<Reset, Nil, _, (), Prog<i64, Nil>, Prog<i64, Nil>>(|k| k(()))
        .flat_map(move |_| nest(n - 1).widen::<Control<Nil>>().cont::<Prog<i64, Nil>>());
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
    let body: Cont<Reset, Prog<String, Nil>, Prog<S, Nil>, i64> =
        shift0::<Reset, Nil, _, i64, Prog<String, Nil>, Prog<S, Nil>>(|k| pure(Rc::new(move |x| k(x).run()) as S));
    let p: Prog<S, Nil> = dollar(body.map(|n| n.to_string()), |s| pure(format!("Hello {s}")));
    assert_eq!(p.run()(42), "Hello 42");
}

/// the old names, deprecated: the same program and handlers as the family's names
#[test]
#[allow(deprecated)]
fn old_names_still_work() {
    use okay_machine::effects::{ask, Reader};
    let p: Eff<row![Reader<i64>], i64> = ask::<i64, _, _>();
    let h: Handler<Reader<i64>, Nil, i64, i64> =
        Handler::answering(|op| match op { Reader::Ask(r) => r.give(2) }, |a| a);
    assert_eq!(p.handle(h).run(), 2);
    let q: Prog<i64, row![Reader<i64>]> = ask::<i64, _, _>();
    assert_eq!(q.handle(Handler::deep(pure, |op, k| match op { Reader::Ask(r) => r.resume(&k, 3) })).run(), 3);
}
