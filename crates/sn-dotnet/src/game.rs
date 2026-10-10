//! The game data Subnautica keeps only in code, read from its
//! `Assembly-CSharp.dll`: enum names (`TechType`), the crafting menus
//! (`CraftTree`) and `TechData`'s defaults. Each reader accepts only the
//! instruction patterns the game's compiler produced for that code and
//! fails on anything else, so a changed game is noticed, not misread.
//! See `docs/formats/dotnet.md`.

use crate::assembly::{Assembly, FIELD_LITERAL, FIELD_STATIC};
use crate::il::{
    CALL, CALLVIRT, DUP, Instr, LDARG_0, LDC_R4, LDSFLD, LDSTR, MUL, NEWARR, NEWOBJ, Operand, RET,
    STELEM_REF, STFLD, STSFLD, decode, opcode_name,
};
use crate::{Error, Result};

/// Element types of field constants (§ II.23.1.16).
const ELEMENT_I1: u8 = 0x04;
const ELEMENT_U1: u8 = 0x05;
const ELEMENT_I2: u8 = 0x06;
const ELEMENT_U2: u8 = 0x07;
const ELEMENT_I4: u8 = 0x08;
const ELEMENT_U4: u8 = 0x09;
const ELEMENT_I8: u8 = 0x0A;
const ELEMENT_U8: u8 = 0x0B;
const ELEMENT_R4: u8 = 0x0C;

/// Deepest crafting menu accepted.
const MAX_DEPTH: usize = 32;
/// Largest `params` array accepted.
const MAX_ARRAY: i32 = 4096;

fn err(message: impl Into<String>) -> Error {
    Error::new(0, message)
}

fn find_type(asm: &Assembly, namespace: &str, name: &str) -> Result<u32> {
    asm.find_type(namespace, name)?
        .ok_or_else(|| err(format!("no type {namespace}.{name}")))
}

fn method_code(asm: &Assembly, type_row: u32, name: &str) -> Result<Vec<Instr>> {
    let method = asm.find_method(type_row, name)?.ok_or_else(|| {
        err(format!(
            "type {}: no method {name}",
            asm.type_name(type_row).map(|t| t.1).unwrap_or("?")
        ))
    })?;
    method_instrs(asm, method)
}

fn method_instrs(asm: &Assembly, method: u32) -> Result<Vec<Instr>> {
    let (code, base) = asm
        .method_body(method)?
        .ok_or_else(|| err(format!("method {method} has no body")))?;
    decode(code, base)
}

/// The named values of an enum: its static literal fields and their
/// constants, in declaration order.
pub fn enum_values(asm: &Assembly, namespace: &str, name: &str) -> Result<Vec<(String, i64)>> {
    let ty = find_type(asm, namespace, name)?;
    let constants = asm.field_constants()?;
    let mut out = Vec::new();
    for field in asm.fields(ty)? {
        let flags = asm.field_flags(field)?;
        if flags & (FIELD_STATIC | FIELD_LITERAL) != FIELD_STATIC | FIELD_LITERAL {
            continue; // `value__`
        }
        let field_name = asm.field_name(field)?;
        let &(ty, bytes) = constants
            .get(&field)
            .ok_or_else(|| err(format!("{name}.{field_name}: no constant")))?;
        let int = |n: usize| -> Result<[u8; 8]> {
            let b = bytes.get(..n).filter(|_| bytes.len() == n).ok_or_else(|| {
                err(format!(
                    "{name}.{field_name}: constant of {} bytes",
                    bytes.len()
                ))
            })?;
            let mut a = [0u8; 8];
            a[..n].copy_from_slice(b);
            Ok(a)
        };
        let value = match ty {
            ELEMENT_I1 => i64::from(int(1)?[0] as i8),
            ELEMENT_U1 => i64::from(int(1)?[0]),
            ELEMENT_I2 => {
                let a = int(2)?;
                i64::from(i16::from_le_bytes([a[0], a[1]]))
            }
            ELEMENT_U2 => {
                let a = int(2)?;
                i64::from(u16::from_le_bytes([a[0], a[1]]))
            }
            ELEMENT_I4 => {
                let a = int(4)?;
                i64::from(i32::from_le_bytes([a[0], a[1], a[2], a[3]]))
            }
            ELEMENT_U4 => {
                let a = int(4)?;
                i64::from(u32::from_le_bytes([a[0], a[1], a[2], a[3]]))
            }
            ELEMENT_I8 | ELEMENT_U8 => i64::from_le_bytes(int(8)?),
            other => {
                return Err(err(format!(
                    "{name}.{field_name}: constant type {other:#x} is not an integer"
                )));
            }
        };
        out.push((field_name.to_string(), value));
    }
    Ok(out)
}

