//! Item rules the game keeps only in code (M9d1): the inventory's size
//! (`new ItemsContainer(6, 8, …)` in `Inventory.Awake`), the equipment
//! slot table (`Equipment.slotMapping`, a static dictionary) and the
//! player's default slots (`Inventory.UnlockDefaultEquipmentSlots`, a
//! string array). As in `game.rs`, each reader accepts only the pattern
//! the game's compiler produced and fails on anything else.

use crate::assembly::Assembly;
use crate::game::{err, find_type, method_code};
use crate::il::{CALLVIRT, DUP, Instr, LDSTR, NEWARR, NEWOBJ, STELEM_REF, STFLD, STSFLD};
use crate::{Error, Result};

/// Largest table or array accepted.
const MAX_ITEMS: usize = 4096;

/// A constructor argument pushed by one instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    Int(i32),
    Str(String),
    Null,
    /// A local or an argument (`ldloc`, `ldarg`): not a constant.
    Variable,
}

fn arg(asm: &Assembly, i: &Instr) -> Result<Option<Arg>> {
    if let Some(v) = i.ldc_i4() {
        return Ok(Some(Arg::Int(v)));
    }
    Ok(match i.opcode {
        LDSTR => Some(Arg::Str(asm.user_string(i.token().unwrap_or(0))?)),
        0x14 => Some(Arg::Null),
        // ldarg.0–3, ldloc.0–3, ldarg.s, ldloc.s
        0x02..=0x09 | 0x0E | 0x11 => Some(Arg::Variable),
        _ => None,
    })
}

/// The arguments of the one `new T(…)` in `type_name.method` whose result
/// is stored into the field `field` of `type_name` (`newobj T::.ctor;
/// stfld field`), each pushed by a single instruction, e.g.
/// `_container = new ItemsContainer(6, 8, root, "InventoryLabel", null)`.
pub fn constructor_args(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    method: &str,
    constructed: &str,
    field: &str,
) -> Result<Vec<Arg>> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, method)?;
    let what = || format!("{type_name}.{method}: new {constructed} into {field}");
    let mut found = Vec::new();
    for (k, pair) in code.windows(2).enumerate() {
        if pair[0].opcode != NEWOBJ || pair[1].opcode != STFLD {
            continue;
        }
        let ctor = asm.method_token(pair[0].token().unwrap_or(0))?;
        let f = asm.field_token(pair[1].token().unwrap_or(0))?;
        if ctor.type_name != constructed
            || ctor.name != ".ctor"
            || f.type_name != type_name
            || f.name != field
        {
            continue;
        }
        let n = ctor.params as usize;
        let first = k
            .checked_sub(n)
            .ok_or_else(|| err(format!("{}: arguments before the method start", what())))?;
        let mut args = Vec::with_capacity(n);
        for i in &code[first..k] {
            args.push(arg(asm, i)?.ok_or_else(|| {
                err(format!(
                    "{}: argument pattern not recognised at IL_{:04x}",
                    what(),
                    i.offset
                ))
            })?);
        }
        found.push(args);
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(err(format!("{}: found {n} times, expected 1", what()))),
    }
}

