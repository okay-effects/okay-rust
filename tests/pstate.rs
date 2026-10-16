//! okay-core's TestPState and TestAtmUses: the state's type in the answer type, in control — what was done checked by
//! the compiler (its refusals: src/cont.rs's doc tests), in a program of effects of the row beside it

use okay::cont::{pstate, PState, P};
use okay::effects::*;
use okay::*;

#[test]
fn pstate_get_after_put_the_type_unchanged() {
    let p = pstate::<Nil, i64, i64, i64>(1, |st| {
        st.put::<i64, i64>(41).flat_map(|_| okay::cont::state::<Nil, (i64, i64)>().get::<i64>()).map(|n| n + 1)
    });
    assert_eq!(p.run(), (41, 42));
}

#[test]
fn pstate_the_type_changed_an_int_in_a_string_put_read_back_as_a_string() {
    type F = (String, i64);
    let p = pstate::<Nil, i64, String, i64>(7, |st| {
        st.get::<i64>()
            .flat_map(|n| okay::cont::state::<Nil, F>().put::<i64, String>("x".repeat(n as usize)))
            .flat_map(|_| okay::cont::state::<Nil, F>().get::<String>())
            .map(|s| s.len() as i64)
    });
    assert_eq!(p.run(), ("xxxxxxx".to_string(), 7));
}

#[test]
fn pstate_under_an_effect_of_the_row_an_operation_between_the_moves_out_of_the_dollar() {
    type R = row![Reader<i64>];
    type F = (String, i64);
    let p = pstate::<R, i64, String, i64>(1, |st| {
        st.get::<i64>()
            .flat_map(|n| {
                ask::<i64, Delim<R>, _>()
                    .cont::<P<R, i64, F>>()
                    .flat_map(move |a| okay::cont::state::<R, F>().put::<i64, String>((n + a).to_string()))
            })
            .map(|_| 0)
    });
    assert_eq!(p.handle(reader(5)).run(), ("6".to_string(), 0));
}

// ATM AT WORK (TestAtmUses): a protocol — a connection closed, open, logged in — each step its state before and after in
// its type; the port asked of the row, the query told to it

type R = row![Writer<String>, Reader<i64>];

#[derive(Clone, Debug, PartialEq)]
struct Closed;
#[derive(Clone, Debug, PartialEq)]
struct Open {
    host: String,
}
#[derive(Clone, Debug, PartialEq)]
struct Authed {
    host: String,
    user: String,
}

type Fin = (Closed, i64);

/// a step of the protocol: the connection's state `From` before it, `To` after it, its value `A`
type Move<From, To, A> = Cont<Delim<R>, P<R, To, Fin>, P<R, From, Fin>, A>;

fn st() -> PState<R, Fin> {
    okay::cont::state::<R, Fin>()
}

fn connect(host: String) -> Move<Closed, Open, ()> {
    st().get::<Closed>().flat_map(move |_| st().put::<Closed, Open>(Open { host: host.clone() }))
}

fn login(user: &'static str) -> Move<Open, Authed, ()> {
    st().get::<Open>().flat_map(move |o| st().put::<Open, Authed>(Authed { host: o.host, user: user.to_string() }))
}

fn query(q: &'static str) -> Move<Authed, Authed, i64> {
    st().get::<Authed>().flat_map(move |a| {
        tell::<String, Delim<R>, _>(format!("{}@{}: {q}", a.user, a.host)).cont::<P<R, Authed, Fin>>().map(move |_| q.len() as i64)
    })
}

fn close() -> Move<Authed, Closed, ()> {
    st().get::<Authed>().flat_map(|_| st().put::<Authed, Closed>(Closed))
}

#[test]
fn a_protocol_connect_login_query_close() {
    let session = pstate::<R, Closed, Closed, i64>(Closed, |_| {
        ask::<i64, Delim<R>, _>().cont::<P<R, Closed, Fin>>().flat_map(|port| {
            connect(format!("db:{port}"))
                .flat_map(|_| login("alice"))
                .flat_map(|_| query("select 1"))
                .flat_map(|n| close().map(move |_| n))
        })
    });
    assert_eq!(session.handle(reader(5432)).handle(collect()).run(), (vec!["alice@db:5432: select 1".to_string()], (Closed, 8)));
}

// A PIPELINE: the rows' schema the state's type — each stage adds its columns, a stage needing a column runs only after
// the one adding it

#[derive(Clone, Debug, PartialEq)]
struct Raw {
    id: i64,
    amount: i64,
}
#[derive(Clone, Debug, PartialEq)]
struct Enriched {
    id: i64,
    amount: i64,
    country: &'static str,
}
#[derive(Clone, Debug, PartialEq)]
struct Scored {
    id: i64,
    amount: i64,
    country: &'static str,
    risky: bool,
}

type PF = (Vec<Scored>, i64);

/// a stage of the pipeline: the rows' schema `From` before it, `To` after it
type Stage<From, To, A> = Cont<Delim<R>, P<R, To, PF>, P<R, From, PF>, A>;

fn pst() -> PState<R, PF> {
    okay::cont::state::<R, PF>()
}

fn load(rows: Vec<Raw>) -> Stage<(), Vec<Raw>, ()> {
    pst().get::<()>().flat_map(move |_| pst().put::<(), Vec<Raw>>(rows.clone()))
}

fn enrich() -> Stage<Vec<Raw>, Vec<Enriched>, ()> {
    pst().get::<Vec<Raw>>().flat_map(|rows| {
        let n = rows.len();
        let enriched: Vec<Enriched> =
            rows.into_iter().map(|x| Enriched { id: x.id, amount: x.amount, country: if x.id % 2 == 0 { "PL" } else { "UA" } }).collect();
        tell::<String, Delim<R>, _>(format!("enriched {n}"))
            .cont::<P<R, Vec<Raw>, PF>>()
            .flat_map(move |_| pst().put::<Vec<Raw>, Vec<Enriched>>(enriched.clone()))
    })
}

fn score() -> Stage<Vec<Enriched>, Vec<Scored>, i64> {
    ask::<i64, Delim<R>, _>().cont::<P<R, Vec<Enriched>, PF>>().flat_map(|limit| {
        pst().get::<Vec<Enriched>>().flat_map(move |rows| {
            let scored: Vec<Scored> = rows
                .into_iter()
                .map(|x| Scored { id: x.id, amount: x.amount, country: x.country, risky: x.amount > limit && x.country != "PL" })
                .collect();
            let n = scored.iter().filter(|s| s.risky).count() as i64;
            pst().put::<Vec<Enriched>, Vec<Scored>>(scored).map(move |_| n)
        })
    })
}

#[test]
fn a_pipeline_load_enrich_score() {
    let raw = vec![Raw { id: 1, amount: 50 }, Raw { id: 2, amount: 900 }, Raw { id: 3, amount: 700 }];
    let run = pstate::<R, (), Vec<Scored>, i64>((), move |_| {
        load(raw.clone()).flat_map(|_| enrich()).flat_map(|_| score())
    });
    let (told, (rows, risky)) = run.handle(reader(500)).handle(collect()).run();
    assert_eq!(told, vec!["enriched 3".to_string()]);
    assert_eq!(risky, 1);
    assert_eq!(rows.iter().map(|r| r.risky).collect::<Vec<_>>(), vec![false, false, true]);
}