/// `TechType` value → name, in declaration order.
pub fn tech_type_names(asm: &Assembly) -> Result<Vec<(i32, String)>> {
    enum_values(asm, "", "TechType")?
        .into_iter()
        .map(|(name, v)| {
            i32::try_from(v)
                .map(|v| (v, name.clone()))
                .map_err(|_| err(format!("TechType.{name} = {v} does not fit i32")))
        })
        .collect()
}

/// One node of a crafting menu (`CraftNode`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftNode {
    pub id: String,
    /// `TreeAction` value (read its names with [`enum_values`]).
    pub action: i32,
    /// `TechType` value; 0 (`None`) for groups.
    pub tech_type: i32,
    pub children: Vec<CraftNode>,
}

impl CraftNode {
    /// This node and every node below it, depth first.
    pub fn walk(&self) -> Vec<&CraftNode> {
        let mut out = vec![self];
        let mut i = 0;
        while i < out.len() {
            let n = out[i];
            out.extend(n.children.iter());
            i += 1;
        }
        out
    }
}

/// A crafting machine's menu as `CraftTree.Initialize` builds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftTree {
    /// The tree's id (first argument of `new CraftTree`).
    pub id: String,
    /// The static field it is stored in.
    pub field: String,
    /// The method that builds its nodes.
    pub method: String,
    pub root: CraftNode,
}

/// The menus `CraftTree.Initialize` builds: each is `ldstr id; call
/// Scheme(); newobj CraftTree::.ctor; stsfld field`.
pub fn craft_trees(asm: &Assembly) -> Result<Vec<CraftTree>> {
    let ty = find_type(asm, "", "CraftTree")?;
    let init = method_code(asm, ty, "Initialize")?;
    let mut trees = Vec::new();
    for w in init.windows(4) {
        if !(w[0].opcode == LDSTR
            && w[1].opcode == CALL
            && w[2].opcode == NEWOBJ
            && w[3].opcode == STSFLD)
        {
            continue;
        }
        let ctor = asm.method_token(w[2].token().unwrap_or(0))?;
        if (ctor.type_name.as_str(), ctor.name.as_str(), ctor.params) != ("CraftTree", ".ctor", 2) {
            continue;
        }
        let call = w[1].token().unwrap_or(0);
        if call >> 24 != 0x06 {
            return Err(err(format!(
                "CraftTree.Initialize: scheme call {call:#x} is not a MethodDef"
            )));
        }
        let method = call & 0x00FF_FFFF;
        let field = asm.field_token(w[3].token().unwrap_or(0))?;
        trees.push(CraftTree {
            id: asm.user_string(w[0].token().unwrap_or(0))?,
            field: field.name,
            method: asm.method_name(method)?.to_string(),
            root: scheme(asm, method)?,
        });
    }
    // Every `new CraftTree` must be one of the trees read above.
    let mut ctors = 0;
    for i in init.iter().filter(|i| i.opcode == NEWOBJ) {
        let m = asm.method_token(i.token().unwrap_or(0))?;
        if (m.type_name.as_str(), m.name.as_str()) == ("CraftTree", ".ctor") {
            ctors += 1;
        }
    }
    if trees.is_empty() || ctors != trees.len() {
        return Err(err(format!(
            "CraftTree.Initialize: {ctors} trees built, {} read",
            trees.len()
        )));
    }
    Ok(trees)
}

#[derive(Clone, Debug)]
enum Value {
    Str(String),
    Int(i32),
    Node(usize),
    Array(usize),
}

struct Built {
    id: String,
    action: i32,
    tech_type: i32,
    children: Vec<usize>,
    has_parent: bool,
}