/// The entries of a static `Dictionary<string, int-like>` built by a
/// collection initialiser in the type's static constructor: `newobj
/// Dictionary::.ctor()`, then per entry `dup; ldstr key; ldc.i4 value;
/// callvirt Add`, then `stsfld field`. In source order.
pub fn static_string_int_dictionary(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    field: &str,
) -> Result<Vec<(String, i32)>> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, ".cctor")?;
    let what = || format!("{type_name}.{field}");
    let mut stores = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode == STSFLD {
            let f = asm.field_token(i.token().unwrap_or(0))?;
            if f.type_name == type_name && f.namespace == namespace && f.name == field {
                stores.push(k);
            }
        }
    }
    let end = match stores[..] {
        [k] => k,
        _ => {
            return Err(err(format!(
                "{}: set {} times in .cctor",
                what(),
                stores.len()
            )));
        }
    };
    // Walk back over the `dup; ldstr; ldc.i4; callvirt Add` groups.
    let mut entries = Vec::new();
    let mut at = end;
    while let Some(start) = at.checked_sub(4) {
        let g = &code[start..at];
        if g[0].opcode != DUP || g[1].opcode != LDSTR || g[3].opcode != CALLVIRT {
            break;
        }
        let add = asm.method_token(g[3].token().unwrap_or(0))?;
        let value = g[2].ldc_i4();
        match (add.name.as_str(), add.params, value) {
            ("Add", 2, Some(v)) => {
                entries.push((asm.user_string(g[1].token().unwrap_or(0))?, v));
            }
            _ => break,
        }
        if entries.len() > MAX_ITEMS {
            return Err(err(format!("{}: more than {MAX_ITEMS} entries", what())));
        }
        at = start;
    }
    let ctor = at
        .checked_sub(1)
        .and_then(|j| code.get(j))
        .filter(|i| i.opcode == NEWOBJ)
        .map(|i| asm.method_token(i.token().unwrap_or(0)))
        .transpose()?;
    match ctor {
        Some(c) if c.type_name.starts_with("Dictionary`") && c.name == ".ctor" && c.params == 0 => {
        }
        _ => {
            return Err(err(format!(
                "{}: dictionary initialiser pattern not recognised",
                what()
            )));
        }
    }
    entries.reverse();
    Ok(entries)
}

/// The entries of a static collection built by a collection initialiser
/// in the type's static constructor, whatever its type: `newobj
/// T::.ctor()` (no arguments), then per entry `dup`, each argument as one
/// constant (`ldc.i4` or `ldstr`), `callvirt Add`, then `stsfld field`.
/// Each entry's arguments in order, entries in source order. E.g.
/// `GameInputSystem.bindingsKeyboard` (M9u6d2): `{ Button.LeftHand,
/// "<Mouse>/leftButton" }`, some with a second binding.
pub fn static_collection(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    field: &str,
) -> Result<Vec<Vec<Arg>>> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, ".cctor")?;
    let what = || format!("{type_name}.{field}");
    let mut stores = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode == STSFLD {
            let f = asm.field_token(i.token().unwrap_or(0))?;
            if f.type_name == type_name && f.namespace == namespace && f.name == field {
                stores.push(k);
            }
        }
    }
    let end = match stores[..] {
        [k] => k,
        _ => {
            return Err(err(format!(
                "{}: set {} times in .cctor",
                what(),
                stores.len()
            )));
        }
    };
    let fail = || {
        err(format!(
            "{}: collection initialiser pattern not recognised",
            what()
        ))
    };
    // Walk back over the `dup; args…; callvirt Add` groups to the `newobj`.
    let mut entries = Vec::new();
    let mut at = end;
    loop {
        let last = at
            .checked_sub(1)
            .and_then(|j| code.get(j))
            .ok_or_else(fail)?;
        if last.opcode == NEWOBJ {
            let c = asm.method_token(last.token().unwrap_or(0))?;
            if c.name != ".ctor" || c.params != 0 {
                return Err(fail());
            }
            break;
        }
        if last.opcode != CALLVIRT {
            return Err(fail());
        }
        let add = asm.method_token(last.token().unwrap_or(0))?;
        let n = add.params as usize;
        if add.name != "Add" || n == 0 {
            return Err(fail());
        }
        let first = (at - 1).checked_sub(n).ok_or_else(fail)?;
        let mut args = Vec::with_capacity(n);
        for i in &code[first..at - 1] {
            match arg(asm, i)? {
                Some(a @ (Arg::Int(_) | Arg::Str(_))) => args.push(a),
                _ => return Err(fail()),
            }
        }
        let dup = first.checked_sub(1).ok_or_else(fail)?;
        if code[dup].opcode != DUP {
            return Err(fail());
        }
        entries.push(args);
        if entries.len() > MAX_ITEMS {
            return Err(err(format!("{}: more than {MAX_ITEMS} entries", what())));
        }
        at = dup;
    }
    entries.reverse();
    Ok(entries)
}

