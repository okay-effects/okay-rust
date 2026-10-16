//! LEVEL 1, direct style: the programs of tests/intercept.rs (okay-core's TestIntercept), their effects passed down as
//! parameters, an interceptor a wrapper — the same trails

use std::cell::RefCell;

use okay::direct::*;

// THE STANDARD EFFECTS

fn program(r: &mut impl Reader<i64>, st: &mut impl State<i64>, w: &mut impl Writer<String>) -> i64 {
    let n = r.ask();
    st.modify(|s| s + n);
    let s = st.get();
    w.tell(format!("s {s}"));
    s * 2
}

/// "reader, state, writer: each by its handler"
#[test]
fn reader_state_writer_each_by_its_handler() {
    let (mut st, mut log) = (Cell(1), Log::new());
    assert_eq!(program(&mut Const(5), &mut st, &mut log), 12);
    assert_eq!((st.0, log.0), (6, vec!["s 6".to_string()]));
}

/// "a plain effect intercepted for a part of the program: its clause runs outside it, its own operation reaches the
/// handler"
#[test]
fn a_plain_effect_intercepted_for_a_part_of_the_program() {
    let mut log = Log::new();
    log.tell("a".to_string());
    {
        let mut shouting = InterceptWriter(&mut log, |s: String, out: &mut Log<String>| out.tell(s.to_uppercase()));
        shouting.tell("b".to_string());
    }
    assert_eq!(log.0, vec!["a".to_string(), "B".to_string()]);
}

// A SENSITIVE EFFECT: payments — the program's trait asks AuditedOnly; the bank's is another

pub trait Pay: AuditedOnly {
    fn transfer(&mut self, to: &str, amount: i64) -> Result<String, String>;
    fn balance(&mut self) -> i64;
}

/// what a handler of payments implements: reached by a program only through an audit
pub trait Bank {
    fn send(&mut self, to: &str, amount: i64) -> String;
    fn balance(&mut self) -> i64;
}

struct Funds(i64);
impl Bank for Funds {
    fn send(&mut self, to: &str, amount: i64) -> String {
        self.0 -= amount;
        format!("ok:{to}:{amount}")
    }
    fn balance(&mut self) -> i64 {
        self.0
    }
}

fn masked(account: &str) -> String {
    let tail: String = account.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("****{tail}")
}

/// THE AUDIT OF PAYMENTS: a transfer over the limit refused; the account written one way (changed before it is passed
/// on); each one told, its account masked, with its receipt; the balance told with its value
struct Guard<'t> {
    limit: i64,
    trail: &'t mut Log<String>,
}

impl<B: Bank> Pay for Audited<'_, B, Guard<'_>> {
    fn transfer(&mut self, to: &str, amount: i64) -> Result<String, String> {
        let to = to.trim().to_uppercase();
        if amount > self.interceptor.limit {
            let why = format!("refused: {amount} to {} over the limit {}", masked(&to), self.interceptor.limit);
            self.interceptor.trail.tell(why.clone());
            return Err(why);
        }
        let receipt = self.handler.send(&to, amount);
        self.interceptor.trail.tell(format!("transfer {amount} to {} → {receipt}", masked(&to)));
        Ok(receipt)
    }
    fn balance(&mut self) -> i64 {
        let b = self.handler.balance();
        self.interceptor.trail.tell(format!("balance → {b}"));
        b
    }
}

/// an observed bank: every payment told with its answer — an observer is an audit too
impl<B: Bank, O: Observer> Pay for Observed<'_, '_, B, O> {
    fn transfer(&mut self, to: &str, amount: i64) -> Result<String, String> {
        let r = self.handler.send(to, amount);
        self.seen("Pay", &format_args!("Transfer({to:?}, {amount})"), &r);
        Ok(r)
    }
    fn balance(&mut self) -> i64 {
        let b = self.handler.balance();
        self.seen("Pay", &"Balance", &b);
        b
    }
}

fn payments(pay: &mut impl Pay) -> Result<i64, String> {
    pay.transfer(" acct-1234 ", 100)?;
    let b = pay.balance();
    pay.transfer("acct-77", 5000)?;
    Ok(b)
}

/// "a sensitive effect under its audit: passed on changed, told masked, refused over the limit"
#[test]
fn a_sensitive_effect_under_its_audit_passed_on_changed_told_masked_refused_over_the_limit() {
    let (mut bank, mut trail) = (Funds(1000), Log::new());
    let result = payments(&mut audit(&mut bank, Guard { limit: 1000, trail: &mut trail }));
    assert_eq!(
        trail.0,
        vec![
            "transfer 100 to ****1234 → ok:ACCT-1234:100".to_string(),
            "balance → 900".to_string(),
            "refused: 5000 to ****T-77 over the limit 1000".to_string(),
        ]
    );
    assert_eq!(result, Err("refused: 5000 to ****T-77 over the limit 1000".to_string()));
}

/// "an observer sees every operation of every effect, and its answer; the sensitive ones audited"
#[test]
fn an_observer_sees_every_operation_of_every_effect_and_its_answer() {
    let seen = RefCell::new(Trail::default());
    let (mut r, mut st, mut bank) = (Const(5i64), Cell(1i64), Funds(100));
    // three handlers, one observer
    let (mut r, mut st, mut pay) = (observe(&mut r, &seen), observe(&mut st, &seen), observe(&mut bank, &seen));
    let n = r.ask();
    st.modify(|s| s + n);
    let s = st.get();
    let receipt = pay.transfer("acct", s);
    let seen = seen.into_inner();
    assert_eq!(receipt, Ok("ok:acct:6".to_string()));
    assert_eq!(
        seen.0,
        vec![
            "Reader \"Ask\"", "  → 5",
            "State \"Get\"", "  → 1",
            "State Put(6)", "  → ()",
            "State \"Get\"", "  → 6",
            "Pay Transfer(\"acct\", 6)", "  → \"ok:acct:6\"",
        ]
    );
}
