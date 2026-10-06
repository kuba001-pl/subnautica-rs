//! Unity serialized files (`*.assets`, `level0`, `CAB-…` inside bundles):
//! a header, metadata (types, object table, external references), then the
//! object data.

use crate::reader::Reader;
use crate::{ErrorKind, Result};

/// Format versions this reader understands (Unity 2017.x – 2021.x). Game
/// build 10 uses 21 everywhere.
pub const SUPPORTED_VERSIONS: std::ops::RangeInclusive<u32> = 14..=22;

const MONO_BEHAVIOUR: i32 = 114;

/// One node of a type tree (field layout description).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeTreeNode {
    pub version: u16,
    pub level: u8,
    pub type_flags: u8,
    pub type_name: String,
    pub name: String,
    pub byte_size: i32,
    pub index: i32,
    pub meta_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedType {
    pub class_id: i32,
    pub is_stripped: bool,
    pub script_type_index: i16,
    /// Present for MonoBehaviours: identifies the script.
    pub script_id: Option<[u8; 16]>,
    pub old_type_hash: [u8; 16],
    /// Only present if the file was built with type trees.
    pub type_tree: Option<Vec<TypeTreeNode>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub path_id: i64,
    /// Absolute offset of the object's data within the file.
    pub byte_start: u64,
    pub byte_size: u32,
    pub type_index: i32,
    pub class_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    pub guid: [u8; 16],
    pub kind: i32,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerializedFile {
    pub version: u32,
    pub unity_version: String,
    pub platform: i32,
    pub big_endian: bool,
    pub type_tree_enabled: bool,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectInfo>,
    pub externals: Vec<External>,
    pub data_offset: u64,
    pub file_size: u64,
}

/// Strings in type trees: offsets with the top bit set point into Unity's
/// built-in common string table, which we don't ship; they are kept as
/// `@common+N`.
fn tree_string(buffer: &[u8], offset: u32) -> String {
    if offset & 0x8000_0000 != 0 {
        return format!("@common+{}", offset & 0x7FFF_FFFF);
    }
    let start = offset as usize;
    match buffer.get(start..) {
        Some(rest) => {
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..end]).into_owned()
        }
        None => format!("@bad+{offset}"),
    }
}

fn read_type_tree(r: &mut Reader, version: u32) -> Result<Vec<TypeTreeNode>> {
    let node_size = if version >= 19 { 32 } else { 24 };
    let count = r.count(node_size)?;
    let string_size = r.count(1)?;
    let raw = r.bytes(count * node_size)?;
    let strings = r.bytes(string_size)?;
    let mut nodes = Vec::with_capacity(count);
    let mut nr = Reader::new(raw, r.big_endian);
    for _ in 0..count {
        let version = nr.u16()?;
        let level = nr.u8()?;
        let type_flags = nr.u8()?;
        let type_offset = nr.u32()?;
        let name_offset = nr.u32()?;
        let byte_size = nr.i32()?;
        let index = nr.i32()?;
        let meta_flags = nr.u32()?;
        if node_size == 32 {
            let _ref_type_hash = nr.u64()?;
        }
        nodes.push(TypeTreeNode {
            version,
            level,
            type_flags,
            type_name: tree_string(strings, type_offset),
            name: tree_string(strings, name_offset),
            byte_size,
            index,
            meta_flags,
        });
    }
    Ok(nodes)
}

fn read_type(
    r: &mut Reader,
    version: u32,
    type_tree: bool,
    is_ref: bool,
) -> Result<SerializedType> {
    let class_id = r.i32()?;
    let is_stripped = version >= 16 && r.u8()? != 0;
    let script_type_index = if version >= 17 { r.i16()? } else { -1 };
    let has_script_id = (is_ref && script_type_index >= 0)
        || (version < 16 && class_id < 0)
        || (version >= 16 && class_id == MONO_BEHAVIOUR);
    let script_id = if has_script_id {
        Some(r.array()?)
    } else {
        None
    };
    let old_type_hash = r.array()?;
    let tree = if type_tree {
        let nodes = read_type_tree(r, version)?;
        if version >= 21 {
            if is_ref {
                let _class_name = r.cstr()?;
                let _namespace = r.cstr()?;
                let _assembly = r.cstr()?;
            } else {
                let deps = r.count(4)?;
                r.bytes(deps * 4)?;
            }
        }
        Some(nodes)
    } else {
        None
    };
    Ok(SerializedType {
        class_id,
        is_stripped,
        script_type_index,
        script_id,
        old_type_hash,
        type_tree: tree,
    })
}

