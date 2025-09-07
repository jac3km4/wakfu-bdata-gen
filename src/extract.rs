use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::{fs, io};

use anyhow::Context;
use heck::ToShoutySnakeCase;
use include_dir::{Dir, include_dir};
use itertools::Itertools;
use noak::descriptor::{BaseType, MethodDescriptor, TypeDescriptor};
use noak::reader::attributes::{Code, RawInstruction, Signature};
use noak::reader::cpool::value::FieldRef;
use noak::reader::cpool::{self};
use noak::reader::{Class, Field, Method};
use noak::{AccessFlags, MStr, MString};
use with_locals::with;

use crate::structure::{FieldType, Member, Structure, serde_members_as_map};

const ASSETS: Dir<'static> = include_dir!("./assets");

pub fn extract(game_root: &Path) -> anyhow::Result<Vec<Structure>> {
    let loader = ClassLoader::open(&game_root.join("lib").join("wakfu-client.jar"))?;

    let mut original_structs = load_original_structs()?;
    let mut structures = extract_obfuscated_structs(&loader)?;

    for structure in &mut structures {
        if let Some(origin) = original_structs.remove(&structure.name) {
            structure.members = diff::slice(&structure.members, &origin)
                .into_iter()
                .filter_map(|r| match r {
                    diff::Result::Left(l) => Some(l),
                    diff::Result::Both(_, r) => Some(r),
                    diff::Result::Right(_) => None,
                })
                .cloned()
                .collect();
        }
    }

    Ok(structures)
}

#[derive(Debug)]
pub struct ClassLoader {
    jar: RefCell<zip::ZipArchive<fs::File>>,
}

impl ClassLoader {
    #[with('local)]
    pub fn class(&self, path: &str) -> anyhow::Result<Class<'local>> {
        let mut buf = Vec::new();
        {
            let mut zip = self.jar.borrow_mut();
            let mut file = zip.by_name(&format!("{path}.class"))?;
            io::copy(&mut file, &mut buf)?;
        }
        Ok(noak::reader::Class::new(&buf)?)
    }

    pub fn class_names(&self) -> Vec<String> {
        let zip = self.jar.borrow();
        zip.file_names()
            .filter_map(|name| name.strip_suffix(".class"))
            .map(str::to_owned)
            .collect()
    }
}

impl ClassLoader {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            jar: RefCell::new(zip::ZipArchive::new(fs::File::open(path)?)?),
        })
    }
}

#[derive(Debug)]
#[allow(unused)]
struct BinaryDataClass<'i> {
    class: Class<'i>,
    read_method: Method<'i>,
    reset_method: Method<'i>,
    type_id_method: Method<'i>,
}

