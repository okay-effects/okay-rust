//! THE SERVICES a program asks of the world, as effects — so a test answers them at once and the same program runs in
//! production under the real ones: the time (`Clock`: virtual or the system's), randomness (`Random`: seeded, the same
//! each run), a log (`Log`), the environment (`Env`) and its SECRETS (`Secrets`: a sensitive effect — read only under
//! an audit), fresh numbers (`Fresh`), a store of keys and values (`Store`).
//!
//! ```
//! use okay::services::*;
//! use okay::typed::*;
//!
//! okay::needs! {
//!     async fn roll(cx: [Clock, Random, Log]) -> u64 {
//!         let t = now_ms(cx).await;
//!         let n = random_below(cx, 6).await + 1;
//!         info(cx, format!("rolled {n} at {t}")).await;
//!         n
//!     }
//! }
//! let cx = Cx(VirtualClock::at(1_000), Cx(SplitMix::seeded(7), Cx(LogCollect::at_least(Level::Info), End)));
//! let first = run(roll(&cx)).unwrap();
//! let again = Cx(VirtualClock::at(1_000), Cx(SplitMix::seeded(7), Cx(LogCollect::at_least(Level::Info), End)));
//! assert_eq!(run(roll(&again)).unwrap(), first);   // seeded: the same each run
//! assert_eq!(cx.1.1.0.lines(), vec![(Level::Info, format!("rolled {first} at 1000"))]);
//! ```

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::kinds::{Effect, Plain};
use crate::reply::Reply;
use crate::typed::{perform, Handles, Has, Perform, Step};

// THE CLOCK

crate::effect! {
    /// the time: now, and a wait
    pub enum Clock: Plain, waits, wire {
        /// milliseconds since the epoch
        Now() -> u64 = now_ms;
        /// a wait of `ms` milliseconds
        Sleep(ms: u64) -> () = sleep_ms;
    }
}

/// VIRTUAL TIME: every wait answered at once, the time moved by it — a test of timeouts and retries runs instantly
pub struct VirtualClock(Cell<u64>);
impl VirtualClock {
    pub fn at(ms: u64) -> Self {
        VirtualClock(Cell::new(ms))
    }
    /// the time now
    pub fn now(&self) -> u64 {
        self.0.get()
    }
}
impl Handles<Clock> for VirtualClock {
    fn handle(&self, op: Clock) -> Step<'_> {
        match op {
            Clock::Now(r) => Step::Now(r.give(self.0.get())),
            Clock::Sleep(ms, r) => {
                self.0.set(self.0.get() + ms);
                Step::Now(r.give(()))
            }
        }
    }
}

/// THE SYSTEM'S TIME: a wait a real one (a timer thread wakes the program): run it with `block_on`, or any executor
pub struct SystemClock;
impl Handles<Clock> for SystemClock {
    fn handle(&self, op: Clock) -> Step<'_> {
        match op {
            Clock::Now(r) => {
                let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                Step::Now(r.give(ms))
            }
            Clock::Sleep(ms, r) => Step::Wait(Box::pin(async move {
                timer(ms).await;
                r.give(())
            })),
        }
    }
}

/// a wait of `ms` milliseconds on the system's time: a thread sleeps, then wakes the program — std alone
pub fn timer(ms: u64) -> impl Future<Output = ()> {
    struct Timer {
        shared: Arc<Mutex<(bool, Option<Waker>)>>,
        after: Option<Duration>,
    }
    impl Future for Timer {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<()> {
            let mut s = self.shared.lock().unwrap();
            if s.0 {
                return Poll::Ready(());
            }
            s.1 = Some(c.waker().clone());
            drop(s);
            if let Some(d) = self.after.take() {
                let shared = self.shared.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(d);
                    let mut s = shared.lock().unwrap();
                    s.0 = true;
                    if let Some(w) = s.1.take() {
                        w.wake();
                    }
                });
            }
            Poll::Pending
        }
    }
    Timer { shared: Arc::new(Mutex::new((false, None))), after: Some(Duration::from_millis(ms)) }
}

// RANDOMNESS

crate::effect! {
    /// randomness: 64 random bits
    pub enum Random: Plain, wire {
        Bits() -> u64 = random_u64;
    }
}

/// a number below `bound` (`bound > 0`)
pub async fn random_below<C: Has<Random, I>, I>(cx: &C, bound: u64) -> u64 {
    random_u64(cx).await % bound
}