impl SerializedFile {
    /// Parses the header and metadata. Object data stays in `bytes`; get it
    /// with [`SerializedFile::object_data`].
    pub fn parse(bytes: &[u8]) -> Result<SerializedFile> {
        let mut r = Reader::new(bytes, true);
        let _metadata_size = r.u32()?;
        let mut file_size = u64::from(r.u32()?);
        let version = r.u32()?;
        let mut data_offset = u64::from(r.u32()?);
        if !SUPPORTED_VERSIONS.contains(&version) {
            return Err(r.error(ErrorKind::Unsupported(format!(
                "serialized file version {version}"
            ))));
        }
        let big_endian = r.u8()? != 0;
        r.bytes(3)?;
        if version >= 22 {
            let _metadata_size = r.u32()?;
            file_size = r.u64()?;
            data_offset = r.u64()?;
            let _unknown = r.u64()?;
        }
        if file_size > bytes.len() as u64 || data_offset > bytes.len() as u64 {
            return Err(r.error(ErrorKind::Invalid(format!(
                "header says {file_size} bytes with data at {data_offset}, file has {}",
                bytes.len()
            ))));
        }
        r.big_endian = big_endian;

        let unity_version = r.cstr()?;
        let platform = r.i32()?;
        let type_tree_enabled = r.u8()? != 0;

        let type_count = r.count(23)?;
        let mut types = Vec::with_capacity(type_count);
        for _ in 0..type_count {
            types.push(read_type(&mut r, version, type_tree_enabled, false)?);
        }

        let object_count = r.count(20)?;
        let mut objects = Vec::with_capacity(object_count);
        for _ in 0..object_count {
            r.align(4)?;
            let path_id = r.i64()?;
            let start = if version >= 22 {
                r.u64()?
            } else {
                u64::from(r.u32()?)
            };
            let byte_size = r.u32()?;
            let type_index = r.i32()?;
            if version < 16 {
                let _class_id = r.u16()?;
            }
            if (11..17).contains(&version) {
                let _script_type_index = r.i16()?;
            }
            if version == 15 || version == 16 {
                let _stripped = r.u8()?;
            }
            let class_id = usize::try_from(type_index)
                .ok()
                .and_then(|i| types.get(i))
                .map(|t| t.class_id)
                .ok_or_else(|| {
                    r.error(ErrorKind::Invalid(format!(
                        "object type index {type_index}"
                    )))
                })?;
            let byte_start = data_offset + start;
            if byte_start + u64::from(byte_size) > bytes.len() as u64 {
                return Err(r.error(ErrorKind::Invalid(format!(
                    "object {path_id} runs past the file"
                ))));
            }
            objects.push(ObjectInfo {
                path_id,
                byte_start,
                byte_size,
                type_index,
                class_id,
            });
        }

        let script_count = r.count(12)?;
        for _ in 0..script_count {
            let _file_index = r.i32()?;
            r.align(4)?;
            let _local_id = r.i64()?;
        }

        let external_count = r.count(22)?;
        let mut externals = Vec::with_capacity(external_count);
        for _ in 0..external_count {
            let _empty = r.cstr()?;
            let guid = r.array()?;
            let kind = r.i32()?;
            let path = r.cstr()?;
            externals.push(External { guid, kind, path });
        }

        if version >= 20 {
            let ref_count = r.count(23)?;
            for _ in 0..ref_count {
                read_type(&mut r, version, type_tree_enabled, true)?;
            }
        }
        let _user_info = r.cstr()?;

        Ok(SerializedFile {
            version,
            unity_version,
            platform,
            big_endian,
            type_tree_enabled,
            types,
            objects,
            externals,
            data_offset,
            file_size,
        })
    }

    /// The raw bytes of one object, given the same `bytes` passed to `parse`.
    pub fn object_data<'a>(&self, bytes: &'a [u8], object: &ObjectInfo) -> Option<&'a [u8]> {
        let start = usize::try_from(object.byte_start).ok()?;
        bytes.get(start..start.checked_add(object.byte_size as usize)?)
    }
}