impl<'i> BinaryDataClass<'i> {
    fn from_class(class: Class<'i>, expect_final: bool) -> anyhow::Result<Option<Self>> {
        fn is_read_method(desc: &MethodDescriptor<'_>) -> bool {
            desc.return_type().is_none()
                && desc
                    .parameters()
                    .exactly_one()
                    .is_ok_and(|p| p.dimensions == 0 && matches!(p.base, BaseType::Object(_)))
        }

        fn is_reset_method(desc: &MethodDescriptor<'_>) -> bool {
            desc.return_type().is_none() && desc.parameters().count() == 0
        }

        fn is_type_id_method(desc: &MethodDescriptor<'_>) -> bool {
            desc.return_type()
                .is_some_and(|r| r.dimensions == 0 && matches!(r.base, BaseType::Integer))
                && desc.parameters().count() == 0
        }

        let methods = class
            .methods()
            .iter()
            .map(|method| {
                let method = method?;
                let desc = MethodDescriptor::parse(class.pool().retrieve(method.descriptor())?)?;
                Ok::<_, anyhow::Error>((desc, method))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if let Some((_, read_method)) = methods.iter().find(|(desc, _)| is_read_method(desc))
            && let Some((_, reset_method)) = methods.iter().find(|(desc, _)| is_reset_method(desc))
            && let Some((_, type_id_method)) = methods.iter().find(|(desc, m)| {
                is_type_id_method(desc)
                    && (!expect_final || m.access_flags().contains(AccessFlags::FINAL))
            })
        {
            Ok(Some(Self {
                class,
                read_method: read_method.clone(),
                reset_method: reset_method.clone(),
                type_id_method: type_id_method.clone(),
            }))
        } else {
            Ok(None)
        }
    }

    fn get_data_type_id(&self) -> anyhow::Result<Option<FieldRef<'i>>> {
        let Some(code) = self
            .type_id_method
            .attributes()
            .find_attribute::<Code<'_>>(self.class.pool())?
        else {
            return Ok(None);
        };

        if let Some((_, RawInstruction::GetStatic { index })) =
            code.raw_instructions().next().transpose()?
        {
            Ok(Some(self.class.pool().retrieve(index)?))
        } else {
            Ok(None)
        }
    }

    fn get_data_fields(&self, loader: &ClassLoader) -> anyhow::Result<Vec<Member>> {
        extract_class_fields(&self.class, loader)
    }
}

#[derive(Debug)]
struct BinaryDataEnumMember {
    obfuscated_name: MString,
    name: MString,
    ordinal: i16,
}

fn load_original_structs() -> Result<HashMap<String, Vec<Member>>, anyhow::Error> {
    let mut original_structs = HashMap::new();
    for file in ASSETS
        .get_dir("structures")
        .expect("missing structures assets dir")
        .files()
    {
        let mut deserializer = ron::de::Deserializer::from_bytes(file.contents())?;
        let fields = serde_members_as_map::deserialize(&mut deserializer)?;
        let name = file
            .path()
            .file_stem()
            .context("file has no name")?
            .to_string_lossy()
            .to_shouty_snake_case();
        original_structs.insert(name, fields);
    }
    Ok(original_structs)
}

#[with]
fn extract_bdata_interface(
    classes: &[String],
    loader: &ClassLoader,
) -> anyhow::Result<Option<MString>> {
    let mut bdata_iface = None;
    for cls in classes {
        #[with]
        let cls = loader.class(cls)?;
        let does_match = cls.access_flags().contains(AccessFlags::INTERFACE)
            && cls.methods().into_iter().count() == 3;
        if !does_match {
            continue;
        }
        if let Some(cls) = BinaryDataClass::from_class(cls, false)? {
            let this = cls.class.this_class();
            bdata_iface = Some(cls.class.pool().retrieve(this)?.name.to_owned());
            break;
        }
    }
    Ok(bdata_iface)
}

#[with]
fn extract_obfuscated_structs(loader: &ClassLoader) -> anyhow::Result<Vec<Structure>> {
    let classes = loader.class_names();

    let bdata_iface = extract_bdata_interface(&classes, loader)?
        .context("could not find the binary data interface")?;

    let mut structures = vec![];
    let mut obfuscated_enum_map = Option::<HashMap<MString, BinaryDataEnumMember>>::None;
    for cls in &classes {
        #[with]
        let cls = loader.class(cls)?;
        if let Some(enum_ref) = extract_struct(cls, &bdata_iface, &mut obfuscated_enum_map, loader)?
        {
            structures.push(enum_ref);
        } else {
            continue;
        }
    }

    // Remove classes that are not referenced by any other class.
    let mut used = structures
        .iter()
        .map(|s| (s.obfuscated_name.as_bytes().to_vec(), false))
        .collect::<HashMap<_, _>>();
    for cls in classes {
        #[with]
        let cls = loader.class(&cls)?;

        for c in cls
            .pool()
            .iter()
            .filter_map(<cpool::Class<'_> as cpool::TryFromItem>::try_from_item)
            .filter(|c| cls.pool().get(cls.this_class()).ok() != Some(c))
        {
            let name = cls.pool().retrieve(c.name).unwrap();
            if let Some(used) = used.get_mut(name.as_bytes()) {
                *used = true;
            }
        }
    }
    structures.retain(|s| {
        used.get(s.obfuscated_name.as_bytes())
            .copied()
            .unwrap_or(false)
    });

    // Remove duplicates by ordinal, keeping the first one.
    let mut ids = HashSet::new();
    structures.retain(|s| ids.insert(s.ordinal));

    Ok(structures)
}

#[with]
fn extract_struct(
    cls: Class<'_>,
    bdata_iface: &MStr,
    obfuscated_enum_map: &mut Option<HashMap<MString, BinaryDataEnumMember>>,
    loader: &ClassLoader,
) -> anyhow::Result<Option<Structure>> {
    if !cls
        .interfaces()
        .into_iter()
        .map(|i| Ok::<_, anyhow::Error>(cls.pool().retrieve(i?)?.name))
        .any(|c| c.ok() == Some(bdata_iface))
    {
        return Ok(None);
    }
    let Some(bdata) = BinaryDataClass::from_class(cls, true)? else {
        return Ok(None);
    };
    let Some(enum_ref) = bdata.get_data_type_id()? else {
        return Ok(None);
    };

    if obfuscated_enum_map.is_none() {
        let cls_name = enum_ref
            .class
            .name
            .to_str()
            .context("invalid UTF-8 in enum class name")?;
        #[with]
        let enum_class = loader.class(cls_name)?;
        let map = extract_enum_values(&enum_class)?
            .into_iter()
            .map(|m| (m.obfuscated_name.clone(), m))
            .collect();
        *obfuscated_enum_map = Some(map);
    }

    let res = if let Some(BinaryDataEnumMember { name, ordinal, .. }) = obfuscated_enum_map
        .as_ref()
        .and_then(|map| map.get(enum_ref.name_and_type.name))
    {
        Some(Structure {
            name: name
                .to_str()
                .expect("invalid UTF-8 in structure name")
                .to_owned(),
            obfuscated_name: bdata
                .class
                .pool()
                .retrieve(bdata.class.this_class())?
                .name
                .to_str()
                .expect("invalid UTF-8 in obfuscated name")
                .to_owned(),
            ordinal: *ordinal,
            members: bdata.get_data_fields(loader)?,
        })
    } else {
        None
    };

    Ok(res)
}

fn extract_class_fields(class: &Class<'_>, loader: &ClassLoader) -> anyhow::Result<Vec<Member>> {
    class
        .fields()
        .into_iter()
        .enumerate()
        .map(|(i, r)| Ok((i, r?)))
        .filter_ok(|(_, f)| f.access_flags().contains(AccessFlags::PROTECTED))
        .map_ok(|(i, f)| {
            let typ = parse_field(&f, class.pool(), loader)?;
            Ok::<_, anyhow::Error>(Member::new_anonymous(i, typ))
        })
        .flatten_ok()
        .collect()
}

fn extract_enum_values(class: &Class<'_>) -> anyhow::Result<Vec<BinaryDataEnumMember>> {
    let ctr = class
        .methods()
        .into_iter()
        .map_ok(|m| Ok::<_, anyhow::Error>((class.pool().retrieve(m.name())?, m)))
        .flatten_ok()
        .filter_ok(|(n, _)| n.as_bytes() == b"<clinit>")
        .map_ok(|(_, m)| m)
        .next()
        .context("could not find <clinit> method for the binary data enum")??;

    let code = ctr
        .attributes()
        .find_attribute::<Code<'_>>(class.pool())?
        .context("could not find code attribute for <clinit> method")?
        .raw_instructions()
        .map_ok(|(_, i)| i)
        .collect::<Result<Vec<_>, _>>()?;

    let mut slice = &code[..];
    let mut members = vec![];

    while let [
        RawInstruction::New { .. },
        RawInstruction::Dup,
        RawInstruction::LdC { index: name_index },
        _,
        value,
        RawInstruction::InvokeSpecial { .. },
        RawInstruction::PutStatic { index },
        rest @ ..,
    ] = slice
    {
        let ordinal = match value {
            RawInstruction::IConst0 => 0,
            RawInstruction::IConst1 => 1,
            RawInstruction::IConst2 => 2,
            RawInstruction::IConst3 => 3,
            RawInstruction::IConst4 => 4,
            RawInstruction::IConst5 => 5,
            RawInstruction::BIPush { value } => *value as i16,
            RawInstruction::SIPush { value } => *value,
            _ => anyhow::bail!("unexpected enum ordinal instruction"),
        };

        let field = class.pool().retrieve(*index)?;
        let cpool::Item::String(str) = class.pool().get(*name_index)? else {
            anyhow::bail!("unexpected enum name value");
        };
        let obfuscated_name = field.name_and_type.name.to_owned();
        let name = class.pool().retrieve(str.string)?.to_owned();
        members.push(BinaryDataEnumMember {
            obfuscated_name,
            name,
            ordinal,
        });

        slice = rest;
    }

    Ok(members)
}

fn parse_field<'i>(
    field: &Field<'i>,
    pool: &cpool::ConstantPool<'i>,
    loader: &ClassLoader,
) -> Result<FieldType, anyhow::Error> {
    let desc = TypeDescriptor::parse(pool.retrieve(field.descriptor())?)?;

    let typ = match desc.base {
        BaseType::Boolean => FieldType::Bool,
        BaseType::Byte | BaseType::Char => FieldType::Int8,
        BaseType::Short => FieldType::Int16,
        BaseType::Integer => FieldType::Int32,
        BaseType::Long => FieldType::Int64,
        BaseType::Float => FieldType::Float32,
        BaseType::Double => FieldType::Float64,
        BaseType::Object(obj) => {
            if let Some(sig) = field
                .attributes()
                .find_attribute::<Signature<'_>>(pool)?
                .map(|sig| pool.retrieve(sig.signature()))
                .transpose()?
            {
                let signature = sig.to_str().context("invalid UTF-8 in signature")?;
                parse_field_signature(signature, loader)?
            } else {
                let obj = obj.to_str().context("invalid UTF-8 in object name")?;
                parse_object_signature(obj, loader)?
            }
        }
    };

    let mut typ = typ;
    for _ in 0..desc.dimensions {
        typ = FieldType::Vec(Box::new(typ));
    }
    Ok(typ)
}

fn parse_field_signature(
    signature: &str,
    loader: &ClassLoader,
) -> Result<FieldType, anyhow::Error> {
    match signature {
        "Z" => Ok(FieldType::Bool),
        "B" => Ok(FieldType::Int8),
        "S" => Ok(FieldType::Int16),
        "I" => Ok(FieldType::Int32),
        "J" => Ok(FieldType::Int64),
        "F" => Ok(FieldType::Float32),
        "D" => Ok(FieldType::Float64),
        _ => {
            if let Some(object) = signature.strip_prefix('L') {
                parse_object_signature(object.strip_suffix(';').unwrap_or(object), loader)
            } else if let Some(inner) = signature.strip_prefix('[') {
                let elem = parse_field_signature(inner, loader)?;
                Ok(FieldType::Vec(Box::new(elem)))
            } else {
                anyhow::bail!("unknown signature: {}", signature);
            }
        }
    }
}

#[with]
fn parse_object_signature(name: &str, loader: &ClassLoader) -> Result<FieldType, anyhow::Error> {
    match name {
        "java/lang/Boolean" => Ok(FieldType::Bool),
        "java/lang/Byte" => Ok(FieldType::Int8),
        "java/lang/Short" => Ok(FieldType::Int16),
        "java/lang/Integer" => Ok(FieldType::Int32),
        "java/lang/Long" => Ok(FieldType::Int64),
        "java/lang/Float" => Ok(FieldType::Float32),
        "java/lang/Double" => Ok(FieldType::Float64),
        "java/lang/String" => Ok(FieldType::String),
        "java/sql/Timestamp" => Ok(FieldType::Timestamp),
        _ => {
            if let Some(("java/util/HashMap", args)) = name.split_once('<') {
                let (k, v) = args
                    .strip_suffix('>')
                    .context("invalid map signature")?
                    .split_inclusive(';')
                    .collect_tuple()
                    .context("invalid map signature")?;
                let k = parse_field_signature(k, loader)?;
                let v = parse_field_signature(v, loader)?;
                Ok(FieldType::Map(Box::new(k), Box::new(v)))
            } else {
                #[with]
                let class = loader.class(name)?;
                Ok(FieldType::Struct(extract_class_fields(&class, loader)?))
            }
        }
    }
}