/// Runs a scheme method: string and integer constants, `new CraftNode(…)`,
/// `params` arrays (`newarr`, `dup`, index, value, `stelem.ref`) and
/// `AddNode(…)`, then `ret`. Any other instruction is an error.
fn scheme(asm: &Assembly, method: u32) -> Result<CraftNode> {
    let name = asm.method_name(method)?;
    let fail = |i: &Instr, what: &str| {
        err(format!(
            "{name} at IL {:#x} ({}): {what}",
            i.offset,
            opcode_name(i.opcode)
        ))
    };
    let mut stack: Vec<Value> = Vec::new();
    let mut nodes: Vec<Built> = Vec::new();
    let mut arrays: Vec<Vec<Option<usize>>> = Vec::new();
    let mut root = None;
    let instrs = method_instrs(asm, method)?;
    for i in &instrs {
        macro_rules! pop {
            () => {
                stack.pop().ok_or_else(|| fail(i, "stack is empty"))
            };
        }
        if let Some(v) = i.ldc_i4() {
            stack.push(Value::Int(v));
            continue;
        }
        match i.opcode {
            LDSTR => {
                let s = asm.user_string(i.token().unwrap_or(0))?;
                stack.push(Value::Str(s));
            }
            NEWOBJ => {
                let m = asm.method_token(i.token().unwrap_or(0))?;
                if m.type_name != "CraftNode" || m.name != ".ctor" || !(1..=3).contains(&m.params) {
                    return Err(fail(
                        i,
                        &format!("unexpected constructor {}::{}", m.type_name, m.name),
                    ));
                }
                let mut args = Vec::with_capacity(3);
                for _ in 0..m.params {
                    args.push(pop!()?);
                }
                args.reverse();
                let int = |v: Option<&Value>| match v {
                    None => Ok(0),
                    Some(Value::Int(n)) => Ok(*n),
                    Some(other) => Err(fail(i, &format!("argument {other:?} is not an integer"))),
                };
                let Some(Value::Str(id)) = args.first() else {
                    return Err(fail(i, "first argument is not a string"));
                };
                nodes.push(Built {
                    id: id.clone(),
                    action: int(args.get(1))?,
                    tech_type: int(args.get(2))?,
                    children: Vec::new(),
                    has_parent: false,
                });
                stack.push(Value::Node(nodes.len() - 1));
            }
            NEWARR => {
                let (_, t) = asm.type_token(i.token().unwrap_or(0))?;
                if t != "CraftNode" {
                    return Err(fail(i, &format!("array of {t}")));
                }
                let n = match pop!()? {
                    Value::Int(n) if (0..=MAX_ARRAY).contains(&n) => n as usize,
                    other => return Err(fail(i, &format!("array length {other:?}"))),
                };
                arrays.push(vec![None; n]);
                stack.push(Value::Array(arrays.len() - 1));
            }
            DUP => {
                let top = stack
                    .last()
                    .cloned()
                    .ok_or_else(|| fail(i, "stack is empty"))?;
                stack.push(top);
            }
            STELEM_REF => {
                let value = pop!()?;
                let index = pop!()?;
                let array = pop!()?;
                match (array, index, value) {
                    (Value::Array(a), Value::Int(k), Value::Node(n)) => {
                        let slot = usize::try_from(k)
                            .ok()
                            .and_then(|k| arrays[a].get_mut(k))
                            .ok_or_else(|| fail(i, &format!("index {k} out of the array")))?;
                        if slot.is_some() {
                            return Err(fail(i, "array slot set twice"));
                        }
                        *slot = Some(n);
                    }
                    other => return Err(fail(i, &format!("unexpected operands {other:?}"))),
                }
            }
            CALL | CALLVIRT => {
                let m = asm.method_token(i.token().unwrap_or(0))?;
                if m.type_name != "CraftNode" || m.name != "AddNode" || m.params != 1 {
                    return Err(fail(
                        i,
                        &format!("unexpected call {}::{}", m.type_name, m.name),
                    ));
                }
                let children = match pop!()? {
                    Value::Array(a) => arrays[a]
                        .iter()
                        .map(|c| c.ok_or_else(|| fail(i, "array slot left empty")))
                        .collect::<Result<Vec<_>>>()?,
                    Value::Node(n) => vec![n],
                    other => return Err(fail(i, &format!("argument {other:?}"))),
                };
                let Value::Node(parent) = pop!()? else {
                    return Err(fail(i, "AddNode on something that is not a node"));
                };
                for &c in &children {
                    if c == parent || nodes[c].has_parent {
                        return Err(fail(i, "node added twice"));
                    }
                    nodes[c].has_parent = true;
                }
                nodes[parent].children.extend(children);
                stack.push(Value::Node(parent));
            }
            RET => {
                let Some(Value::Node(n)) = stack.pop() else {
                    return Err(fail(i, "returns something that is not a node"));
                };
                if !stack.is_empty() || nodes[n].has_parent {
                    return Err(fail(i, "values left on the stack"));
                }
                root = Some(n);
                break;
            }
            _ => return Err(fail(i, "instruction not used by menu code")),
        }
    }
    let root = root.ok_or_else(|| err(format!("{name}: no ret")))?;
    let tree = build(&nodes, root, 0)?;
    let reached = tree.walk().len();
    if reached != nodes.len() {
        return Err(err(format!(
            "{name}: {} nodes built, {reached} in the tree",
            nodes.len()
        )));
    }
    Ok(tree)
}

