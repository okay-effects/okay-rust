//! a program of the machine over a wire: its operations by name and record, its rest resumed by the other side — as
//! many times as it wants

use okay::effects::*;
use okay::wire::{program, serve, Remotes, Wire};
use okay::*;

effect! {
    /// a price list, over the wire
    pub enum Shop: Plain, wire {
        PriceOf(sku: String) -> f64 = price_of;
        Discount(amount: f64) -> f64 = discount;
    }
}

/// the far side's CHOICE: every path, `among` answered with each index in turn — the same rest resumed n times
fn every_path(w: Wire) -> Vec<Data> {
    match w {
        Wire::Done(d) => vec![d],
        Wire::Failed(why) => panic!("{why}"),
        Wire::Perform { name, args, k } => {
            assert_eq!(name, "among");
            let n = i64::from_data(args[0].field("n").unwrap()).unwrap();
            (0..n).flat_map(|i| every_path(k(Ok(Data::Int(i))))).collect()
        }
    }
}

#[test]
fn multi_shot_across_the_wire_every_branch_of_two_choices() {
    type R = row![Choose];
    let pairs: Eff<R, i64> = eff! { data;
        x <- among(vec![1i64, 2]);
        y <- among(vec![10i64, 20]);
        yield x + y
    };
    let got = every_path(program(pairs.step(), &Remotes::new().with::<Choose>()));
    assert_eq!(got, vec![Data::Int(11), Data::Int(21), Data::Int(12), Data::Int(22)]);
}

#[test]
fn an_effect_declared_wire_crosses_by_its_functions_names() {
    type R = row![Shop];
    let total: Eff<R, f64> = eff! { data;
        p <- Shop::price_of("tea".to_string());
        Shop::discount(p * 3.0)
    };
    let mut asked = Vec::new();
    let mut w = program(total.step(), &Remotes::new().with::<Shop>());
    let got = loop {
        w = match w {
            Wire::Done(d) => break d,
            Wire::Failed(why) => panic!("{why}"),
            Wire::Perform { name, args, k } => {
                asked.push(format!("{name} {:?}", args[0]));
                match name.as_str() {
                    "price_of" => k(Ok(Data::Float(4.0))),
                    "discount" => k(Ok(Data::Float(f64::from_data(args[0].field("amount").unwrap()).unwrap() * 0.5))),
                    other => panic!("{other}"),
                }
            }
        }
    };
    assert_eq!(got, Data::Float(6.0));
    assert_eq!(asked, vec![
        r#"price_of Dict([("sku", Str("tea"))])"#.to_string(),
        r#"discount Dict([("amount", Float(12.0))])"#.to_string(),
    ]);
}

#[test]
fn a_failure_from_the_far_side_is_raised_at_the_operation() {
    type R = row![Shop];
    let p: Eff<R, f64> = Shop::price_of("x".to_string());
    let Wire::Perform { k, .. } = program(p.step(), &Remotes::new().with::<Shop>()) else { panic!() };
    assert!(matches!(k(Err("no such sku".into())), Wire::Failed(m) if m == "no such sku"));
    assert!(matches!(k(Ok(Data::Str("not a number".into()))), Wire::Failed(m) if m.contains("expected a number")));
}

#[test]
fn an_effect_not_declared_to_cross_is_refused_by_name() {
    type R = row![Reader<i64>];
    let p: Eff<R, i64> = ask::<i64, R, _>();
    assert!(matches!(program(p.step(), &Remotes::new()), Wire::Failed(m) if m.contains("does not cross the wire")));
}

#[test]
fn the_other_way_a_rust_handler_answers_the_wire() {
    let prices = serve(|op: Shop| match op {
        Shop::PriceOf(sku, r) => r.give(if sku == "tea" { 4.0 } else { 0.0 }),
        Shop::Discount(a, r) => r.give(a * 0.5),
    });
    let rec = |k: &str, v: Data| Data::Dict(vec![(k.into(), v)]);
    assert_eq!(prices("price_of", &rec("sku", Data::Str("tea".into()))), Some(Ok(Data::Float(4.0))));
    assert_eq!(prices("discount", &rec("amount", Data::Float(10.0))), Some(Ok(Data::Float(5.0))));
    assert_eq!(prices("nope", &Data::Dict(vec![])), None);
    assert!(prices("price_of", &Data::Dict(vec![])).unwrap().is_err());
    // a standard effect too: a state served
    let st = std::cell::Cell::new(1i64);
    let state = serve(|op: State<i64>| match op {
        State::Get(r) => r.give(st.get()),
        State::Put(s, r) => { st.set(s); r.give(()) }
    });
    assert_eq!(state("put", &rec("s", Data::Int(7))), Some(Ok(Data::Null)));
    assert_eq!(state("get", &Data::Dict(vec![])), Some(Ok(Data::Int(7))));
}
