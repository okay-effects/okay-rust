package okay.foreign

import java.nio.file.{Files, Path}
import okay.codec.Schema
import okay.std.Reader

/** okay-rust (Rust effects on async/await, its own capabilities and handlers)
 * over okay's worker wire: each side sees only operations and answers */
object TestOkayRustWire:
  final case class Reserve(item: String) derives Schema
  final case class Charge(user: String, cents: Long) derives Schema
  final case class Email(to: String, text: String) derives Schema
  final case class Receipt(paid: Boolean, detail: String) derives Schema
  final case class Store(stock: Set[String], approve: Boolean)

  val sent = scala.collection.mutable.ArrayBuffer[String]()
  val reserve = Foreign.callback[Reserve, Boolean]("reserve")(r => Reader.ask[Store].map(_.stock(r.item)))
  val charge = Foreign.callback[Charge, Receipt]("charge")(c =>
    Reader.ask[Store].map(s => if s.approve then Receipt(true, s"scala-${c.user}-${c.cents}") else Receipt(false, "declined")))
  val email = Foreign.callback[Email, Boolean]("email")(e => Reader.ask[Store].map { _ => sent += s"${e.to}: ${e.text}"; true })

  val okayRust = "/Users/sergiy/work/my/okay-rust"

  val main: String = """use okay::{function, okay_call, Functions, Op, Programs, Value, Wire, Worker};
use okay_effects::{drive, start, wire, Data};
use okay_shop::{remote, Charge, Receipt};

// okay's wire values and okay-effects' plain values, one for one
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
    let mut functions = Functions::new();
    // the Rust shop, unchanged, handed out: its capabilities answered by okay, step by step
    functions.insert("checkout".into(), function(|args| {
        let (user, item, cents) = (String::from_value(&args[0])?, String::from_value(&args[1])?, i64::from_value(&args[2])?);
        let run = start(move |h| async move { remote(&h).checkout(&user, &item, cents).await });
        let r = drive(run, wire::remote(|name: &str, args: Vec<Data>| {
            okay_call(Op::<Value>::new(name, args.iter().map(value).collect())).map(|v| data(&v)).map_err(|e| e.to_string())
        }))?;
        Ok(Value::Str(format!("{} {}", r.paid, r.detail)))
    }));
    // a Rust handler, served to okay by its operation's name
    let (name, charge) = wire::serve(|op: Charge| Receipt { paid: op.cents <= 5000, detail: format!("rust-{}-{}", op.user, op.cents) });
    functions.insert(name, function(move |args| charge(args.iter().map(data).collect()).map(|d| value(&d))));
    Worker::new(Programs::new(), functions)
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
      s"okay-effects = { path = \"$okayRust\" }\nokay-shop = { path = \"$okayRust/examples/shop\" }\n"): Unit
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
}