fn build(nodes: &[Built], at: usize, depth: usize) -> Result<CraftNode> {
    if depth > MAX_DEPTH {
        return Err(err("crafting menu too deep"));
    }
    let n = &nodes[at];
    Ok(CraftNode {
        id: n.id.clone(),
        action: n.action,
        tech_type: n.tech_type,
        children: n
            .children
            .iter()
            .map(|&c| build(nodes, c, depth + 1))
            .collect::<Result<_>>()?,
    })
}

/// A default value from `TechData`'s static constructor.
#[derive(Clone, Debug, PartialEq)]
pub enum DefaultValue {
    /// Integers, enum values and bools (0 / 1).
    Int(i32),
    Float(f32),
    Str(String),
    /// A two-integer struct (`Vector2int`).
    Pair(i32, i32),
}

/// `TechData`'s `static readonly default…` fields as its static
/// constructor sets them: (field name, value), in order. Each is one
/// constant (`ldc.i4`, `ldc.r4`, `ldstr`, `ldsfld String.Empty`) or a
/// two-integer constructor, followed by `stsfld`. A `default…` field set
/// from `new List<…>(…)` (the list of default keys) is skipped.
pub fn tech_data_defaults(asm: &Assembly) -> Result<Vec<(String, DefaultValue)>> {
    let ty = find_type(asm, "", "TechData")?;
    let code = method_code(asm, ty, ".cctor")?;
    let mut out = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode != STSFLD {
            continue;
        }
        let f = asm.field_token(i.token().unwrap_or(0))?;
        let is_default = f.type_name == "TechData"
            && f.name
                .strip_prefix("default")
                .and_then(|rest| rest.chars().next())
                .is_some_and(|c| c.is_ascii_uppercase());
        if !is_default {
            continue;
        }
        let back = |n: usize| k.checked_sub(n).map(|j| &code[j]);
        let fail = || err(format!("TechData.{}: value pattern not recognised", f.name));
        let prev = back(1).ok_or_else(fail)?;
        let value = if let Some(v) = prev.ldc_i4() {
            DefaultValue::Int(v)
        } else {
            match (prev.opcode, &prev.operand) {
                (LDC_R4, Operand::F32(v)) => DefaultValue::Float(*v),
                (LDSTR, Operand::Token(t)) => DefaultValue::Str(asm.user_string(*t)?),
                (LDSFLD, Operand::Token(t)) => {
                    let s = asm.field_token(*t)?;
                    if (s.namespace.as_str(), s.type_name.as_str(), s.name.as_str())
                        != ("System", "String", "Empty")
                    {
                        return Err(fail());
                    }
                    DefaultValue::Str(String::new())
                }
                (NEWOBJ, Operand::Token(t)) => {
                    let m = asm.method_token(*t)?;
                    if (m.namespace.as_str(), m.type_name.as_str())
                        == ("System.Collections.Generic", "List`1")
                    {
                        // `defaultProperties = new List<string>(defaults.Keys)`:
                        // derived from the values above, not a value itself.
                        continue;
                    }
                    let a = back(3).and_then(Instr::ldc_i4);
                    let b = back(2).and_then(Instr::ldc_i4);
                    match (m.name.as_str(), m.params, a, b) {
                        (".ctor", 2, Some(a), Some(b)) => DefaultValue::Pair(a, b),
                        _ => return Err(fail()),
                    }
                }
                _ => return Err(fail()),
            }
        };
        out.push((f.name, value));
    }
    Ok(out)
}