/// a number in [0, 1)
pub async fn random_f64<C: Has<Random, I>, I>(cx: &C) -> f64 {
    (random_u64(cx).await >> 11) as f64 / (1u64 << 53) as f64
}

/// SPLITMIX64 from a seed: the same numbers each run — a test's randomness
pub struct SplitMix(Cell<u64>);
impl SplitMix {
    pub fn seeded(seed: u64) -> Self {
        SplitMix(Cell::new(seed))
    }
    /// seeded from the system's time: different each run
    pub fn from_time() -> Self {
        SplitMix(Cell::new(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)))
    }
}
impl Handles<Random> for SplitMix {
    fn handle(&self, op: Random) -> Step<'_> {
        match op {
            Random::Bits(r) => {
                let s = self.0.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
                self.0.set(s);
                let mut z = s;
                z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                Step::Now(r.give(z ^ (z >> 31)))
            }
        }
    }
}

// A LOG

/// how much a line matters
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// a line logged at its level
#[derive(Clone, Debug)]
pub enum Log {
    Write(Level, String, Reply<()>),
}
impl Effect for Log {
    type Kind = Plain;
}

/// a line at `level`
pub fn log<C: Has<Log, I>, I>(cx: &C, level: Level, line: String) -> Perform<'_, C, Log, I, ()> {
    perform(cx, |r| Log::Write(level, line, r))
}
pub fn debug<C: Has<Log, I>, I>(cx: &C, line: String) -> Perform<'_, C, Log, I, ()> {
    log(cx, Level::Debug, line)
}
pub fn info<C: Has<Log, I>, I>(cx: &C, line: String) -> Perform<'_, C, Log, I, ()> {
    log(cx, Level::Info, line)
}
pub fn warn<C: Has<Log, I>, I>(cx: &C, line: String) -> Perform<'_, C, Log, I, ()> {
    log(cx, Level::Warn, line)
}
pub fn error<C: Has<Log, I>, I>(cx: &C, line: String) -> Perform<'_, C, Log, I, ()> {
    log(cx, Level::Error, line)
}

/// the lines at `least` and above, kept in order
pub struct LogCollect {
    least: Level,
    lines: RefCell<Vec<(Level, String)>>,
}
impl LogCollect {
    pub fn at_least(least: Level) -> Self {
        LogCollect { least, lines: RefCell::new(Vec::new()) }
    }
    /// the lines so far, taken
    pub fn lines(&self) -> Vec<(Level, String)> {
        std::mem::take(&mut self.lines.borrow_mut())
    }
}
impl Handles<Log> for LogCollect {
    fn handle(&self, op: Log) -> Step<'_> {
        match op {
            Log::Write(level, line, r) => {
                if level >= self.least {
                    self.lines.borrow_mut().push((level, line));
                }
                Step::Now(r.give(()))
            }
        }
    }
}

/// the lines at `least` and above, each to `sink` (stderr, a file, a service)
pub struct LogTo<F: Fn(Level, &str)> {
    least: Level,
    sink: F,
}
impl<F: Fn(Level, &str)> LogTo<F> {
    pub fn new(least: Level, sink: F) -> Self {
        LogTo { least, sink }
    }
}
impl<F: Fn(Level, &str)> Handles<Log> for LogTo<F> {
    fn handle(&self, op: Log) -> Step<'_> {
        match op {
            Log::Write(level, line, r) => {
                if level >= self.least {
                    (self.sink)(level, &line);
                }
                Step::Now(r.give(()))
            }
        }
    }
}

// THE ENVIRONMENT, AND ITS SECRETS

crate::effect! {
    /// the environment's variables
    pub enum Env: Plain, wire {
        Variable(name: String) -> Option<String> = variable;
    }
}

/// a secret's value: never printed (`Debug` says `Secret(***)`), read by `expose`
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }
    /// the value, for the one place that needs it
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

crate::effect! {
    /// SECRETS: a sensitive effect — handled only under its audit (`Audited::new(secrets, interceptor)`), so every
    /// read of a secret is seen, and may be refused
    pub enum Secrets: Sensitive {
        Read(name: String) -> Option<Secret> = secret;
    }
}

