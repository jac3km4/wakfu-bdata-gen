use std::borrow::Cow;
use std::fmt;

use heck::{ToPascalCase, ToSnakeCase};

use crate::structure::{FieldType, Member, Structure, Substructure};

pub struct RustExport<'a>(&'a Structure);

impl<'a> RustExport<'a> {
    pub fn new(structure: &'a Structure) -> Self {
        Self(structure)
    }
}

impl fmt::Display for RustExport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "use std::io;")?;
        writeln!(f)?;
        writeln!(f, "use serde::Serialize;")?;
        writeln!(f)?;
        writeln!(f, "use crate::data::BinaryData;")?;
        writeln!(f, "use crate::decode::{{Decode, DecodeState}};")?;

        let name = self.0.name.to_pascal_case();
        writeln!(f)?;
        writeln!(
            f,
            "{}",
            RustStruct {
                name: &name,
                members: &self.0.members
            }
        )?;

        writeln!(f, "impl BinaryData for {name} {{")?;
        writeln!(f, "    const TYPE_ID: i16 = {};", self.0.ordinal)?;
        writeln!(f, "}}")?;
        writeln!(f)?;

        let mut substructs = vec![];
        for member in &self.0.members {
            member
                .typ
                .collect_structs(&member.name, &name, &mut substructs);
        }

        for substruct in substructs {
            writeln!(
                f,
                "{}",
                RustStruct {
                    name: &substruct.name,
                    members: substruct.members
                }
            )?;
        }

        Ok(())
    }
}

struct RustStruct<'a> {
    name: &'a str,
    members: &'a [Member],
}

impl fmt::Display for RustStruct<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "#[derive(Debug, Clone, Serialize)]")?;
        writeln!(f, "pub struct {} {{", self.name)?;
        for member in self.members {
            let name = sanitized_field_name(member);
            writeln!(
                f,
                "    pub {name}: {},",
                RustType::new(&name, &member.typ, self.name)
            )?;
        }
        writeln!(f, "}}")?;
        writeln!(f)?;

        writeln!(f, "impl Decode for {} {{", self.name)?;
        writeln!(
            f,
            "    fn decode<R: io::Read>(state: &mut DecodeState<R>) -> io::Result<Self> {{"
        )?;
        for member in self.members {
            let name = sanitized_field_name(member);
            writeln!(f, "        let {name} = state.decode()?;")?;
        }
        write!(f, "        Ok(Self {{ ")?;
        for member in self.members {
            let name = sanitized_field_name(member);
            write!(f, "{name}, ")?;
        }
        writeln!(f, "}})")?;
        writeln!(f, "    }}")?;
        writeln!(f, "}}")?;

        Ok(())
    }
}

struct RustType<'a> {
    field: &'a str,
    typ: &'a FieldType,
    parent: &'a str,
}

impl<'a> RustType<'a> {
    fn new(field: &'a str, typ: &'a FieldType, parent: &'a str) -> Self {
        Self { field, typ, parent }
    }
}

impl fmt::Display for RustType<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { field, typ, parent } = self;
        match typ {
            FieldType::Bool => write!(f, "bool"),
            FieldType::Int8 => write!(f, "i8"),
            FieldType::Int16 => write!(f, "i16"),
            FieldType::Int32 => write!(f, "i32"),
            FieldType::Int64 | FieldType::Timestamp => write!(f, "i64"),
            FieldType::Float32 => write!(f, "f32"),
            FieldType::Float64 => write!(f, "f64"),
            FieldType::String => write!(f, "String"),
            FieldType::Vec(elem) => {
                write!(f, "Vec<{}>", Self::new(field, elem, parent))
            }
            FieldType::Map(key, val) => {
                write!(
                    f,
                    "std::collections::HashMap<{}, {}>",
                    Self::new(field, key, parent),
                    Self::new(field, val, parent)
                )
            }
            FieldType::Struct(members) => {
                write!(f, "{}", Substructure::new(field, parent, members).name)
            }
        }
    }
}

fn sanitized_field_name(member: &Member) -> Cow<'_, str> {
    if member.name == "type" {
        "type_".into()
    } else if member.name.starts_with('_') {
        Cow::Borrowed(&member.name)
    } else {
        member.name.to_snake_case().into()
    }
}