/// The value of a `const float` field (its `Constant` row), e.g.
/// `GameInputSystem.defaultMouseSensitivity`.
pub fn const_f32(asm: &Assembly, namespace: &str, type_name: &str, field: &str) -> Result<f32> {
    let ty = find_type(asm, namespace, type_name)?;
    let constants = asm.field_constants()?;
    for row in asm.fields(ty)? {
        if asm.field_name(row)? != field {
            continue;
        }
        let what = || format!("{type_name}.{field}");
        let &(element, bytes) = constants
            .get(&row)
            .ok_or_else(|| err(format!("{}: no constant", what())))?;
        if element != ELEMENT_R4 {
            return Err(err(format!(
                "{}: constant type {element:#x} is not a float",
                what()
            )));
        }
        let b: [u8; 4] = bytes
            .try_into()
            .map_err(|_| err(format!("{}: constant of {} bytes", what(), bytes.len())))?;
        return Ok(f32::from_le_bytes(b));
    }
    Err(err(format!("{type_name}: no field {field}")))
}

/// The value an instance field gets in the type's constructor, from a
/// C# initialiser such as `private float rate = 30f;` (the compiler puts
/// it at the start of `.ctor` as `ldarg.0; ldc.r4 v; stfld field`). Only
/// that pattern is accepted, and it must occur exactly once.
pub fn field_initializer_f32(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    field: &str,
) -> Result<f32> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, ".ctor")?;
    let mut found = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode != STFLD {
            continue;
        }
        let f = asm.field_token(i.token().unwrap_or(0))?;
        if f.type_name != type_name || f.namespace != namespace || f.name != field {
            continue;
        }
        let fail = || {
            err(format!(
                "{type_name}.{field}: initialiser pattern not recognised"
            ))
        };
        let (Some(load), Some(value)) = (
            k.checked_sub(2).and_then(|j| code.get(j)),
            k.checked_sub(1).and_then(|j| code.get(j)),
        ) else {
            return Err(fail());
        };
        match (load.opcode, value.opcode, &value.operand) {
            (LDARG_0, LDC_R4, Operand::F32(v)) => found.push(*v),
            _ => return Err(fail()),
        }
    }
    match found[..] {
        [v] => Ok(v),
        [] => Err(err(format!("{type_name}.{field}: not set in .ctor"))),
        _ => Err(err(format!(
            "{type_name}.{field}: set {} times in .ctor",
            found.len()
        ))),
    }
}

/// The bounds of the one `UnityEngine.Random.Range(min, max)` call in a
/// method, both float constants (`ldc.r4 min; ldc.r4 max; call Range`),
/// e.g. `CrashedShipExploder.SetExplodeTime`'s `Random.Range(2.3f, 4f)`.
pub fn random_range_f32(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    method: &str,
) -> Result<(f32, f32)> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, method)?;
    let what = || format!("{type_name}.{method}");
    let mut found = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode != CALL {
            continue;
        }
        let m = asm.method_token(i.token().unwrap_or(0))?;
        if (m.namespace.as_str(), m.type_name.as_str(), m.name.as_str())
            != ("UnityEngine", "Random", "Range")
        {
            continue;
        }
        let arg = |n: usize| match k.checked_sub(n).and_then(|j| code.get(j)) {
            Some(Instr {
                opcode: LDC_R4,
                operand: Operand::F32(v),
                ..
            }) => Some(*v),
            _ => None,
        };
        match (m.params, arg(2), arg(1)) {
            (2, Some(min), Some(max)) => found.push((min, max)),
            _ => {
                return Err(err(format!(
                    "{}: Random.Range pattern not recognised",
                    what()
                )));
            }
        }
    }
    match found[..] {
        [v] => Ok(v),
        _ => Err(err(format!(
            "{}: {} Random.Range calls, expected 1",
            what(),
            found.len()
        ))),
    }
}

