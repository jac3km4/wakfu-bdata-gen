use std::borrow::Cow;

use heck::ToPascalCase;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct Structure {
    pub name: String,
    pub obfuscated_name: String,
    pub ordinal: i16,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub typ: FieldType,
}

impl Member {
    pub fn new_anonymous(i: usize, typ: FieldType) -> Self {
        Self {
            name: format!("_{i}"),
            typ,
        }
    }
}

impl PartialEq for Member {
    fn eq(&self, other: &Self) -> bool {
        self.typ == other.typ
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldType {
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "i8")]
    Int8,
    #[serde(rename = "i16")]
    Int16,
    #[serde(rename = "i32")]
    Int32,
    #[serde(rename = "i64")]
    Int64,
    #[serde(rename = "f32")]
    Float32,
    #[serde(rename = "f64")]
    Float64,
    Timestamp,
    String,
    Vec(Box<Self>),
    Map(Box<Self>, Box<Self>),
    #[serde(with = "serde_members_as_map")]
    Struct(Vec<Member>),
}

impl FieldType {
    pub fn collect_structs<'a>(
        &'a self,
        field: &str,
        parent: &str,
        acc: &mut Vec<Substructure<'a>>,
    ) {
        match self {
            Self::Vec(elem) => {
                elem.collect_structs(field, parent, acc);
            }
            Self::Map(key, val) => {
                key.collect_structs(field, parent, acc);
                val.collect_structs(field, parent, acc);
            }
            Self::Struct(members) => {
                let substruct = Substructure::new(field, parent, members);
                for member in members {
                    member
                        .typ
                        .collect_structs(&member.name, &substruct.name, acc);
                }
                acc.push(substruct);
            }
            _ => {}
        }
    }
}

impl PartialEq for FieldType {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bool, Self::Bool)
            | (Self::Int8, Self::Int8)
            | (Self::Int16, Self::Int16)
            | (Self::Int32, Self::Int32)
            | (Self::Int64, Self::Int64)
            | (Self::Float32, Self::Float32)
            | (Self::Float64, Self::Float64)
            | (Self::Timestamp, Self::Timestamp)
            | (Self::String, Self::String) => true,
            (Self::Vec(a), Self::Vec(b)) => a == b,
            (Self::Map(ka, va), Self::Map(kb, vb)) => ka == kb && va == vb,
            (Self::Struct(a), Self::Struct(b)) => {
                a.iter().map(|m| &m.typ).eq(b.iter().map(|m| &m.typ))
            }
            _ => false,
        }
    }
}

pub struct Substructure<'a> {
    pub name: String,
    pub members: &'a [Member],
}

impl<'a> Substructure<'a> {
    pub fn new(field_name: &str, parent_name: &str, members: &'a [Member]) -> Self {
        Self {
            name: format!(
                "{}{}",
                parent_name,
                if field_name.starts_with("_") {
                    Cow::Borrowed(field_name)
                } else {
                    field_name.to_pascal_case().into()
                }
            ),
            members,
        }
    }
}

pub mod serde_members_as_map {
    use std::fmt;

    use serde::de::{MapAccess, Visitor};
    use serde::{Deserializer, Serializer};

    use super::{FieldType, Member};

    pub fn serialize<S>(members: &[Member], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_map(members.iter().map(|m| (m.name.clone(), &m.typ)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Member>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MemberVisitor;

        impl<'de> Visitor<'de> for MemberVisitor {
            type Value = Vec<Member>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map of string keys to field types")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut members = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some((name, typ)) = map.next_entry::<String, FieldType>()? {
                    members.push(Member { name, typ });
                }
                Ok(members)
            }
        }

        deserializer.deserialize_map(MemberVisitor)
    }
}