/// The strings of the one `new string[n] { … }` in a method: `ldc.i4 n;
/// newarr String`, then per element `dup; ldc.i4 index; ldstr s;
/// stelem.ref` with the indices 0, 1, …, n − 1 in order.
pub fn string_array(
    asm: &Assembly,
    namespace: &str,
    type_name: &str,
    method: &str,
) -> Result<Vec<String>> {
    let ty = find_type(asm, namespace, type_name)?;
    let code = method_code(asm, ty, method)?;
    let what = || format!("{type_name}.{method}");
    let mut found = Vec::new();
    for (k, i) in code.iter().enumerate() {
        if i.opcode != NEWARR {
            continue;
        }
        let (ns, name) = asm.type_token(i.token().unwrap_or(0))?;
        if (ns.as_str(), name.as_str()) != ("System", "String") {
            continue;
        }
        let bad = || -> Error { err(format!("{}: string array pattern not recognised", what())) };
        let n = k
            .checked_sub(1)
            .and_then(|j| code.get(j))
            .and_then(Instr::ldc_i4)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n <= MAX_ITEMS)
            .ok_or_else(bad)?;
        let mut items = Vec::with_capacity(n);
        for index in 0..n {
            let at = k + 1 + index * 4;
            let g = code.get(at..at + 4).ok_or_else(bad)?;
            let ok = g[0].opcode == DUP
                && g[1].ldc_i4() == i32::try_from(index).ok()
                && g[2].opcode == LDSTR
                && g[3].opcode == STELEM_REF;
            if !ok {
                return Err(bad());
            }
            items.push(asm.user_string(g[2].token().unwrap_or(0))?);
        }
        found.push(items);
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(err(format!("{}: {n} string arrays, expected 1", what()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{Builder, Il};
    use crate::tables::Table;

    const STATIC: u16 = 0x0016;
    const INSTANCE: u16 = 0x0001;
    const LDARG_0: u8 = 0x02;
    const LDLOC_0: u8 = 0x06;
    const LDNULL: u8 = 0x14;

    #[derive(Default)]
    struct Options {
        /// A second `new ItemsContainer` into the same field.
        twice: bool,
        /// A dictionary value that is not a constant.
        bad_value: bool,
        /// Array indices out of order.
        bad_index: bool,
    }

    fn sample(o: &Options) -> Vec<u8> {
        let mut b = Builder::new();
        let dict = b.type_ref("System.Collections.Generic", "Dictionary`2");
        let dict = b.generic_type_spec(dict);
        let dict_ctor = b.member_ref((Table::TYPE_SPEC, dict), ".ctor", Some(0));
        let dict_add = b.member_ref((Table::TYPE_SPEC, dict), "Add", Some(2));
        let string = b.type_ref("System", "String");
        let string = 0x0100_0000 | string;

        b.type_def("", "ItemsContainer");
        let ctor = b.method(".ctor", 5, Some(&Il::default().ret().0));

        b.type_def("", "Inventory");
        let container = b.field(INSTANCE, "_container");
        let label = b.user_string("InventoryLabel");
        let mut awake = Il::default()
            .op(LDARG_0)
            .ldc(6)
            .ldc(8)
            .op(LDLOC_0)
            .ldstr(label)
            .op(LDNULL)
            .newobj(ctor)
            .stfld(container);
        if o.twice {
            awake = awake
                .op(LDARG_0)
                .ldc(1)
                .ldc(1)
                .op(LDLOC_0)
                .ldstr(label)
                .op(LDNULL)
                .newobj(ctor)
                .stfld(container);
        }
        b.method("Awake", 0, Some(&awake.ret().0));
        let (head, tank) = (b.user_string("Head"), b.user_string("Tank"));
        let (i0, i1) = if o.bad_index { (1, 0) } else { (0, 1) };
        let slots = Il::default()
            .ldc(2)
            .newarr(string)
            .dup()
            .ldc(i0)
            .ldstr(head)
            .stelem_ref()
            .dup()
            .ldc(i1)
            .ldstr(tank)
            .stelem_ref()
            .pop();
        b.method("UnlockDefaultEquipmentSlots", 0, Some(&slots.ret().0));

        b.type_def("", "Equipment");
        let mapping = b.field(STATIC, "slotMapping");
        let other = b.field(STATIC, "other");
        let mut cctor = Il::default()
            .newobj(dict_ctor)
            .dup()
            .ldstr(head)
            .ldc(2)
            .callvirt(dict_add)
            .dup()
            .ldstr(tank);
        cctor = if o.bad_value {
            cctor.op(LDLOC_0)
        } else {
            cctor.ldc(6)
        };
        cctor = cctor
            .callvirt(dict_add)
            .stsfld(mapping)
            .op(LDNULL)
            .stsfld(other);
        b.method(".cctor", 0, Some(&cctor.ret().0));
        b.build()
    }

    fn parse(bytes: &[u8]) -> Assembly<'_> {
        Assembly::parse(bytes).unwrap()
    }

    #[test]
    fn reads_constructor_arguments() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        let args = constructor_args(
            &asm,
            "",
            "Inventory",
            "Awake",
            "ItemsContainer",
            "_container",
        )
        .unwrap();
        assert_eq!(
            args,
            [
                Arg::Int(6),
                Arg::Int(8),
                Arg::Variable,
                Arg::Str("InventoryLabel".into()),
                Arg::Null
            ]
        );
        assert!(
            constructor_args(&asm, "", "Inventory", "Awake", "ItemsContainer", "other").is_err()
        );
        let bytes = sample(&Options {
            twice: true,
            ..Options::default()
        });
        let e = constructor_args(
            &parse(&bytes),
            "",
            "Inventory",
            "Awake",
            "ItemsContainer",
            "_container",
        )
        .unwrap_err();
        assert!(e.message.contains("found 2 times"), "{e}");
    }

    #[test]
    fn reads_static_dictionaries() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        assert_eq!(
            static_string_int_dictionary(&asm, "", "Equipment", "slotMapping").unwrap(),
            [("Head".to_string(), 2), ("Tank".to_string(), 6)]
        );
        // Set from something other than a dictionary initialiser.
        assert!(static_string_int_dictionary(&asm, "", "Equipment", "other").is_err());
        // The same initialiser read as any collection.
        assert_eq!(
            static_collection(&asm, "", "Equipment", "slotMapping").unwrap(),
            [
                vec![Arg::Str("Head".into()), Arg::Int(2)],
                vec![Arg::Str("Tank".into()), Arg::Int(6)]
            ]
        );
        assert!(static_collection(&asm, "", "Equipment", "other").is_err());
        let bytes = sample(&Options {
            bad_value: true,
            ..Options::default()
        });
        let e = static_string_int_dictionary(&parse(&bytes), "", "Equipment", "slotMapping")
            .unwrap_err();
        assert!(e.message.contains("not recognised"), "{e}");
        let e = static_collection(&parse(&bytes), "", "Equipment", "slotMapping").unwrap_err();
        assert!(e.message.contains("not recognised"), "{e}");
    }

    #[test]
    fn reads_string_arrays() {
        let bytes = sample(&Options::default());
        let asm = parse(&bytes);
        assert_eq!(
            string_array(&asm, "", "Inventory", "UnlockDefaultEquipmentSlots").unwrap(),
            ["Head", "Tank"]
        );
        assert!(string_array(&asm, "", "Inventory", "Awake").is_err());
        let bytes = sample(&Options {
            bad_index: true,
            ..Options::default()
        });
        let e = string_array(
            &parse(&bytes),
            "",
            "Inventory",
            "UnlockDefaultEquipmentSlots",
        )
        .unwrap_err();
        assert!(e.message.contains("not recognised"), "{e}");
    }

    #[test]
    fn damaged_assemblies_never_panic() {
        let bytes = sample(&Options::default());
        let read = |b: &[u8]| {
            if let Ok(asm) = Assembly::parse(b) {
                let _ = constructor_args(
                    &asm,
                    "",
                    "Inventory",
                    "Awake",
                    "ItemsContainer",
                    "_container",
                );
                let _ = static_string_int_dictionary(&asm, "", "Equipment", "slotMapping");
                let _ = static_collection(&asm, "", "Equipment", "slotMapping");
                let _ = string_array(&asm, "", "Inventory", "UnlockDefaultEquipmentSlots");
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
