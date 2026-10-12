//! Script layouts from the game's assemblies (M9u1, `docs/DESIGN.md` "M9u
//! plan", `docs/formats/ui.md` § Script fields): which fields of a
//! MonoBehaviour Unity 2019.4 stores and how, from the field metadata of
//! `Assembly-CSharp` and the assemblies it references, by Unity's
//! serialization rules:
//! - a field is stored if it is not static, const, readonly or
//!   `[NonSerialized]`, and is public or `[SerializeField]`;
//! - its type must be serializable: the primitives and `string`; enums
//!   (as their underlying type); references to `UnityEngine.Object`
//!   subclasses (PPtrs); Unity's built-in structs (a table here);
//!   `[Serializable]` classes and structs that are not abstract, generic
//!   or delegates (their fields inline, base classes first); `T[]` and
//!   `List<T>` of those (not of arrays). Other fields are skipped;
//! - nested custom classes stop at Unity's depth limit.
//!
//! The rules are checked on data: every object read must end with its
//! last field (`sn-inspect ui --scripts`).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use sn_dotnet::{
    Assembly, FIELD_ACCESS_MASK, FIELD_INIT_ONLY, FIELD_LITERAL, FIELD_NOT_SERIALIZED,
    FIELD_PUBLIC, FIELD_STATIC, NamedType, RefScope, SigType, TYPE_ABSTRACT, TYPE_INTERFACE,
    TYPE_SERIALIZABLE, Table,
};
use sn_install::GameData;
use sn_unity::{Field, Kind, StructLayout};

use crate::Result;

/// The assemblies the layouts start from; everything they reference
/// (but the .NET libraries) is loaded too.
const ROOTS: &[&str] = &[
    "Assembly-CSharp",
    "Assembly-CSharp-firstpass",
    "UnityEngine.UI",
    "Unity.TextMeshPro",
];

/// Nested custom classes deeper than this are not stored (Unity's
/// serialization depth limit). *Hypothesis* for 2019.4: 7; checked by
/// the objects' sizes.
const DEPTH_LIMIT: usize = 7;

/// Base classes whose own fields are the object header, not script
/// fields.
const ENGINE_BASES: &[&str] = &[
    "UnityEngine.MonoBehaviour",
    "UnityEngine.Behaviour",
    "UnityEngine.Component",
    "UnityEngine.ScriptableObject",
    "UnityEngine.Object",
    "System.Object",
    "System.ValueType",
];

fn is_system(name: &str) -> bool {
    name == "mscorlib"
        || name == "netstandard"
        || name.starts_with("System")
        || name.starts_with("Mono.")
        || name.starts_with("Microsoft.")
}

/// The game's script assemblies, as bytes (name without `.dll`).
pub struct ScriptAssemblies {
    pub files: Vec<(String, Vec<u8>)>,
}

impl ScriptAssemblies {
    /// `Managed/` assemblies: the roots and what they reference, but the
    /// .NET libraries (whose types Unity does not serialize, but for the
    /// primitives, `string` and `List<T>`, which are known by name).
    pub fn load(game: &GameData) -> Result<ScriptAssemblies> {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut queue: Vec<String> = ROOTS.iter().map(|s| s.to_string()).collect();
        while let Some(name) = queue.pop() {
            if is_system(&name) || files.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let path = game.data_dir.join("Managed").join(format!("{name}.dll"));
            if !path.is_file() {
                continue;
            }
            let bytes = game.read_file(&path)?;
            let refs: Vec<String> = Assembly::parse(&bytes)
                .and_then(|a| a.assembly_refs())
                .map_err(|e| format!("{}: {e}", path.display()))?
                .into_iter()
                .map(String::from)
                .collect();
            files.push((name, bytes));
            queue.extend(refs);
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(ScriptAssemblies { files })
    }

    pub fn layouts(&self) -> Result<Layouts<'_>> {
        Layouts::new(&self.files)
    }
}

/// A TypeDef of one of the loaded assemblies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct TypeKey {
    asm: usize,
    row: u32,
}

/// A generic argument, with the assembly its tokens belong to.
type TypeArg = (usize, SigType);

struct Loaded<'a> {
    name: String,
    asm: Assembly<'a>,
    /// Top-level types by (namespace, name).
    top: HashMap<(&'a str, &'a str), u32>,
    /// Nested types by (outer row, name).
    nested: HashMap<(u32, &'a str), u32>,
    /// Outer row of each nested type.
    outer: HashMap<u32, u32>,
    /// Field rows with `[SerializeField]`.
    serialize_field: std::collections::HashSet<u32>,
    /// Field rows with `[SerializeReference]`.
    serialize_reference: std::collections::HashSet<u32>,
}

