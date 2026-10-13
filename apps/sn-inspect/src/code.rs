//! `code`: what the game keeps only in its code (`Assembly-CSharp.dll`),
//! read with `sn-dotnet` (Phase E, P1).

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::Assets;
use sn_dotnet::{Assembly, CraftNode, Operand, Table, decode, opcode_name};
use sn_install::GameData;

use crate::Result;

/// `code [--trees]`: assembly totals, every method body decoded, the
/// `TechType` names, the crafting menus (with `--trees`, every node) and
/// `TechData`'s defaults, checked against `Balance/TechData`.
pub fn run(game: &GameData, print_trees: bool) -> Result<ExitCode> {
    let start = Instant::now();
    let bytes = sn_assets::read_assembly(game)?;
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    println!(
        "{}: {} bytes, runtime {}",
        sn_assets::GAME_ASSEMBLY,
        bytes.len(),
        asm.runtime_version
    );
    for (name, t) in [
        ("TypeDef", Table::TYPE_DEF),
        ("Field", Table::FIELD),
        ("MethodDef", Table::METHOD_DEF),
        ("MemberRef", Table::MEMBER_REF),
        ("Constant", Table::CONSTANT),
        ("TypeRef", Table::TYPE_REF),
    ] {
        println!("  {name:<10} {}", asm.rows(t));
    }
    // Every method body splits into instructions to its last byte.
    let (mut bodies, mut instrs, mut errors) = (0usize, 0usize, Vec::new());
    for m in 1..=asm.rows(Table::METHOD_DEF) {
        match asm.method_body(m) {
            Ok(Some((code, base))) => {
                bodies += 1;
                match decode(code, base) {
                    Ok(i) => instrs += i.len(),
                    Err(e) => errors.push(format!("method {m}: {e}")),
                }
            }
            Ok(None) => {}
            Err(e) => errors.push(format!("method {m}: {e}")),
        }
    }
    println!(
        "method bodies: {bodies}, instructions: {instrs}, errors: {}",
        errors.len()
    );
    for e in errors.iter().take(10) {
        println!("  {e}");
    }

    let code = sn_assets::game_code(&bytes)?;
    let distinct: std::collections::BTreeSet<i32> = code.tech_types.iter().map(|t| t.0).collect();
    println!(
        "TechType names: {}, distinct values: {}",
        code.tech_types.len(),
        distinct.len()
    );
    println!("TreeAction: {:?}", code.tree_actions);
    let craft = code.tree_action("Craft").ok_or("TreeAction has no Craft")?;
    let assets = Assets::index(game)?;
    let tech = sn_assets::tech_data(&assets)?;
    let unnamed: Vec<i32> = tech
        .entries
        .iter()
        .map(|e| e.tech_type)
        .filter(|t| code.tech_name(*t).is_none())
        .collect();
    println!(
        "TechData entries without a TechType name: {} {unnamed:?}",
        unnamed.len()
    );
    let mut total = 0;
    for tree in &code.craft_trees {
        let nodes = tree.root.walk();
        let crafts: Vec<&&CraftNode> = nodes.iter().filter(|n| n.action == craft).collect();
        let no_recipe: Vec<String> = crafts
            .iter()
            .filter(|n| tech.get(n.tech_type).is_none())
            .map(|n| name(&code, n.tech_type))
            .collect();
        total += nodes.len();
        println!(
            "tree {:<18} ({}, field {}): {} nodes, {} craft nodes; craft nodes without TechData: {} {no_recipe:?}",
            tree.id,
            tree.method,
            tree.field,
            nodes.len(),
            crafts.len(),
            no_recipe.len()
        );
        if print_trees {
            print_node(&code, &tree.root, 1);
        }
    }
    println!("craft tree nodes: {total}");
    let mut by_action: BTreeMap<i32, usize> = BTreeMap::new();
    for tree in &code.craft_trees {
        for n in tree.root.walk() {
            *by_action.entry(n.action).or_default() += 1;
        }
    }
    println!("nodes per TreeAction value: {by_action:?}");
    println!("TechData defaults: {:?}", code.tech_defaults);
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn name(code: &sn_assets::GameCode, t: i32) -> String {
    code.tech_name(t)
        .map_or_else(|| format!("#{t}"), String::from)
}

fn print_node(code: &sn_assets::GameCode, n: &CraftNode, depth: usize) {
    for c in &n.children {
        let tech = if c.tech_type != 0 {
            format!(" -> {}", name(code, c.tech_type))
        } else {
            String::new()
        };
        println!("{}{} [{}]{tech}", "  ".repeat(depth), c.id, c.action);
        print_node(code, c, depth + 1);
    }
}

/// `code --il <Type> <Method>`: one method's instructions with their
/// tokens resolved (printed only, for reading the patterns P1 accepts).
pub fn il(game: &GameData, type_name: &str, method: &str, params: Option<u32>) -> Result<ExitCode> {
    // `ASSEMBLY:TYPE` reads `Managed/ASSEMBLY.dll` instead of the game's.
    let (bytes, type_name) = match type_name.split_once(':') {
        Some((assembly, ty)) => (
            game.read_file(
                &game
                    .data_dir
                    .join("Managed")
                    .join(format!("{assembly}.dll")),
            )?,
            ty,
        ),
        None => (sn_assets::read_assembly(game)?, type_name),
    };
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let ty = resolve_type(&asm, type_name)?;
    // `NAME#N`: the Nth method of that name (overloads in declaration
    // order, from 1).
    let (name, nth) = match method.rsplit_once('#') {
        Some((n, k)) => (
            n,
            k.parse::<usize>()
                .map_err(|_| format!("bad overload {k:?}"))?,
        ),
        None => (method, 1),
    };
    let found = match params {
        Some(n) if nth == 1 => asm.find_method_with(ty, name, n),
        _ => {
            let mut seen = 0;
            let mut found = None;
            for m in asm.methods(ty).map_err(|e| e.to_string())? {
                if asm.method_name(m).map_err(|e| e.to_string())? == name {
                    seen += 1;
                    if seen == nth {
                        found = Some(m);
                        break;
                    }
                }
            }
            Ok(found)
        }
    };
    let m = found
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{type_name}: no method {method}"))?;
    let (code, base) = asm
        .method_body(m)
        .map_err(|e| e.to_string())?
        .ok_or("method has no body")?;
    for i in decode(code, base).map_err(|e| e.to_string())? {
        let operand = match (&i.operand, i.ldc_i4()) {
            (_, Some(v)) => v.to_string(),
            (Operand::Token(t), _) => describe_token(&asm, *t),
            (Operand::None, _) => String::new(),
            (other, _) => format!("{other:?}"),
        };
        println!(
            "IL_{:04x}  {:<12} {operand}",
            i.offset,
            opcode_name(i.opcode)
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// `code --enum <TYPE>`: an enum's names and values, in declaration
/// order.
pub fn enum_values(game: &GameData, type_name: &str) -> Result<ExitCode> {
    let bytes = sn_assets::read_assembly(game)?;
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let ty = resolve_type(&asm, type_name)?;
    let values = sn_dotnet::enum_values_of(&asm, ty, type_name).map_err(|e| e.to_string())?;
    for (name, value) in values {
        println!("{value:>12}  {name}");
    }
    Ok(ExitCode::SUCCESS)
}

/// A type by name: `Namespace.Name`, or `Outer/Inner` for a nested one
/// (a compiler-made coroutine's `<Method>d__N`, a nested enum).
fn resolve_type(asm: &Assembly, type_name: &str) -> Result<u32> {
    let (outer, inner) = match type_name.split_once('/') {
        Some((o, i)) => (o, Some(i)),
        None => (type_name, None),
    };
    let (ns, n) = outer.rsplit_once('.').unwrap_or(("", outer));
    let mut ty = asm
        .find_type(ns, n)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no type {outer}"))?;
    if let Some(i) = inner {
        ty = asm
            .find_nested_type(ty, i)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no type {type_name}"))?;
    }
    Ok(ty)
}

/// `code --implements <INTERFACE> [<ASSEMBLY>]`: every type of the game's
/// assembly (or `Managed/ASSEMBLY.dll`) that declares an interface of
/// that name.
pub fn implements(game: &GameData, interface: &str, assembly: Option<&str>) -> Result<ExitCode> {
    let bytes = match assembly {
        Some(a) => game.read_file(&game.data_dir.join("Managed").join(format!("{a}.dll")))?,
        None => sn_assets::read_assembly(game)?,
    };
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let e = |e: sn_dotnet::Error| e.to_string();
    let mut found = 0;
    for row in 1..=asm.rows(sn_dotnet::Table::TYPE_DEF) {
        if !asm
            .interfaces(row)
            .map_err(e)?
            .iter()
            .any(|(_, n)| n == interface)
        {
            continue;
        }
        let (ns, n) = asm.type_name(row).map_err(e)?;
        let base = asm
            .base_type(row)
            .map_err(e)?
            .map_or("nothing".to_string(), |(bns, bn)| format!("{bns}.{bn}"));
        println!("{ns}.{n} extends {base}");
        found += 1;
    }
    println!("{found} types declare {interface}");
    Ok(ExitCode::SUCCESS)
}

/// `code --fields <TYPE>`: a type's fields in declaration order with
/// their flags and declared types, the type it extends (M9u0), and its
/// methods' names.
pub fn fields(game: &GameData, type_name: &str) -> Result<ExitCode> {
    // `ASSEMBLY:TYPE` reads `Managed/ASSEMBLY.dll` instead of the game's.
    let (bytes, type_name) = match type_name.split_once(':') {
        Some((assembly, ty)) => (
            game.read_file(
                &game
                    .data_dir
                    .join("Managed")
                    .join(format!("{assembly}.dll")),
            )?,
            ty,
        ),
        None => (sn_assets::read_assembly(game)?, type_name),
    };
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let ty = resolve_type(&asm, type_name)?;
    let e = |e: sn_dotnet::Error| e.to_string();
    match asm.base_type(ty).map_err(e)? {
        Some((ns, n)) if ns.is_empty() => println!("{type_name} extends {n}"),
        Some((ns, n)) => println!("{type_name} extends {ns}.{n}"),
        None => println!("{type_name} extends nothing"),
    }
    let interfaces = asm.interfaces(ty).map_err(|e| e.to_string())?;
    if !interfaces.is_empty() {
        let names: Vec<String> = interfaces
            .iter()
            .map(|(ns, n)| {
                if ns.is_empty() {
                    n.clone()
                } else {
                    format!("{ns}.{n}")
                }
            })
            .collect();
        println!("implements {}", names.join(", "));
    }
    for row in asm.fields(ty).map_err(e)? {
        let flags = asm.field_flags(row).map_err(e)?;
        let mut marks = Vec::new();
        if flags & sn_dotnet::FIELD_ACCESS_MASK == sn_dotnet::FIELD_PUBLIC {
            marks.push("public");
        }
        for (bit, name) in [
            (sn_dotnet::FIELD_STATIC, "static"),
            (sn_dotnet::FIELD_INIT_ONLY, "readonly"),
            (sn_dotnet::FIELD_LITERAL, "const"),
            (sn_dotnet::FIELD_NOT_SERIALIZED, "[NonSerialized]"),
        ] {
            if flags & bit != 0 {
                marks.push(name);
            }
        }
        let ty = asm
            .field_type(row)
            .map_or_else(|err| format!("[{err}]"), |t| t.to_string());
        println!(
            "  {:<32} {:<40} {}",
            asm.field_name(row).map_err(e)?,
            ty,
            marks.join(" ")
        );
    }
    let methods: Vec<&str> = asm
        .methods(ty)
        .map_err(e)?
        .map(|m| asm.method_name(m))
        .collect::<std::result::Result<_, _>>()
        .map_err(e)?;
    println!("methods: {}", methods.join(", "));
    Ok(ExitCode::SUCCESS)
}

/// `code --nested <TYPE>`: the types nested in a type and their methods.
pub fn nested(game: &GameData, type_name: &str) -> Result<ExitCode> {
    let bytes = sn_assets::read_assembly(game)?;
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let ty = resolve_type(&asm, type_name)?;
    let e = |e: sn_dotnet::Error| e.to_string();
    for row in asm.nested_types(ty).map_err(e)? {
        let methods: Vec<&str> = asm
            .methods(row)
            .map_err(e)?
            .map(|m| asm.method_name(m))
            .collect::<std::result::Result<_, _>>()
            .map_err(e)?;
        println!(
            "{}/{}: {}",
            type_name,
            asm.type_name(row).map_err(e)?.1,
            methods.join(", ")
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn describe_token(asm: &Assembly, t: u32) -> String {
    let r = match t >> 24 {
        0x70 => asm.user_string(t).map(|s| format!("{s:?}")),
        0x04 => asm
            .field_token(t)
            .map(|f| format!("field {}.{}::{}", f.namespace, f.type_name, f.name)),
        0x06 | 0x0A => asm.method_token(t).map(|m| {
            format!(
                "{}.{}::{} ({} params)",
                m.namespace, m.type_name, m.name, m.params
            )
        }),
        0x01 | 0x02 | 0x1B => asm.type_token(t).map(|(ns, n)| format!("type {ns}.{n}")),
        _ => Ok(String::new()),
    };
    match r {
        Ok(s) => format!("{t:#010x} {s}"),
        Err(e) => format!("{t:#010x} <{e}>"),
    }
}
