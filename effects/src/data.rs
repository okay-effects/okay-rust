//! Operations as data: a name and a record, answers as plain values — what
//! middleware, recording, replay and the wire see.

pub use okay::data::{camel, snake, Data, IsData};

/// An operation that can be seen as data: a name and a record, its answer a plain value.
/// `okay_effects::op!` writes one.
pub trait Operation: crate::Op<Answer: IsData> + IsData + std::fmt::Debug {
    /// its name on the wire: the struct's name in snake_case
    fn name() -> String;
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