/// Script layouts, built on demand and cached.
pub struct Layouts<'a> {
    loaded: Vec<Loaded<'a>>,
    by_name: HashMap<String, usize>,
    /// By type and depth (the depth limit makes a type's layout depend on
    /// how deep it is).
    cache: RefCell<HashMap<(TypeKey, usize), LayoutResult>>,
    /// Why fields were not stored, with counts (for the logs).
    pub skipped: RefCell<BTreeMap<String, usize>>,
}

type LayoutResult = Result<Arc<StructLayout>>;

/// What a type token names.
enum Resolved {
    Def(TypeKey),
    /// In an assembly that is not loaded (the .NET libraries); full name.
    External(String),
}

impl<'a> Layouts<'a> {
    pub fn new(files: &'a [(String, Vec<u8>)]) -> Result<Layouts<'a>> {
        let mut loaded = Vec::with_capacity(files.len());
        let mut by_name = HashMap::new();
        for (name, bytes) in files {
            let e = |e: sn_dotnet::Error| format!("{name}: {e}");
            let asm = Assembly::parse(bytes).map_err(e)?;
            let mut outer = HashMap::new();
            let mut nested = HashMap::new();
            for (inner, out) in asm.nested_pairs().map_err(e)? {
                outer.insert(inner, out);
                nested.insert((out, asm.type_name(inner).map_err(e)?.1), inner);
            }
            let mut top = HashMap::new();
            for row in 1..=asm.rows(Table::TYPE_DEF) {
                if !outer.contains_key(&row) {
                    top.insert(asm.type_name(row).map_err(e)?, row);
                }
            }
            let mut serialize_field = std::collections::HashSet::new();
            let mut serialize_reference = std::collections::HashSet::new();
            for a in asm.attributes().map_err(e)? {
                if a.parent.table != Table::FIELD || a.namespace != "UnityEngine" {
                    continue;
                }
                match a.name.as_str() {
                    "SerializeField" => {
                        serialize_field.insert(a.parent.row);
                    }
                    "SerializeReference" => {
                        serialize_reference.insert(a.parent.row);
                    }
                    _ => {}
                }
            }
            by_name.insert(name.clone(), loaded.len());
            loaded.push(Loaded {
                name: name.clone(),
                asm,
                top,
                nested,
                outer,
                serialize_field,
                serialize_reference,
            });
        }
        Ok(Layouts {
            loaded,
            by_name,
            cache: RefCell::new(HashMap::new()),
            skipped: RefCell::new(BTreeMap::new()),
        })
    }

    /// The stored fields of script `namespace.class` in `assembly` (a
    /// MonoScript's names; `.dll` is optional). Nested script classes are
    /// written `Outer/Inner`.
    pub fn script(
        &self,
        assembly: &str,
        namespace: &str,
        class: &str,
    ) -> Result<Arc<StructLayout>> {
        let name = assembly.strip_suffix(".dll").unwrap_or(assembly);
        let &asm = self
            .by_name
            .get(name)
            .ok_or_else(|| format!("assembly {name} is not loaded"))?;
        let l = &self.loaded[asm];
        let mut parts = class.split('/');
        let first = parts.next().unwrap_or(class);
        let mut row = *l
            .top
            .get(&(namespace, first))
            .ok_or_else(|| format!("{name}: no type {namespace}.{first}"))?;
        for inner in parts {
            row = *l
                .nested
                .get(&(row, inner))
                .ok_or_else(|| format!("{name}: no type {class}"))?;
        }
        self.layout_of(TypeKey { asm, row }, &[], 0)
    }

    fn skip(&self, why: String) {
        *self.skipped.borrow_mut().entry(why).or_default() += 1;
    }

    /// `Namespace.Outer/Inner`.
    fn full_name(&self, key: TypeKey) -> Result<String> {
        let l = &self.loaded[key.asm];
        let e = |e: sn_dotnet::Error| format!("{}: {e}", l.name);
        let (ns, n) = l.asm.type_name(key.row).map_err(e)?;
        match l.outer.get(&key.row) {
            Some(&o) => Ok(format!(
                "{}/{n}",
                self.full_name(TypeKey {
                    asm: key.asm,
                    row: o
                })?
            )),
            None if ns.is_empty() => Ok(n.into()),
            None => Ok(format!("{ns}.{n}")),
        }
    }

    fn resolve(&self, asm: usize, t: &NamedType) -> Result<Resolved> {
        let l = &self.loaded[asm];
        let e = |e: sn_dotnet::Error| format!("{}: {e}", l.name);
        match t.token.table {
            Table::TYPE_DEF => Ok(Resolved::Def(TypeKey {
                asm,
                row: t.token.row,
            })),
            Table::TYPE_REF => match l.asm.type_ref_scope(t.token.row).map_err(e)? {
                RefScope::Assembly(name) => match self.by_name.get(&name) {
                    Some(&other) => match self.loaded[other]
                        .top
                        .get(&(t.namespace.as_str(), t.name.as_str()))
                    {
                        Some(&row) => Ok(Resolved::Def(TypeKey { asm: other, row })),
                        None => Ok(Resolved::External(qualified(&t.namespace, &t.name))),
                    },
                    None => Ok(Resolved::External(qualified(&t.namespace, &t.name))),
                },
                RefScope::Module => match l.top.get(&(t.namespace.as_str(), t.name.as_str())) {
                    Some(&row) => Ok(Resolved::Def(TypeKey { asm, row })),
                    None => Ok(Resolved::External(qualified(&t.namespace, &t.name))),
                },
                RefScope::Outer(outer_ref) => {
                    let (ns, n) = l
                        .asm
                        .type_def_or_ref_name(Table::TYPE_REF, outer_ref)
                        .map_err(e)?;
                    let outer = NamedType {
                        token: sn_dotnet::TypeToken {
                            table: Table::TYPE_REF,
                            row: outer_ref,
                        },
                        namespace: ns,
                        name: n,
                    };
                    match self.resolve(asm, &outer)? {
                        Resolved::Def(o) => {
                            match self.loaded[o.asm].nested.get(&(o.row, t.name.as_str())) {
                                Some(&row) => Ok(Resolved::Def(TypeKey { asm: o.asm, row })),
                                None => Err(format!(
                                    "no nested type {} in {}",
                                    t.name,
                                    self.full_name(o)?
                                )),
                            }
                        }
                        Resolved::External(o) => Ok(Resolved::External(format!("{o}/{}", t.name))),
                    }
                }
            },
            other => Err(format!("type token in table {other:#x}")),
        }
    }

    /// The type a TypeDef extends, resolved, with its generic arguments.
    fn base(&self, key: TypeKey) -> Result<Option<(Resolved, Vec<TypeArg>)>> {
        let l = &self.loaded[key.asm];
        let e = |e: sn_dotnet::Error| format!("{}: {e}", l.name);
        Ok(match l.asm.extends(key.row).map_err(e)? {
            None => None,
            Some(SigType::Class(t) | SigType::ValueType(t)) => {
                Some((self.resolve(key.asm, &t)?, Vec::new()))
            }
            Some(SigType::Generic(base, args)) => match *base {
                SigType::Class(t) | SigType::ValueType(t) => Some((
                    self.resolve(key.asm, &t)?,
                    args.into_iter().map(|a| (key.asm, a)).collect(),
                )),
                _ => {
                    return Err(format!(
                        "{}: generic base of a non-type",
                        self.full_name(key)?
                    ));
                }
            },
            Some(other) => return Err(format!("{}: extends {other}", self.full_name(key)?)),
        })
    }

    /// Full names of the type and its bases, nearest first.
    fn ancestry(&self, key: TypeKey) -> Result<Vec<String>> {
        let mut out = vec![self.full_name(key)?];
        let mut at = key;
        for _ in 0..64 {
            match self.base(at)? {
                Some((Resolved::Def(b), _)) => {
                    out.push(self.full_name(b)?);
                    at = b;
                }
                Some((Resolved::External(n), _)) => {
                    out.push(n);
                    break;
                }
                None => break,
            }
        }
        Ok(out)
    }

    /// The stored fields of a class or struct, base classes first.
    fn layout_of(&self, key: TypeKey, args: &[TypeArg], depth: usize) -> Result<Arc<StructLayout>> {
        if args.is_empty()
            && let Some(cached) = self.cache.borrow().get(&(key, depth))
        {
            return cached.clone();
        }
        let result = self.build_layout(key, args, depth);
        if args.is_empty() {
            self.cache.borrow_mut().insert((key, depth), result.clone());
        }
        result
    }

    fn build_layout(
        &self,
        key: TypeKey,
        args: &[TypeArg],
        depth: usize,
    ) -> Result<Arc<StructLayout>> {
        let type_name = self.full_name(key)?;
        let mut fields = Vec::new();
        if let Some((base, base_args)) = self.base(key)? {
            if let Resolved::Def(b) = base
                && !ENGINE_BASES.contains(&self.full_name(b)?.as_str())
            {
                // The base's own arguments may name this type's parameters.
                let base_args: Vec<TypeArg> = base_args
                    .into_iter()
                    .map(|(a, t)| substitute(a, t, args))
                    .collect();
                fields.extend(
                    self.build_layout(b, &base_args, depth)?
                        .fields
                        .iter()
                        .cloned(),
                );
            }
        }
        let l = &self.loaded[key.asm];
        let e = |e: sn_dotnet::Error| format!("{}: {e}", l.name);
        for row in l.asm.fields(key.row).map_err(e)? {
            let flags = l.asm.field_flags(row).map_err(e)?;
            if flags & (FIELD_STATIC | FIELD_LITERAL | FIELD_INIT_ONLY | FIELD_NOT_SERIALIZED) != 0
            {
                continue;
            }
            let name = l.asm.field_name(row).map_err(e)?;
            if l.serialize_reference.contains(&row) {
                return Err(format!(
                    "{type_name}.{name}: [SerializeReference] is not supported"
                ));
            }
            let public = flags & FIELD_ACCESS_MASK == FIELD_PUBLIC;
            if !public && !l.serialize_field.contains(&row) {
                continue;
            }
            let sig = l.asm.field_type(row).map_err(e)?;
            if let Some(kind) = self.kind_of(key.asm, &sig, args, depth)? {
                fields.push(Field::script(name, kind));
            }
        }
        Ok(Arc::new(StructLayout { type_name, fields }))
    }

    /// How a field of type `sig` (in assembly `asm`) is stored; `None`
    /// if Unity does not store it.
    fn kind_of(
        &self,
        asm: usize,
        sig: &SigType,
        args: &[TypeArg],
        depth: usize,
    ) -> Result<Option<Kind>> {
        Ok(match sig {
            SigType::Primitive(e) => match e {
                0x02 => Some(Kind::Bool),
                0x03 | 0x07 => Some(Kind::U16),
                0x04 => Some(Kind::I8),
                0x05 => Some(Kind::U8),
                0x06 => Some(Kind::I16),
                0x08 => Some(Kind::I32),
                0x09 => Some(Kind::U32),
                0x0A => Some(Kind::I64),
                0x0B => Some(Kind::U64),
                0x0C => Some(Kind::F32),
                0x0D => Some(Kind::F64),
                0x0E => Some(Kind::String),
                _ => {
                    self.skip(format!("type {sig}"));
                    None
                }
            },
            SigType::Var(n) => match args.get(*n as usize) {
                Some((a, t)) => self.kind_of(*a, t, &[], depth)?,
                None => return Err(format!("generic parameter {n} without an argument")),
            },
            SigType::SzArray(element) => self.array_of(asm, element, args, depth)?,
            SigType::Generic(base, generic_args) => {
                let is_list = matches!(&**base,
                    SigType::Class(t) if t.namespace == "System.Collections.Generic" && t.name == "List`1");
                match (is_list, generic_args.as_slice()) {
                    (true, [element]) => self.array_of(asm, element, args, depth)?,
                    _ => {
                        self.skip(format!("generic {base}"));
                        None
                    }
                }
            }
            SigType::Class(t) | SigType::ValueType(t) => self.named_kind(asm, t, depth)?,
            SigType::MVar(_) | SigType::Array(..) | SigType::Unsupported(_) => {
                self.skip(format!("type {sig}"));
                None
            }
        })
    }

    fn array_of(
        &self,
        asm: usize,
        element: &SigType,
        args: &[TypeArg],
        depth: usize,
    ) -> Result<Option<Kind>> {
        Ok(match self.kind_of(asm, element, args, depth)? {
            Some(Kind::Array(_)) => {
                self.skip("array of arrays".into());
                None
            }
            Some(k) => Some(Kind::Array(Box::new(k))),
            None => None,
        })
    }

    fn named_kind(&self, asm: usize, t: &NamedType, depth: usize) -> Result<Option<Kind>> {
        let key = match self.resolve(asm, t)? {
            Resolved::Def(k) => k,
            Resolved::External(name) => {
                if let Some(b) = builtin(&name) {
                    return Ok(Some(b));
                }
                self.skip(format!("external {name}"));
                return Ok(None);
            }
        };
        let name = self.full_name(key)?;
        if let Some(b) = builtin(&name) {
            return Ok(Some(b));
        }
        let ancestry = self.ancestry(key)?;
        if ancestry.iter().any(|a| a == "UnityEngine.Object") {
            return Ok(Some(Kind::PPtr));
        }
        if ancestry
            .iter()
            .any(|a| a == "System.MulticastDelegate" || a == "System.Delegate")
        {
            self.skip("delegate".into());
            return Ok(None);
        }
        let l = &self.loaded[key.asm];
        let e = |e: sn_dotnet::Error| format!("{}: {e}", l.name);
        if ancestry.get(1).map(String::as_str) == Some("System.Enum") {
            for row in l.asm.fields(key.row).map_err(e)? {
                if l.asm.field_flags(row).map_err(e)? & FIELD_STATIC == 0 {
                    let underlying = l.asm.field_type(row).map_err(e)?;
                    return self.kind_of(key.asm, &underlying, &[], depth);
                }
            }
            return Err(format!("enum {name} without a value field"));
        }
        let flags = l.asm.type_flags(key.row).map_err(e)?;
        if flags & (TYPE_INTERFACE | TYPE_ABSTRACT) != 0 {
            self.skip(format!("abstract {name}"));
            return Ok(None);
        }
        if flags & TYPE_SERIALIZABLE == 0 {
            self.skip(format!("not [Serializable]: {name}"));
            return Ok(None);
        }
        if name.contains('`') {
            self.skip(format!("generic {name}"));
            return Ok(None);
        }
        if depth >= DEPTH_LIMIT {
            self.skip(format!("depth limit at {name}"));
            return Ok(None);
        }
        Ok(Some(Kind::Struct(self.layout_of(key, &[], depth + 1)?)))
    }
}

fn qualified(ns: &str, name: &str) -> String {
    if ns.is_empty() {
        name.into()
    } else {
        format!("{ns}.{name}")
    }
}

/// A generic argument that names a parameter of the type being laid out
/// takes that parameter's argument.
fn substitute(asm: usize, t: SigType, args: &[TypeArg]) -> TypeArg {
    match t {
        SigType::Var(n) => args
            .get(n as usize)
            .cloned()
            .unwrap_or((asm, SigType::Var(n))),
        other => (asm, other),
    }
}

fn packed_struct(type_name: &str, fields: Vec<Field>) -> Kind {
    Kind::Struct(Arc::new(StructLayout {
        type_name: type_name.into(),
        fields,
    }))
}

fn floats(type_name: &str, names: &[&str]) -> Kind {
    packed_struct(
        type_name,
        names.iter().map(|n| Field::packed(*n, Kind::F32)).collect(),
    )
}

fn ints(type_name: &str, names: &[&str]) -> Kind {
    packed_struct(
        type_name,
        names.iter().map(|n| Field::packed(*n, Kind::I32)).collect(),
    )
}

/// Unity's built-in structs, stored natively (their managed fields are
/// not what is written, or need names Unity uses).
fn builtin(name: &str) -> Option<Kind> {
    Some(match name {
        "UnityEngine.Vector2" => floats(name, &["x", "y"]),
        "UnityEngine.Vector3" => floats(name, &["x", "y", "z"]),
        "UnityEngine.Vector4" | "UnityEngine.Quaternion" => floats(name, &["x", "y", "z", "w"]),
        "UnityEngine.Color" => floats(name, &["r", "g", "b", "a"]),
        "UnityEngine.Rect" => floats(name, &["x", "y", "width", "height"]),
        "UnityEngine.Matrix4x4" => {
            let names: Vec<String> = (0..4)
                .flat_map(|r| (0..4).map(move |c| format!("e{r}{c}")))
                .collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            floats(name, &refs)
        }
        "UnityEngine.Bounds" => packed_struct(
            name,
            vec![
                Field::packed("m_Center", builtin("UnityEngine.Vector3")?),
                Field::packed("m_Extent", builtin("UnityEngine.Vector3")?),
            ],
        ),
        "UnityEngine.Color32" => packed_struct(
            name,
            ["r", "g", "b", "a"]
                .iter()
                .map(|n| Field::packed(*n, Kind::U8))
                .collect(),
        ),
        "UnityEngine.Vector2Int" => ints(name, &["m_X", "m_Y"]),
        "UnityEngine.Vector3Int" => ints(name, &["m_X", "m_Y", "m_Z"]),
        "UnityEngine.RectInt" => ints(name, &["m_XMin", "m_YMin", "m_Width", "m_Height"]),
        "UnityEngine.BoundsInt" => packed_struct(
            name,
            vec![
                Field::packed("m_Position", builtin("UnityEngine.Vector3Int")?),
                Field::packed("m_Size", builtin("UnityEngine.Vector3Int")?),
            ],
        ),
        "UnityEngine.LayerMask" => packed_struct(name, vec![Field::packed("m_Bits", Kind::U32)]),
        "UnityEngine.RectOffset" => ints(name, &["m_Left", "m_Right", "m_Top", "m_Bottom"]),
        "UnityEngine.GUIStyle" => {
            // Stored natively; checked on `UWE.LargeWorldStreamerGUI`, whose
            // fields end exactly after it (M9u1).
            let state = packed_struct(
                "UnityEngine.GUIStyleState",
                vec![
                    Field::packed("m_Background", Kind::PPtr),
                    Field::packed("m_TextColor", builtin("UnityEngine.Color")?),
                ],
            );
            let mut f = vec![Field::script("m_Name", Kind::String)];
            for s in [
                "m_Normal",
                "m_Hover",
                "m_Active",
                "m_Focused",
                "m_OnNormal",
                "m_OnHover",
                "m_OnActive",
                "m_OnFocused",
            ] {
                f.push(Field::packed(s, state.clone()));
            }
            for s in ["m_Border", "m_Margin", "m_Padding", "m_Overflow"] {
                f.push(Field::packed(s, builtin("UnityEngine.RectOffset")?));
            }
            f.extend([
                Field::packed("m_Font", Kind::PPtr),
                Field::packed("m_FontSize", Kind::I32),
                Field::packed("m_FontStyle", Kind::I32),
                Field::packed("m_Alignment", Kind::I32),
                Field::packed("m_WordWrap", Kind::Bool),
                Field::script("m_RichText", Kind::Bool),
                Field::packed("m_TextClipping", Kind::I32),
                Field::packed("m_ImagePosition", Kind::I32),
                Field::packed("m_ContentOffset", builtin("UnityEngine.Vector2")?),
                Field::packed("m_FixedWidth", Kind::F32),
                Field::packed("m_FixedHeight", Kind::F32),
                Field::packed("m_StretchWidth", Kind::Bool),
                Field::script("m_StretchHeight", Kind::Bool),
            ]);
            packed_struct(name, f)
        }
        "UnityEngine.AnimationCurve" => {
            let key = packed_struct(
                "UnityEngine.Keyframe",
                vec![
                    Field::packed("time", Kind::F32),
                    Field::packed("value", Kind::F32),
                    Field::packed("inSlope", Kind::F32),
                    Field::packed("outSlope", Kind::F32),
                    Field::packed("weightedMode", Kind::I32),
                    Field::packed("inWeight", Kind::F32),
                    Field::packed("outWeight", Kind::F32),
                ],
            );
            packed_struct(
                name,
                vec![
                    Field::script("m_Curve", Kind::Array(Box::new(key))),
                    Field::packed("m_PreInfinity", Kind::I32),
                    Field::packed("m_PostInfinity", Kind::I32),
                    Field::packed("m_RotationOrder", Kind::I32),
                ],
            )
        }
        "UnityEngine.Gradient" => {
            let mut f: Vec<Field> = (0..8)
                .map(|i| {
                    Field::packed(
                        format!("key{i}"),
                        builtin("UnityEngine.Color").unwrap_or(Kind::I32),
                    )
                })
                .collect();
            f.extend((0..8).map(|i| Field::packed(format!("ctime{i}"), Kind::U16)));
            f.extend((0..8).map(|i| Field::packed(format!("atime{i}"), Kind::U16)));
            f.push(Field::packed("m_Mode", Kind::I32));
            f.push(Field::packed("m_NumColorKeys", Kind::U8));
            f.push(Field {
                name: "m_NumAlphaKeys".into(),
                kind: Kind::U8,
                align: true,
            });
            packed_struct(name, f)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_dotnet::encode::Builder;

    const SERIALIZABLE: u32 = TYPE_SERIALIZABLE;

    /// An engine assembly with MonoBehaviour, Object, Vector3,
    /// SerializeField, and a game assembly using them.
    fn assemblies() -> Vec<(String, Vec<u8>)> {
        let mut core = Builder::new();
        core.assembly("UnityEngine.CoreModule");
        let mscorlib = core.assembly_ref("mscorlib");
        let object = core.type_ref_in((Table::ASSEMBLY_REF, mscorlib), "System", "Object");
        let value_type = core.type_ref_in((Table::ASSEMBLY_REF, mscorlib), "System", "ValueType");
        let uobject =
            core.type_def_with(0, "UnityEngine", "Object", Some((Table::TYPE_REF, object)));
        let component = core.type_def_with(
            0,
            "UnityEngine",
            "Component",
            Some((Table::TYPE_DEF, uobject)),
        );
        let behaviour = core.type_def_with(
            0,
            "UnityEngine",
            "Behaviour",
            Some((Table::TYPE_DEF, component)),
        );
        core.type_def_with(
            0,
            "UnityEngine",
            "MonoBehaviour",
            Some((Table::TYPE_DEF, behaviour)),
        );
        core.type_def_with(
            SERIALIZABLE,
            "UnityEngine",
            "Vector3",
            Some((Table::TYPE_REF, value_type)),
        );
        core.type_def_with(
            0,
            "UnityEngine",
            "SerializeField",
            Some((Table::TYPE_REF, object)),
        );
        core.method(".ctor", 0, None);
        core.type_def_with(
            0,
            "UnityEngine",
            "Texture",
            Some((Table::TYPE_DEF, uobject)),
        );

        let mut game = Builder::new();
        game.assembly("Assembly-CSharp");
        let core_ref = game.assembly_ref("UnityEngine.CoreModule");
        let ms = game.assembly_ref("mscorlib");
        let mono = game.type_ref_in(
            (Table::ASSEMBLY_REF, core_ref),
            "UnityEngine",
            "MonoBehaviour",
        );
        let vec3 = game.type_ref_in((Table::ASSEMBLY_REF, core_ref), "UnityEngine", "Vector3");
        let tex = game.type_ref_in((Table::ASSEMBLY_REF, core_ref), "UnityEngine", "Texture");
        let sf = game.type_ref_in(
            (Table::ASSEMBLY_REF, core_ref),
            "UnityEngine",
            "SerializeField",
        );
        let sf_ctor = game.member_ref((Table::TYPE_REF, sf), ".ctor", Some(0)) & 0x00FF_FFFF;
        let sys_object = game.type_ref_in((Table::ASSEMBLY_REF, ms), "System", "Object");
        let sys_enum = game.type_ref_in((Table::ASSEMBLY_REF, ms), "System", "Enum");
        let list = game.type_ref_in(
            (Table::ASSEMBLY_REF, ms),
            "System.Collections.Generic",
            "List`1",
        );
        let dict = game.type_ref_in(
            (Table::ASSEMBLY_REF, ms),
            "System.Collections.Generic",
            "Dictionary`2",
        );

        // enum Mode : byte
        let mode = game.type_def_with(0, "", "Mode", Some((Table::TYPE_REF, sys_enum)));
        game.field_with_sig(0x0006, "value__", &[0x06, 0x05]);
        game.field_with_sig(0x0056, "A", &[0x06, 0x11, (mode << 2) as u8]);
        // [Serializable] class Entry { public int id; public Entry next; }
        let entry = game.type_def_with(
            SERIALIZABLE,
            "",
            "Entry",
            Some((Table::TYPE_REF, sys_object)),
        );
        game.field_with_sig(0x0006, "id", &[0x06, 0x08]);
        game.field_with_sig(0x0006, "next", &[0x06, 0x12, (entry << 2) as u8]);
        // class NotSerializable { public int x; }
        let plain = game.type_def_with(0, "", "Plain", Some((Table::TYPE_REF, sys_object)));
        game.field_with_sig(0x0006, "x", &[0x06, 0x08]);
        // class Base : MonoBehaviour { public float speed; }
        let base = game.type_def_with(0, "", "Base", Some((Table::TYPE_REF, mono)));
        game.field_with_sig(0x0006, "speed", &[0x06, 0x0C]);
        // class Script : Base
        game.type_def_with(0, "", "Script", Some((Table::TYPE_DEF, base)));
        let coded_ref = |r: u32| ((r << 2) | 1) as u8;
        game.field_with_sig(0x0006, "flag", &[0x06, 0x02]);
        game.field_with_sig(0x0006, "position", &[0x06, 0x11, coded_ref(vec3)]);
        game.field_with_sig(0x0006, "texture", &[0x06, 0x12, coded_ref(tex)]);
        game.field_with_sig(0x0006, "mode", &[0x06, 0x11, (mode << 2) as u8]);
        game.field_with_sig(
            0x0006,
            "entries",
            &[
                0x06,
                0x15,
                0x12,
                coded_ref(list),
                1,
                0x12,
                (entry << 2) as u8,
            ],
        );
        game.field_with_sig(0x0006, "names", &[0x06, 0x1D, 0x0E]);
        let hidden = game.field_with_sig(0x0001, "hidden", &[0x06, 0x08]) & 0x00FF_FFFF;
        let kept = game.field_with_sig(0x0001, "kept", &[0x06, 0x08]) & 0x00FF_FFFF;
        game.field_with_sig(0x0016, "shared", &[0x06, 0x08]); // static
        game.field_with_sig(0x0026, "fixedValue", &[0x06, 0x08]); // readonly
        game.field_with_sig(0x0086, "notSerialized", &[0x06, 0x08]);
        game.field_with_sig(0x0006, "plain", &[0x06, 0x12, (plain << 2) as u8]);
        game.field_with_sig(
            0x0006,
            "map",
            &[0x06, 0x15, 0x12, coded_ref(dict), 2, 0x08, 0x08],
        );
        game.field_with_sig(0x0006, "grid", &[0x06, 0x1D, 0x1D, 0x08]);
        game.attribute((Table::FIELD, kept), (Table::MEMBER_REF, sf_ctor));
        let _ = hidden;
        vec![
            ("UnityEngine.CoreModule".into(), core.build()),
            ("Assembly-CSharp".into(), game.build()),
        ]
    }

    fn names(l: &StructLayout) -> Vec<&str> {
        l.fields.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn applies_unitys_rules() {
        let files = assemblies();
        let layouts = Layouts::new(&files).unwrap();
        let l = layouts.script("Assembly-CSharp.dll", "", "Script").unwrap();
        assert_eq!(
            names(&l),
            [
                "speed", "flag", "position", "texture", "mode", "entries", "names", "kept"
            ]
        );
        let kind = |n: &str| &l.fields.iter().find(|f| f.name == n).unwrap().kind;
        assert_eq!(kind("speed"), &Kind::F32);
        assert_eq!(kind("texture"), &Kind::PPtr);
        assert_eq!(kind("mode"), &Kind::U8);
        assert!(l.fields.iter().find(|f| f.name == "mode").unwrap().align);
        assert_eq!(kind("names"), &Kind::Array(Box::new(Kind::String)));
        let Kind::Struct(position) = kind("position") else {
            panic!("position is not a struct")
        };
        assert_eq!(names(position), ["x", "y", "z"]);
        // The self-referencing class stops at the depth limit.
        let Kind::Array(entry) = kind("entries") else {
            panic!("entries is not an array")
        };
        let mut depth = 0;
        let mut at = (**entry).clone();
        while let Kind::Struct(s) = at {
            depth += 1;
            assert_eq!(s.fields[0].name, "id");
            match s.fields.get(1) {
                Some(f) => at = f.kind.clone(),
                None => break,
            }
        }
        assert_eq!(depth, DEPTH_LIMIT);
        let skipped = layouts.skipped.borrow();
        assert!(
            skipped.contains_key("not [Serializable]: Plain"),
            "{skipped:?}"
        );
        assert!(skipped.contains_key("array of arrays"), "{skipped:?}");
        assert!(
            skipped.keys().any(|k| k.contains("Dictionary")),
            "{skipped:?}"
        );
    }

    #[test]
    fn generic_bases_take_their_arguments() {
        let mut core = Builder::new();
        core.assembly("UnityEngine.CoreModule");
        let ms = core.assembly_ref("mscorlib");
        let object = core.type_ref_in((Table::ASSEMBLY_REF, ms), "System", "Object");
        let uobject =
            core.type_def_with(0, "UnityEngine", "Object", Some((Table::TYPE_REF, object)));
        core.type_def_with(
            0,
            "UnityEngine",
            "MonoBehaviour",
            Some((Table::TYPE_DEF, uobject)),
        );

        let mut game = Builder::new();
        game.assembly("Assembly-CSharp");
        let core_ref = game.assembly_ref("UnityEngine.CoreModule");
        let ms = game.assembly_ref("mscorlib");
        let mono = game.type_ref_in(
            (Table::ASSEMBLY_REF, core_ref),
            "UnityEngine",
            "MonoBehaviour",
        );
        let list = game.type_ref_in(
            (Table::ASSEMBLY_REF, ms),
            "System.Collections.Generic",
            "List`1",
        );
        // class Holder<T> : MonoBehaviour { public T value; public List<T> items; }
        let holder = game.type_def_with(0, "", "Holder`1", Some((Table::TYPE_REF, mono)));
        game.field_with_sig(0x0006, "value", &[0x06, 0x13, 0]);
        game.field_with_sig(
            0x0006,
            "items",
            &[0x06, 0x15, 0x12, ((list << 2) | 1) as u8, 1, 0x13, 0],
        );
        // class Script : Holder<float> { public int own; }
        let spec = game.type_spec(&[0x15, 0x12, (holder << 2) as u8, 1, 0x0C]);
        game.type_def_with(0, "", "Script", Some((Table::TYPE_SPEC, spec)));
        game.field_with_sig(0x0006, "own", &[0x06, 0x08]);
        let files = vec![
            ("UnityEngine.CoreModule".to_string(), core.build()),
            ("Assembly-CSharp".to_string(), game.build()),
        ];
        let layouts = Layouts::new(&files).unwrap();
        let l = layouts.script("Assembly-CSharp", "", "Script").unwrap();
        let fields: Vec<(&str, &Kind)> = l
            .fields
            .iter()
            .map(|f| (f.name.as_str(), &f.kind))
            .collect();
        assert_eq!(
            fields,
            [
                ("value", &Kind::F32),
                ("items", &Kind::Array(Box::new(Kind::F32))),
                ("own", &Kind::I32),
            ]
        );
    }

    #[test]
    fn missing_types_and_assemblies_are_errors() {
        let files = assemblies();
        let layouts = Layouts::new(&files).unwrap();
        assert!(layouts.script("Assembly-CSharp", "", "Nope").is_err());
        assert!(layouts.script("Other", "", "Script").is_err());
        assert!(
            layouts
                .script("Assembly-CSharp", "", "Script/Inner")
                .is_err()
        );
    }

    #[test]
    fn reads_an_object_by_its_layout() {
        let files = assemblies();
        let layouts = Layouts::new(&files).unwrap();
        let mut simple = Builder::new();
        simple.assembly("Small");
        let core_ref = simple.assembly_ref("UnityEngine.CoreModule");
        let mono = simple.type_ref_in(
            (Table::ASSEMBLY_REF, core_ref),
            "UnityEngine",
            "MonoBehaviour",
        );
        simple.type_def_with(0, "N", "S", Some((Table::TYPE_REF, mono)));
        simple.field_with_sig(0x0006, "on", &[0x06, 0x02]);
        simple.field_with_sig(0x0006, "value", &[0x06, 0x0C]);
        let mut all = files.clone();
        all.push(("Small".into(), simple.build()));
        drop(layouts);
        let layouts = Layouts::new(&all).unwrap();
        let l = layouts.script("Small", "N", "S").unwrap();
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&1i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&2i64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&2.5f32.to_le_bytes());
        let (_, f) = sn_unity::read_script(&b, false, &l).unwrap();
        assert_eq!(f.get("on").and_then(sn_unity::Value::as_bool), Some(true));
        assert_eq!(f.get("value").and_then(sn_unity::Value::as_f64), Some(2.5));
    }
}
