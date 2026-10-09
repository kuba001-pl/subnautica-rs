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
pub fn il(game: &GameData, type_name: &str, method: &str) -> Result<ExitCode> {
    let bytes = sn_assets::read_assembly(game)?;
    let asm = Assembly::parse(&bytes).map_err(|e| e.to_string())?;
    let (ns, n) = type_name.rsplit_once('.').unwrap_or(("", type_name));
    let ty = asm
        .find_type(ns, n)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no type {type_name}"))?;
    let m = asm
        .find_method(ty, method)
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
