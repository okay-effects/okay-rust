//! Operations as data: a name and a record, answers as plain values — what
//! middleware, recording, replay and the wire see.

/// A plain value: what crosses a wire, what a recording keeps.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Data>),
    Dict(Vec<(String, Data)>),
}

impl Data {
    /// a field of a record
    pub fn field(&self, name: &str) -> Result<&Data, String> {
        match self {
            Data::Dict(kv) => kv.iter().find(|(k, _)| k == name).map(|(_, v)| v).ok_or_else(|| format!("no field {name} in {self:?}")),
            other => Err(format!("expected a record with {name}, got {other:?}")),
        }
    }
}

/// A Rust type as a plain value, and back.
pub trait IsData: Sized {
    fn to_data(&self) -> Data;
    fn from_data(d: &Data) -> Result<Self, String>;
}

impl IsData for Data {
    fn to_data(&self) -> Data { self.clone() }
    fn from_data(d: &Data) -> Result<Self, String> { Ok(d.clone()) }
}
impl IsData for () {
    fn to_data(&self) -> Data { Data::Null }
    fn from_data(_: &Data) -> Result<Self, String> { Ok(()) } // nothing to answer: whatever came says "done"
}
impl IsData for bool {
    fn to_data(&self) -> Data { Data::Bool(*self) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d { Data::Bool(b) => Ok(*b), o => Err(format!("expected a boolean, got {o:?}")) }
    }
}
impl IsData for i64 {
    fn to_data(&self) -> Data { Data::Int(*self) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d {
            Data::Int(n) => Ok(*n),
            Data::Float(f) if f.fract() == 0.0 => Ok(*f as i64),
            o => Err(format!("expected an integer, got {o:?}")),
        }
    }
}
impl IsData for f64 {
    fn to_data(&self) -> Data { Data::Float(*self) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d { Data::Float(f) => Ok(*f), Data::Int(n) => Ok(*n as f64), o => Err(format!("expected a number, got {o:?}")) }
    }
}
impl IsData for String {
    fn to_data(&self) -> Data { Data::Str(self.clone()) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d { Data::Str(s) => Ok(s.clone()), o => Err(format!("expected a string, got {o:?}")) }
    }
}
impl<T: IsData> IsData for Vec<T> {
    fn to_data(&self) -> Data { Data::List(self.iter().map(IsData::to_data).collect()) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d { Data::List(xs) => xs.iter().map(T::from_data).collect(), o => Err(format!("expected a list, got {o:?}")) }
    }
}
impl<T: IsData> IsData for Option<T> {
    fn to_data(&self) -> Data { self.as_ref().map(IsData::to_data).unwrap_or(Data::Null) }
    fn from_data(d: &Data) -> Result<Self, String> {
        match d { Data::Null => Ok(None), o => T::from_data(o).map(Some) }
    }
}

/// An operation that can be seen as data: a name and a record, its answer a plain value.
/// `okay_effects::op!` writes one.
pub trait Operation: crate::Op<Answer: IsData> + IsData + std::fmt::Debug {
    /// its name on the wire: the struct's name in snake_case
    fn name() -> String;
}

/// `user_id` as `userId`: a Rust field as a Scala case class's
pub fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut up = false;
    for c in s.chars() {
        if c == '_' { up = true } else if up { out.extend(c.to_uppercase()); up = false } else { out.push(c) }
    }
    out
}

/// `PriceOf` as `price_of`
pub fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 { out.push('_') }
            out.extend(c.to_lowercase());
        } else {
            out.push(c)
        }
    }
    out
}

/// A record: a struct whose fields are plain values, as data.
#[macro_export]
macro_rules! data {
    ($(#[$m:meta])* $vis:vis struct $name:ident { $($fvis:vis $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        $vis struct $name { $($fvis $field: $ty),* }
        impl $crate::IsData for $name {
            fn to_data(&self) -> $crate::Data {
                $crate::Data::Dict(vec![$(($crate::camel(stringify!($field)), $crate::IsData::to_data(&self.$field))),*])
            }
            #[allow(unused_variables)]
            fn from_data(d: &$crate::Data) -> Result<Self, String> {
                Ok($name { $($field: $crate::IsData::from_data(d.field(&$crate::camel(stringify!($field)))?)?),* })
            }
        }
    };
}

/// An operation: a record, what it answers, its name.
/// `op!(pub struct Charge { pub user: String, pub cents: i64 } -> Receipt);`
#[macro_export]
macro_rules! op {
    ($(#[$m:meta])* $vis:vis struct $name:ident { $($fvis:vis $field:ident : $ty:ty),* $(,)? } -> $ans:ty) => {
        $crate::data!($(#[$m])* $vis struct $name { $($fvis $field : $ty),* });
        impl $crate::Op for $name { type Answer = $ans; }
        impl $crate::Operation for $name { fn name() -> String { $crate::snake(stringify!($name)) } }
    };
}
