# .NET assemblies: what we read from `Assembly-CSharp.dll`

Subnautica keeps a few pieces of gameplay data only in its C# code, not in
any data file: the names of the `TechType` enum, the crafting menus
(`CraftTree`) and `TechData`'s default values. P1 (`docs/DESIGN.md` § 4.3)
reads them from the player's own `Subnautica_Data/Managed/Assembly-CSharp.dll`
at runtime with our own reader, `crates/sn-dotnet`. Nothing read from the
DLL is stored in the repository.

The container format is public (ECMA-335, 6th edition, Partition II § 22–25
for metadata, Partition III for IL); this page records what we implement
and what the game's assembly looks like. Facts are marked *confirmed* (with
how) or *hypothesis*.

Readers: `sn-dotnet` (`Assembly`, `decode`, `enum_values`,
`tech_type_names`, `craft_trees`, `tech_data_defaults`),
`sn-assets::code` (`read_assembly`, `game_code`). Tools: `sn-inspect code`
(totals and checks), `sn-inspect code --trees` (every menu node),
`sn-inspect code --il <Type> <Method>` (one method's instructions, printed
to the console only).

## Container

- PE file: `MZ`, `e_lfanew` at 0x3C, `PE\0\0`, COFF header, optional
  header (PE32 or PE32+; the data directories start at 96 or 112), CLI
  header in data directory 14. RVAs map to file offsets through the section
  table.
- CLI header → metadata root (`BSJB`, version string, stream headers).
  Streams used: `#~` (tables), `#Strings`, `#US` (string literals,
  UTF-16 with a trailing flag byte), `#Blob`. Uncompressed metadata (`#-`)
  and the `…Ptr` indirection tables are rejected with an error.
- `#~`: version 2, heap-size flags (wide `#Strings` / `#GUID` / `#Blob`
  indexes), a 64-bit mask of present tables, row counts, then the rows.
  Column widths follow § II.24.2.6: table indexes and coded indexes are 4
  bytes once the (largest) target table outgrows 16 bits minus the tag
  bits. The schema of all 45 tables is in `sn-dotnet/src/tables.rs`.
- **Confirmed (real data):** the game's assembly (3,777,024 bytes, runtime
  `v4.0.30319`) parses: 3,111 TypeDefs, 20,826 Fields, 22,340 MethodDefs,
  8,210 MemberRefs, 4,657 Constants, 1,042 TypeRefs.

## Method bodies and IL

- Tiny header (1 byte, size in the top 6 bits) or fat header (12+ bytes:
  flags/size, max stack, code size, locals token). We read only the code;
  exception sections after it are not needed.
- Every opcode's operand size is in `sn-dotnet/src/il.rs` (one-byte
  opcodes and the `0xFE` two-byte page; `switch` has a count and that many
  targets).
- **Confirmed (real data, `sn-inspect code`):** all 21,383 method bodies of
  the game's assembly split into instructions to their last byte (570,477
  instructions, 0 errors).

## Tokens

- `ldstr` → `#US`; `call` / `newobj` → MethodDef or MemberRef; `ldsfld` /
  `stsfld` → Field or MemberRef; `newarr` → TypeDef, TypeRef or TypeSpec.
- A MethodDef's or Field's declaring type is the last TypeDef whose method
  or field list starts at or before it (lists are non-decreasing; binary
  search).
- A MemberRef's parent can be a TypeSpec. **Confirmed (real data):** the
  game's code has such references (e.g. a constructor of a generic
  `List<…>`). We name a generic instance by its generic type
  (`List`1`); other specs get a placeholder name that equals no real type.
- Parameter count: from the method signature blob (calling convention,
  optional generic count, parameter count).

## `TechType` names

- **Confirmed (metadata):** `TechType` is a top-level enum: a `value__`
  field, then one `static literal` field per member with an `int32`
  constant in the Constant table. 793 members, 793 distinct values, in
  declaration order (the decompiled source also has 793; an earlier note
  of 787 in `gameplay.md` was a miscount).
- **Confirmed (real-data test `game_code_names_menus_and_defaults`):**
  every TechData entry and every ingredient has a name.

## Crafting menus (`CraftTree`)

- **Confirmed (IL):** `CraftTree.Initialize` builds each menu with the
  sequence `ldstr <id>; call <Scheme>(); newobj CraftTree::.ctor(string,
  CraftNode); stsfld <field>`. We require every `new CraftTree` in the
  method to be part of such a sequence. 7 menus: `Fabricator`,
  `Constructor`, `Workbench`, `SeamothUpgrades`, `MapRoom`, `Centrifuge`,
  `CyclopsFabricator`.
- **Confirmed (decompiled source):** a `RocketScheme` method exists but
  `Initialize` does not call it, and `GetTree` returns no menu for the
  `Rocket` type. How the rocket's stages are built in the game is not
  checked yet.
- **Confirmed (IL):** each scheme method is straight-line code (no
  branches, no locals): string and integer constants, `newobj
  CraftNode::.ctor` with 1–3 arguments (id, `TreeAction`, `TechType`),
  `params` arrays (`newarr CraftNode; dup; <index>; <node>; stelem.ref`),
  `AddNode(params CraftNode[])` (returns its node, so calls chain) and
  `ret`. Our reader runs these on a small value stack and fails on any
  other instruction, so a changed game is reported, not misread.
- `TreeAction` (`None` 0, `Expand` 1, `Craft` 2) is read from its enum.
- **Confirmed (real data):** 159 nodes: 7 roots (`None`), 18 groups
  (`Expand`), 134 craft nodes. Per menu: Fabricator 100, Constructor 7,
  Workbench 13, SeamothUpgrades 22, MapRoom 5, Centrifuge 3,
  CyclopsFabricator 9. Every craft node's tech type has a TechData entry.

## `TechData` defaults

- **Confirmed (IL):** `TechData`'s static constructor sets
  `static readonly default…` fields, each from one constant (`ldc.i4`,
  `ldc.r4`, `ldstr`, `ldsfld String.Empty`) or, for `defaultItemSize`,
  `newobj Vector2int(int, int)` with two constants, then `stsfld`. It then
  builds the `defaults` JSON object from them and
  `defaultProperties = new List<string>(defaults.Keys)`; that last one is
  derived, and the reader skips it. Any other pattern is an error.
- **Confirmed (real data):** 17 defaults are read. Values our tool prints:
  item size 1×1, craft amount 1, craft time 0, max charge −1, not
  buildable, empty powered prefab; the rest 0 or a sound event path.
- **Hypothesis:** an entry without `craftTime` crafts with a duration
  chosen by the crafting code, not 0 s; check the crafter's code before
  M9e uses craft times.
