package okay.foreign

import java.nio.file.{Files, Path}
import okay.codec.Schema
import okay.effect
import okay.std.{Choose, Reader, runChoice}

/** okay-rust (Rust effects on async/await: the one API, okay::typed and okay::host)
 * over okay's worker wire: each side sees only operations and answers */
object TestOkayRustWire:
  final case class Reserve(item: String) derives Schema
  final case class Charge(user: String, cents: Long) derives Schema
  final case class Email(to: String, text: String) derives Schema
  final case class Receipt(paid: Boolean, detail: String) derives Schema
  final case class Store(stock: Set[String], approve: Boolean)
  final case class Among(n: Long) derives Schema
  final case class PriceOf(sku: String) derives Schema
  final case class Discount(amount: Double) derives Schema

  // the kernel's programs (okay::Eff, programs as data): their operations answered by okay's own effects
  val among = Foreign.callback[Among, Long]("among")(a => effect[Choose, Long](Choose((0L until a.n).toVector)))
  val priceOf = Foreign.callback[PriceOf, Double]("price_of")(p => Reader.ask[Map[String, Double]].map(_(p.sku)))
  val discount = Foreign.callback[Discount, Double]("discount")(d => Reader.ask[Map[String, Double]].map(m => d.amount * m("rate")))

  val sent = scala.collection.mutable.ArrayBuffer[String]()
  val reserve = Foreign.callback[Reserve, Boolean]("reserve")(r => Reader.ask[Store].map(_.stock(r.item)))
  val charge = Foreign.callback[Charge, Receipt]("charge")(c =>
    Reader.ask[Store].map(s => if s.approve then Receipt(true, s"scala-${c.user}-${c.cents}") else Receipt(false, "declined")))
  val email = Foreign.callback[Email, Boolean]("email")(e => Reader.ask[Store].map { _ => sent += s"${e.to}: ${e.text}"; true })

  val okayRust = "/Users/sergiy/work/my/okay-rust"

  val main: String = """use okay::{function, okay_call, Functions, Op, Programs, Value, Wire, Worker};
use okay_kernel::host::{drive, remote, start};
use okay_kernel::typed::{Audited, Cx, End};
use okay_kernel::Data;
use okay_shop::{checkout, Allow, Inventory, Notify, Payments, Receipt};
use okay_kernel::effects::{among, Choose};
use okay_kernel::wire::{program, Remotes, Wire as KWire};
use okay_kernel::{eff, row, Eff};
use std::rc::Rc;

okay_kernel::effect! {
    /// a price list: a program of the kernel over the wire
    pub enum Prices: Plain, wire {
        PriceOf(sku: String) -> f64 = price_of;
        Discount(amount: f64) -> f64 = discount;
    }
}

// the kernel's plain values and okay's wire values
fn kdata(v: &Value) -> okay_kernel::Data {
    use okay_kernel::Data as D;
    match v {
        Value::Null | Value::Table(_) => D::Null,
        Value::Bool(b) => D::Bool(*b),
        Value::Int(n) => D::Int(*n),
        Value::Float(f) => D::Float(*f),
        Value::Str(s) => D::Str(s.clone()),
        Value::List(xs) => D::List(xs.iter().map(kdata).collect()),
        Value::Dict(kv) => D::Dict(kv.iter().map(|(k, v)| (k.clone(), kdata(v))).collect()),
    }
}
fn kvalue(d: &okay_kernel::Data) -> Value {
    use okay_kernel::Data as D;
    match d {
        D::Null => Value::Null,
        D::Bool(b) => Value::Bool(*b),
        D::Int(n) => Value::Int(*n),
        D::Float(f) => Value::Float(*f),
        D::Str(s) => Value::Str(s.clone()),
        D::List(xs) => Value::List(xs.iter().map(kvalue).collect()),
        D::Dict(kv) => Value::Dict(kv.iter().map(|(k, v)| (k.clone(), kvalue(v))).collect()),
    }
}

/// a program of the kernel on the wire as okay's program as data: each rest called as many times as okay resumes it
fn prog(w: KWire) -> okay::Prog {
    match w {
        KWire::Done(d) => okay::Prog::Done(kvalue(&d)),
        KWire::Failed(why) => panic!("{why}"),
        KWire::Perform { name, args, k } => okay::Prog::Perform {
            name,
            args: args.iter().map(kvalue).collect(),
            k: Rc::new(move |v| prog(k(Ok(kdata(&v))))),
        },
    }
}

// okay's wire values and the shop's plain values, one for one
fn data(v: &Value) -> Data {
    match v {
        Value::Null => Data::Null,
        Value::Bool(b) => Data::Bool(*b),
        Value::Int(n) => Data::Int(*n),
        Value::Float(f) => Data::Float(*f),
        Value::Str(s) => Data::Str(s.clone()),
        Value::List(xs) => Data::List(xs.iter().map(data).collect()),
        Value::Dict(kv) => Data::Dict(kv.iter().map(|(k, v)| (k.clone(), data(v))).collect()),
        Value::Table(_) => Data::Null,
    }
}
fn value(d: &Data) -> Value {
    match d {
        Data::Null => Value::Null,
        Data::Bool(b) => Value::Bool(*b),
        Data::Int(n) => Value::Int(*n),
        Data::Float(f) => Value::Float(*f),
        Data::Str(s) => Value::Str(s.clone()),
        Data::List(xs) => Value::List(xs.iter().map(value).collect()),
        Data::Dict(kv) => Value::Dict(kv.iter().map(|(k, v)| (k.clone(), value(v))).collect()),
    }
}

fn make() -> Worker {
    let mut programs = Programs::new();
    // MULTI-SHOT across the wire: okay's Choice handler resumes the kernel's rest for every branch
    programs.insert("pairs".into(), Box::new(|_| {
        let p: Eff<row![Choose], i64> = eff! { data;
            x <- among(vec![1i64, 2]);
            y <- among(vec![10i64, 20]);
            yield x + y
        };
        prog(program(p.step(), &Remotes::new().with::<Choose>()))
    }));
    // an effect declared to cross, answered by Scala callbacks under okay's Reader
    programs.insert("total".into(), Box::new(|args| {
        let (sku, qty) = (String::from_value(&args[0]).unwrap(), i64::from_value(&args[1]).unwrap());
        let p: Eff<row![Prices], f64> = eff! { data;
            price <- Prices::price_of(sku.clone());
            Prices::discount(price * qty as f64)
        };
        prog(program(p.step(), &Remotes::new().with::<Prices>()))
    }));
    let mut functions = Functions::new();
    // a kernel handler of Prices, served: okay's operations answered in Rust
    let prices = okay_kernel::wire::serve(|op: Prices| match op {
        Prices::PriceOf(sku, r) => r.give(if sku == "tea" { 4.0 } else { 1.0 }),
        Prices::Discount(a, r) => r.give(a * 0.9),
    });
    let prices = std::sync::Arc::new(prices);
    for name in ["price_of", "discount"] {
        let prices = prices.clone();
        functions.insert(format!("rust_{name}"), function(move |args| {
            let rec = args.first().map(kdata).unwrap_or(okay_kernel::Data::Dict(vec![]));
            match prices(name, &rec) {
                Some(Ok(d)) => Ok(kvalue(&d)),
                Some(Err(e)) => Err(e),
                None => Err(format!("no {name}")),
            }
        }));
    }
    // the Rust shop, unchanged, handed out: its effects answered by okay, step by step — payments under their
    // interceptor
    functions.insert("checkout".into(), function(|args| {
        let (user, item, cents) = (String::from_value(&args[0])?, String::from_value(&args[1])?, i64::from_value(&args[2])?);
        let run = start(move |h| async move {
            let pay = Audited::new(h.outside::<Payments>(), Allow);
            let cx = Cx(h.outside::<Inventory>(), Cx(pay, Cx(h.outside::<Notify>(), End)));
            checkout(&cx, &user, &item, cents).await
        });
        let r = drive(run, remote(|name: &str, args: Vec<Data>| {
            okay_call(Op::<Value>::new(name, args.iter().map(value).collect())).map(|v| data(&v)).map_err(|e| e.to_string())
        }))?;
        Ok(Value::Str(format!("{} {}", r.paid, r.detail)))
    }));
    // a Rust handler, served to okay by its operation's name
    let charge = okay_kernel::wire::serve(|op: Payments| match op {
        Payments::Charge(user, cents, r) => {
            r.give(Receipt { paid: cents <= 5000, detail: format!("rust-{user}-{cents}") })
        }
    });
    functions.insert("charge".into(), function(move |args| {
        let rec = args.first().map(data).unwrap_or(Data::Dict(vec![]));
        match charge("charge", &rec) {
            Some(r) => r.map(|d| value(&d)),
            None => Err("no charge".into()),
        }
    }));
    Worker::new(programs, functions)
}

fn main() {
    okay::main(make)
}
"""