/// variables and secrets from maps: a test's, a configuration's
pub struct EnvOf {
    vars: HashMap<String, String>,
    secrets: HashMap<String, String>,
}
impl EnvOf {
    pub fn new<K: Into<String>, V: Into<String>>(
        vars: impl IntoIterator<Item = (K, V)>,
        secrets: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        let map = |kv: (K, V)| (kv.0.into(), kv.1.into());
        EnvOf { vars: vars.into_iter().map(map).collect(), secrets: secrets.into_iter().map(map).collect() }
    }
}
impl Handles<Env> for EnvOf {
    fn handle(&self, op: Env) -> Step<'_> {
        match op {
            Env::Variable(name, r) => Step::Now(r.give(self.vars.get(&name).cloned())),
        }
    }
}
impl Handles<Secrets> for EnvOf {
    fn handle(&self, op: Secrets) -> Step<'_> {
        match op {
            Secrets::Read(name, r) => Step::Now(r.give(self.secrets.get(&name).cloned().map(Secret))),
        }
    }
}

/// the process's environment: variables and secrets both from it
pub struct EnvSystem;
impl Handles<Env> for EnvSystem {
    fn handle(&self, op: Env) -> Step<'_> {
        match op {
            Env::Variable(name, r) => Step::Now(r.give(std::env::var(name).ok())),
        }
    }
}
impl Handles<Secrets> for EnvSystem {
    fn handle(&self, op: Secrets) -> Step<'_> {
        match op {
            Secrets::Read(name, r) => Step::Now(r.give(std::env::var(name).ok().map(Secret))),
        }
    }
}

// FRESH NUMBERS

crate::effect! {
    /// a number not given before
    pub enum Fresh: Plain, wire {
        Next() -> u64 = fresh;
    }
}

/// counting from `n`
pub struct FreshFrom(Cell<u64>);
impl FreshFrom {
    pub fn new(n: u64) -> Self {
        FreshFrom(Cell::new(n))
    }
}
impl Handles<Fresh> for FreshFrom {
    fn handle(&self, op: Fresh) -> Step<'_> {
        match op {
            Fresh::Next(r) => {
                let n = self.0.get();
                self.0.set(n + 1);
                Step::Now(r.give(n))
            }
        }
    }
}

// A STORE OF KEYS AND VALUES

/// a store: a value by its key loaded, saved, removed
#[derive(Clone, Debug)]
pub enum Store<K, V> {
    Load(K, Reply<Option<V>>),
    Save(K, V, Reply<()>),
    Remove(K, Reply<Option<V>>),
}
impl<K: Clone + Debug + 'static, V: Clone + Debug + 'static> Effect for Store<K, V> {
    type Kind = Plain;
}

pub fn load<K, V, C: Has<Store<K, V>, I>, I>(cx: &C, k: K) -> Perform<'_, C, Store<K, V>, I, Option<V>>
where
    K: Clone + Debug + 'static,
    V: Clone + Debug + 'static,
{
    perform(cx, |r| Store::Load(k, r))
}
pub fn save<K, V, C: Has<Store<K, V>, I>, I>(cx: &C, k: K, v: V) -> Perform<'_, C, Store<K, V>, I, ()>
where
    K: Clone + Debug + 'static,
    V: Clone + Debug + 'static,
{
    perform(cx, |r| Store::Save(k, v, r))
}
pub fn remove<K, V, C: Has<Store<K, V>, I>, I>(cx: &C, k: K) -> Perform<'_, C, Store<K, V>, I, Option<V>>
where
    K: Clone + Debug + 'static,
    V: Clone + Debug + 'static,
{
    perform(cx, |r| Store::Remove(k, r))
}

/// a store in memory
pub struct InMemory<K, V>(RefCell<HashMap<K, V>>);
impl<K: Eq + Hash, V> InMemory<K, V> {
    pub fn new() -> Self {
        InMemory(RefCell::new(HashMap::new()))
    }
    pub fn of(kv: impl IntoIterator<Item = (K, V)>) -> Self {
        InMemory(RefCell::new(kv.into_iter().collect()))
    }
    /// what it holds now
    pub fn into_map(self) -> HashMap<K, V> {
        self.0.into_inner()
    }
}
impl<K: Eq + Hash, V> Default for InMemory<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K, V> Handles<Store<K, V>> for InMemory<K, V>
where
    K: Clone + Debug + Eq + Hash + 'static,
    V: Clone + Debug + 'static,
{
    fn handle(&self, op: Store<K, V>) -> Step<'_> {
        match op {
            Store::Load(k, r) => Step::Now(r.give(self.0.borrow().get(&k).cloned())),
            Store::Save(k, v, r) => {
                self.0.borrow_mut().insert(k, v);
                Step::Now(r.give(()))
            }
            Store::Remove(k, r) => Step::Now(r.give(self.0.borrow_mut().remove(&k))),
        }
    }
}