/// The float constant of the one `ldc.r4 v; mul` in a method, e.g. the
/// day length in `CrashedShipExploder.SetExplodeTime`'s `num * 1200f`.
pub fn multiplier_f32(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    method: &str,
) -> Result<f32> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, method)?;
    let found: Vec<f32> = code
        .windows(2)
        .filter_map(|w| match (&w[0], &w[1]) {
            (
                Instr {
                    opcode: LDC_R4,
                    operand: Operand::F32(v),
                    ..
                },
                Instr { opcode: MUL, .. },
            ) => Some(*v),
            _ => None,
        })
        .collect();
    match found[..] {
        [v] => Ok(v),
        _ => Err(err(format!(
            "{type_name}.{method}: {} constant multipliers, expected 1",
            found.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{Builder, Il};
    use crate::tables::Table;

    /// `public static literal`, and an enum's `value__`.
    const LITERAL: u16 = 0x0056;
    const VALUE_FIELD: u16 = 0x0606;
    const STATIC: u16 = 0x0016;
    const LDARG_0: u8 = 0x02;
    const LDNULL: u8 = 0x14;
    const MUL_OP: u8 = 0x5A;

    #[derive(Default)]
    struct Options {
        /// An instruction the menu reader does not accept, in the scheme.
        bad_scheme: bool,
        /// A `new CraftTree` outside the `ldstr; call; newobj; stsfld` pattern.
        stray_tree: bool,
        /// A `default…` field set from an unknown pattern.
        bad_default: bool,
    }

    fn enum_type(b: &mut Builder, name: &str, values: &[(&str, i32)]) {
        b.type_def("", name);
        b.field(VALUE_FIELD, "value__");
        for &(n, v) in values {
            let f = b.field(LITERAL, n);
            b.constant_i4(f, v);
        }
    }

    /// An assembly shaped like the game's: the enums, `CraftNode`,
    /// `CraftTree` with one scheme, and `TechData`'s static constructor.
    fn sample(o: &Options) -> Vec<u8> {
        let mut b = Builder::new();
        let list = b.type_ref("System.Collections.Generic", "List`1");
        let list = b.generic_type_spec(list);
        let list_ctor0 = b.member_ref((Table::TYPE_SPEC, list), ".ctor", Some(0));
        let list_ctor1 = b.member_ref((Table::TYPE_SPEC, list), ".ctor", Some(1));
        let string = b.type_ref("System", "String");
        let empty = b.member_ref((Table::TYPE_REF, string), "Empty", None);

        enum_type(
            &mut b,
            "TechType",
            &[("None", 0), ("Titanium", 5), ("Knife", 1000)],
        );
        enum_type(
            &mut b,
            "TreeAction",
            &[("None", 0), ("Expand", 1), ("Craft", 2)],
        );

        b.type_def("", "Vector2int");
        let pair_ctor = b.method(".ctor", 2, Some(&Il::default().ret().0));

        let node = 0x0200_0000 | b.type_def("", "CraftNode");
        let node_ctor1 = b.method(".ctor", 1, Some(&Il::default().ret().0));
        let node_ctor3 = b.method(".ctor", 3, Some(&Il::default().ret().0));
        let add = b.method("AddNode", 1, Some(&Il::default().op(LDARG_0).ret().0));

        b.type_def("", "CraftTree");
        let field = b.field(STATIC, "fabricator");
        let tree_ctor = b.method(".ctor", 2, Some(&Il::default().ret().0));
        let (root, group, knife, ti) = (
            b.user_string("Root"),
            b.user_string("Group"),
            b.user_string("Knife"),
            b.user_string("Ti"),
        );
        let mut scheme = Il::default()
            .ldstr(root)
            .newobj(node_ctor1)
            .ldc(2)
            .newarr(node)
            .dup()
            .ldc(0)
            .ldstr(group)
            .ldc(1)
            .ldc(0)
            .newobj(node_ctor3)
            .ldc(1)
            .newarr(node)
            .dup()
            .ldc(0)
            .ldstr(knife)
            .ldc(2)
            .ldc(1000)
            .newobj(node_ctor3)
            .stelem_ref()
            .callvirt(add)
            .stelem_ref()
            .dup()
            .ldc(1)
            .ldstr(ti)
            .ldc(2)
            .ldc(5)
            .newobj(node_ctor3)
            .stelem_ref()
            .call(add);
        if o.bad_scheme {
            scheme = scheme.op(LDARG_0).pop();
        }
        let scheme = b.method("FabricatorScheme", 0, Some(&scheme.ret().0));
        let id = b.user_string("Fabricator");
        let mut init = Il::default()
            .ldstr(id)
            .call(scheme)
            .newobj(tree_ctor)
            .stsfld(field)
            .newobj(list_ctor0)
            .pop();
        if o.stray_tree {
            init = init.ldstr(id).call(scheme).newobj(tree_ctor).pop();
        }
        b.method("Initialize", 0, Some(&init.ret().0));

        b.type_def("", "TechData");
        let names = [
            "defaultItemSize",
            "defaultCraftTime",
            "defaultBuildable",
            "defaultSoundUse",
            "defaultPoweredPrefab",
            "defaultProperties",
            "defaults",
        ];
        let f: Vec<u32> = names.iter().map(|n| b.field(STATIC, n)).collect();
        let sound = b.user_string("event:/test");
        let mut cctor = Il::default()
            .ldc(1)
            .ldc(2)
            .newobj(pair_ctor)
            .stsfld(f[0])
            .ldc_r4(2.5)
            .stsfld(f[1])
            .ldc(1)
            .stsfld(f[2])
            .ldstr(sound)
            .stsfld(f[3])
            .ldsfld(empty)
            .stsfld(f[4])
            .op(LDNULL)
            .newobj(list_ctor1)
            .stsfld(f[5])
            .ldc(7)
            .stsfld(f[6]);
        if o.bad_default {
            cctor = cctor.op(LDNULL).stsfld(f[1]);
        }
        b.method(".cctor", 0, Some(&cctor.ret().0));

        // `class Manager { const float defaultRate = 0.15f; float rate = 30f; }`
        b.type_def("", "Manager");
        let c = b.field(LITERAL, "defaultRate");
        b.constant_r4(c, 0.15);
        let rate = b.field(0x0001, "rate");
        let other = b.field(0x0001, "other");
        let mut ctor = Il::default()
            .op(LDARG_0)
            .ldc_r4(30.0)
            .stfld(rate)
            .op(LDARG_0)
            .op(LDNULL)
            .stfld(other);
        if o.bad_default {
            ctor = ctor.op(LDARG_0).ldc_r4(1.0).stfld(rate);
        }
        b.method(".ctor", 0, Some(&ctor.ret().0));

        // `void SetExplodeTime() { float n = Random.Range(2.3f, 4f); t = n * 1200f; }`
        let random = b.type_ref("UnityEngine", "Random");
        let range = b.member_ref((Table::TYPE_REF, random), "Range", Some(2));
        b.type_def("", "Exploder");
        let t = b.field(0x0001, "t");
        let mut code = Il::default()
            .ldc_r4(2.3)
            .ldc_r4(4.0)
            .call(range)
            .ldc_r4(1200.0)
            .op(MUL_OP)
            .stfld(t);
        if o.bad_default {
            code = code
                .ldc_r4(1.0)
                .ldc_r4(1.5)
                .call(range)
                .ldc_r4(2.0)
                .op(MUL_OP)
                .pop();
        }
        b.method("SetExplodeTime", 0, Some(&code.ret().0));
        b.build()
    }

    fn parse(bytes: &[u8]) -> Assembly<'_> {
        Assembly::parse(bytes).unwrap()
    }

    #[test]
    fn reads_enum_names_in_order() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        assert_eq!(asm.runtime_version, "v4.0.30319");
        assert_eq!(
            tech_type_names(&asm).unwrap(),
            vec![
                (0, "None".into()),
                (5, "Titanium".into()),
                (1000, "Knife".into())
            ]
        );
        assert_eq!(enum_values(&asm, "", "TreeAction").unwrap().len(), 3);
        assert!(enum_values(&asm, "", "Missing").is_err());
    }

    #[test]
    fn runs_the_craft_tree_scheme() {
        let bytes = sample(&Options::default());
        let trees = craft_trees(&parse(&bytes)).unwrap();
        let node = |id: &str, action, tech_type, children| CraftNode {
            id: id.into(),
            action,
            tech_type,
            children,
        };
        assert_eq!(
            trees,
            vec![CraftTree {
                id: "Fabricator".into(),
                field: "fabricator".into(),
                method: "FabricatorScheme".into(),
                root: node(
                    "Root",
                    0,
                    0,
                    vec![
                        node("Group", 1, 0, vec![node("Knife", 2, 1000, vec![])]),
                        node("Ti", 2, 5, vec![]),
                    ]
                ),
            }]
        );
        assert_eq!(trees[0].root.walk().len(), 4);
    }

    #[test]
    fn unknown_menu_code_is_an_error() {
        let bad = sample(&Options {
            bad_scheme: true,
            ..Options::default()
        });
        let e = craft_trees(&parse(&bad)).unwrap_err();
        assert!(e.message.contains("not used by menu code"), "{e}");
        let stray = sample(&Options {
            stray_tree: true,
            ..Options::default()
        });
        let e = craft_trees(&parse(&stray)).unwrap_err();
        assert!(e.message.contains("2 trees built, 1 read"), "{e}");
    }

    #[test]
    fn reads_tech_data_defaults() {
        let bytes = sample(&Options::default());
        assert_eq!(
            tech_data_defaults(&parse(&bytes)).unwrap(),
            vec![
                ("defaultItemSize".into(), DefaultValue::Pair(1, 2)),
                ("defaultCraftTime".into(), DefaultValue::Float(2.5)),
                ("defaultBuildable".into(), DefaultValue::Int(1)),
                (
                    "defaultSoundUse".into(),
                    DefaultValue::Str("event:/test".into())
                ),
                (
                    "defaultPoweredPrefab".into(),
                    DefaultValue::Str(String::new())
                ),
            ]
        );
        let bad = sample(&Options {
            bad_default: true,
            ..Options::default()
        });
        assert!(tech_data_defaults(&parse(&bad)).is_err());
    }

    #[test]
    fn reads_float_constants_and_initialisers() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        assert_eq!(const_f32(&asm, "", "Manager", "defaultRate").unwrap(), 0.15);
        assert!(const_f32(&asm, "", "Manager", "rate").is_err());
        assert!(const_f32(&asm, "", "TechType", "Knife").is_err());
        assert_eq!(
            field_initializer_f32(&asm, "", "Manager", "rate").unwrap(),
            30.0
        );
        // Set from something other than a float constant.
        assert!(field_initializer_f32(&asm, "", "Manager", "other").is_err());
        assert!(field_initializer_f32(&asm, "", "Manager", "missing").is_err());
        let bad = sample(&Options {
            bad_default: true,
            ..Options::default()
        });
        let e = field_initializer_f32(&parse(&bad), "", "Manager", "rate").unwrap_err();
        assert!(e.message.contains("set 2 times"), "{e}");
    }

    #[test]
    fn reads_random_ranges_and_multipliers() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        assert_eq!(
            random_range_f32(&asm, "", "Exploder", "SetExplodeTime").unwrap(),
            (2.3, 4.0)
        );
        assert_eq!(
            multiplier_f32(&asm, "", "Exploder", "SetExplodeTime").unwrap(),
            1200.0
        );
        assert!(random_range_f32(&asm, "", "Exploder", "Missing").is_err());
        // No Random.Range, no multiplier.
        assert!(random_range_f32(&asm, "", "Manager", ".ctor").is_err());
        assert!(multiplier_f32(&asm, "", "Manager", ".ctor").is_err());
        // Two of each: an error, not a guess.
        let bad = sample(&Options {
            bad_default: true,
            ..Options::default()
        });
        let asm = parse(&bad);
        let e = random_range_f32(&asm, "", "Exploder", "SetExplodeTime").unwrap_err();
        assert!(e.message.contains("2 Random.Range calls"), "{e}");
        let e = multiplier_f32(&asm, "", "Exploder", "SetExplodeTime").unwrap_err();
        assert!(e.message.contains("2 constant multipliers"), "{e}");
    }

    #[test]
    fn damaged_assemblies_never_panic() {
        let bytes = sample(&Options::default());
        let read = |b: &[u8]| {
            if let Ok(asm) = Assembly::parse(b) {
                let _ = tech_type_names(&asm);
                let _ = craft_trees(&asm);
                let _ = tech_data_defaults(&asm);
                let _ = const_f32(&asm, "", "Manager", "defaultRate");
                let _ = field_initializer_f32(&asm, "", "Manager", "rate");
                let _ = random_range_f32(&asm, "", "Exploder", "SetExplodeTime");
                let _ = multiplier_f32(&asm, "", "Exploder", "SetExplodeTime");
                for m in 1..=asm.rows(Table::METHOD_DEF) {
                    if let Ok(Some((code, base))) = asm.method_body(m) {
                        let _ = decode(code, base);
                    }
                }
            }
        };
        for n in 0..bytes.len() {
            read(&bytes[..n]);
        }
        for i in 0..bytes.len() {
            for x in [0x01, 0x80, 0xFF] {
                let mut b = bytes.clone();
                b[i] ^= x;
                read(&b);
            }
        }
    }
}