class TestOkayRustWire extends munit.FunSuite {
  import TestOkayRustWire.*

  override def munitTests(): Seq[Test] = super.munitTests().map(_.tag(new munit.Tag("Live")))
  override def munitIgnore: Boolean = !RustWorkerBinary.available

  private lazy val w =
    val dir = Files.createTempDirectory("okay-rust-wire")
    Files.createDirectories(dir.resolve("src")): Unit
    Files.writeString(dir.resolve("Cargo.toml"),
      s"[package]\nname = \"worker\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nokay = { path = \"okay\" }\n" +
      s"okay-shop = { path = \"$okayRust/examples/shop\" }\n" +
      s"okay-kernel = { package = \"okay-machine\", path = \"$okayRust/machine\" }\n"): Unit
    Files.writeString(dir.resolve("src").resolve("main.rs"), main): Unit
    ForeignWorker.speaking(Seq(RustWorker.build(dir).toString))
  private given okay.Answers[ForeignEval] = w.handler
  override def afterAll(): Unit = if RustWorkerBinary.available then w.close()

  private val cbs = Foreign.callbacks(reserve, charge, email)

  test("the Rust shop runs in Rust; its inventory, payments and mail are Scala handlers, step by step") {
    sent.clear()
    val paid = Foreign.fn[String]("checkout").calling(cbs)("bob", "book", 2000L)
    assertEquals(Reader.run(Store(Set("book"), approve = true))(paid).runWith, Right("true scala-bob-2000"))
    val out = Foreign.fn[String]("checkout").calling(cbs)("eve", "pen", 100L)
    assertEquals(Reader.run(Store(Set("book"), approve = true))(out).runWith, Right("false pen: out of stock"))
    assertEquals(sent.toList, List("bob: paid, receipt scala-bob-2000", "eve: not paid: pen: out of stock"))
  }

  test("the other way: a Rust handler answers okay's operation, a record in, a record out") {
    assertEquals(Foreign.fn[Receipt]("charge")(Charge("ann", 300L)).runWith, Right(Receipt(true, "rust-ann-300")))
    assertEquals(Foreign.fn[Receipt]("charge")(Charge("ann", 9000L)).runWith, Right(Receipt(false, "rust-ann-9000")))
  }

  test("MULTI-SHOT across the wire, the kernel's program: okay's Choice resumes the Rust rest for every branch") {
    val pairs = Foreign.program[Long]("pairs").calling(Foreign.callbacks(among))()
    assertEquals(runChoice(pairs.program).runWith.toList, List(Right(11L), Right(21L), Right(12L), Right(22L)))
    pairs.forget.runWith
  }

  test("the kernel's effect declared to cross: each operation a Scala callback under okay's Reader") {
    val run = Foreign.program[Double]("total").calling(Foreign.callbacks(priceOf, discount))("tea", 3L)
    assertEquals(Reader.run(Map("tea" -> 4.0, "rate" -> 0.5))(run.program).runWith, Right(6.0))
  }

  test("the other way: the kernel's handler answers okay's operation, a record in") {
    assertEquals(Foreign.fn[Double]("rust_price_of")(PriceOf("tea")).runWith, Right(4.0))
    assertEquals(Foreign.fn[Double]("rust_discount")(Discount(10.0)).runWith, Right(9.0))
  }
}
