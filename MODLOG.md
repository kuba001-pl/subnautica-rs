# MODLOG

One entry per change: what, why, how it was verified. Record dead ends too.

## 2026-10-27 — M9s6: knife harvest, cabinets, supply crates and stored loot, headless

**What/why:** sixth step of M9s. `sn_sim::survival::world`: `ToolUseGate`
(one move-only `ToolAction` per accepted sequence; knife events behind the
strict 0.5 s re-entry guard), `knife_strike` (target validity, one call to a
caller-supplied damage resolver, before/after liveness, DamageAlive/
DamageDead output and final-cut bonus), `grant_harvest` (failed additions
returned, never spawned loose), `outcrop_click`/`bash_outcrop` (a held bash
tool waits for its event: one input, one hit), `Cabinet` (strict deadline,
door/take/full-grid paths, 2 s delayed toggle on a second clock, f32
deadlines like the game's float field), `SupplyCrate` (sealed/closed/open/
full/empty, absent Animation) and `draw_stored_loot`/`StoredLoot`
(rejection sampling, distinct indices, once flag). `sn_assets`:
`world_survival_rules` (Assembly-CSharp IL), `verify_loot_sampler`
(firstpass `UWE.Utils.Sample` recogniser), `knife_info`, `cabinet_info`,
`harvest_info`, `stored_loot_info` (stored fields/TechData).

**Review of the previous agent's uncommitted draft:** kept the sn-sim module
and readers. Fixed a compile error (closure type inference) in
`world_survival_rules`; replaced the cabinet door delay's bare float match
with `sn_dotnet::invokes`, requiring exactly one
`Invoke("ToggleDoorState", …)`; made cabinet deadlines single precision
(source: `timeSpawnMedKit` is a float) with a 2^24 s rounding test; added
`stored_loot_info`, synthetic reader fixtures (shape changes, wrong branch,
truncations: errors, 0 panics) and the opt-in install test with a crate census.

**Decisions (ours, not the game's):** SplitMix64 seeded loot RNG; 4,096
rejection attempts then an explicit error; negative/inverted loot bounds
refused; monotone action sequences for replay protection.

**Findings:** crate contents are a single active `PrefabPlaceholder` child in
all 8 SupplyCrate prefabs (setActiveOnOpen null in all), so SupplyCrate
never uses SpawnStoredLoot. Dead end recorded: a bounded scan of all 3,336
catalog-loadable prefabs found 0 `SpawnStoredLoot` users; its installed
caller (scene objects or none) is left to M9s7. Details in
`docs/formats/gameplay.md` § M9s6.

**Verification:** `cargo test -p sn-sim survival::world` (6 passed);
`cargo test -p sn-assets --lib reader_tests` (4 passed, 2 new);
`SUBNAUTICA_DIR=D:\SteamLibrary\steamapps\common\Subnautica cargo test -p
sn-assets --test real_data the_world_survival_rules_are_read -- --ignored
--nocapture` → guard 0.5, door delay 2, room (1,1), FirstAidKit 4514 ×1,
DamageAlive 3/DamageDead 4, base count 1; Knife damage 20, reach 1.2,
type 0; cabinet interval 600, startWithMedKit false; harvest kinds
{0:384,1:8,2:17,3:46,4:3,5:5}; Creepvine → CreepvinePiece bonus 0; 8 crates
(log `out/m9s6-real.log`). `cargo test --workspace` (796 passed, 0
failed, 70 ignored; log `out/m9s6-workspace.log`), `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all -- --check` clean. This
full run also closes M9s5's pending workspace check (STATUS.md, DESIGN row).
**Not tested:** server authority/reach/occlusion (M9s9), save of cabinet/
crate/loot flags (M9s10), live hand targets/UI (M9s11/13), placeholder child
resolution and any SpawnStoredLoot caller (M9s7).

## 2026-10-27 — M9s5: item-identity quick slots and held-tool state

Add pure binding/automatic eligibility/refill, desired vs held identities,
Sequence draw/hold/holster/drop, per-player first-use flags and PDA/death
teardown. Six synthetic tests cover row checklist, including same-clock
updates and reverse transitions. No inventory clones, dependency or whitelist
change. Runtime rules read from bounded IL and enum identities; stored tool
fields use metadata layouts.

Dead end corrected: the initial slot count pattern assumed a field; source
shows a property, so require the setter call after newobj instead. Runtime
reader now reports five slots, cap 10, default timings 0.5/0.35/1/0.7;
stored Knife timings 0/0.1/0/0.7 (first-use enabled). Draft fixture used an
unavailable IL emitter method; replaced it with the existing newobj helper.

Host-resource dead end: default workspace build failed out-of-memory/pagefile
1455; CARGO_BUILD_JOBS=1 retry failed writing client output (no space). Stop
after these two attempts; STATUS.md and DESIGN explicitly leave completion
pending. Targeted tests/install checks pass; no claim of a clean workspace
build at this step. Required post-merge verification remains.

Verification: `cargo test --workspace`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo fmt --all -- --check`; with SUBNAUTICA_DIR,
`cargo test -p sn-assets the_quick_slot_rules_are_read -- --ignored --nocapture`.
Clippy/format/install pass (logs out/m9s5-serial-1/2/3.log); workspace tests
remain resource-blocked above. Synthetic slot-count fixture passes separately
(out/m9s5-reader-test.log). Visual sockets,
IK/colliders and live input are **not tested**; M9s11 consumes the events.

## 2026-10-27 — M9s4: oxygen sources and ordered equipment effects

Borrow authoritative oxygen amounts; tank unregister/re-register preserves them,
spares contribute nothing, totals and fill/drain follow source ordering. Add
ordered speed calculation and gated charging with actual velocity/accepted
energy. Read coefficients from bounded DLL patterns, and stored fields through
existing metadata layouts (no new dependency). Four synthetic rule tests plus
runtime-reader malformed/ambiguity test. No whitelist change.

Environment: first workspace test link failed with LNK1180 (insufficient disk
space). Cache deletion was rejected automatically; no files removed. Free
space subsequently recovered; retry logged separately in
out/m9s4-workspace-retry.log (passed). Clippy/format/install checks passed first run.

Dead end: main-scene UpdateSwimCharge count 0; charging-fin prefab count 0.
Stop placement search after these two attempts; record STATUS.md. Correct M9s7
measurement wording, require an explicit serialized rate and leave active
placement/rate open. No balance fallback. DLL threshold is read successfully.

Verification: `cargo test --workspace`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo fmt --all -- --check`; with SUBNAUTICA_DIR,
`cargo test -p sn-assets the_equipment_rules_are_read -- --ignored --nocapture`.
Logs out/m9s4-check-*.log. Expected: all pass; four capacities 30/90/90/180,
surface fill 30/s, threshold 2; repeated reads identical. Live movement/vitals
integration and original-game acquisition comparison: **not tested**.

## 2026-10-27 — M9s2–3 fixes: audit and unique battery selector entries

Read the full repository rules and compared consumption, Eatable and battery
methods directly with ILSpy source (read only). All initial workspace checks
passed. Duplicate compatibility entries previously repeated grid batteries in
the reload selector; match the source's once-per-group behavior and test it.
Clarify research versus implemented claims and caller-supplied prefab inputs.
No new dependency or whitelist pattern. Old diffs have encoding damage; the
current M9s sections are UTF-8. No game content copied.

Verification: `cargo test --workspace`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo fmt --all -- --check`; all pass. With SUBNAUTICA_DIR,
`cargo test -p sn-assets the_consume_rules_are_read -- --ignored --nocapture`
and `cargo test -p sn-assets the_decay_and_energy_rules_are_read -- --ignored
--nocapture` pass (logs out/audit-0.log, out/audit-1.log): food gate 99,
heal threshold f32 0.1, oxygen 15, decay floor -25, fractions 0.5/f32 0.9,
despawn 5/300 s, default battery 504, charge/capacity/recovery 1/100/2.
Full prefab input loading and live application are **not tested** (M9s7+).

## 2026-10-27 — M9s3: decay, battery charge and replacement in sn-sim, headless

**What/why:** third step of M9s. Added `sn_sim::survival::decay`
(`DecayRules`, `Freshness`, `DespawnState`, `is_rotten`, `freshness`,
`set_decomposes`, `iterate_despawn`), `sn_sim::survival::energy`
(`EnergyRules`, `EnergyMixinInfo`, `ReloadOutcome`, `ReloadRefused`,
`make_battery`, `restore_battery`, `set_battery_charge`,
`spawn_default_battery`, `energy_charge`, `energy_capacity`,
`energy_scalar`, `modify_charge`, `add_energy`, `consume_energy`,
`sort_reload_candidates`, `select_battery`), and
`sn_sim::inventory::{force_pickup, flush_pending}` (`Inventory.pending`) so
replacing or unloading a battery at a full grid retains the outgoing
battery in `inv.pending` without losing or duplicating it. Added
`sn_assets::{decay_rules, energy_rules}` reading `Eatable`, `Battery` and
`EnergyMixin` constants from `Assembly-CSharp.dll`. Settled the stored-item
active-state hypothesis in `docs/formats/gameplay.md` from `Pickupable` /
`StorageSlot` / `Inventory`.

**Verification:** `cargo test -p sn-sim survival` (22 passed: 7 hunger + 8
consume + 3 decay + 4 energy covering exact 0.5 and 0.9 freshness
boundaries, -25 floor, inactive rotten retention vs active >300 s deadline,
disabling/re-enabling decomposition, default battery only when absent,
restored used/empty batteries never refilling, charge conservation across
empty/full/partial changes, candidate sorting, full-grid swap/unload into
`inv.pending` and `flush_pending` with 0 duplicate ids, and explicit error
on missing rule inputs). `cargo test -p sn-assets --lib survival` (5
passed). `SUBNAUTICA_DIR=D:\SteamLibrary\steamapps\common\Subnautica cargo
test -p sn-assets --test real_data the_decay_and_energy_rules_are_read --
--ignored --nocapture` → `decay_floor: -25.0, ripe_fraction: 0.5,
decomposing_fraction: 0.8999999761581421, despawn_check_interval: 5.0,
default_despawn_delay: 300.0`; `default_battery_tech: 504,
default_battery_charge: 1.0, default_battery_capacity: 100.0,
recovery_time: 2.0` (log `out/m9s3-real.log`). `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` clean,
`cargo fmt --all`.

## 2026-10-27 — M9s2: per-item state, eating, drinking and first aid, headless

**What/why:** second step of M9s. Extended `sn_sim::inventory::Item` with
`ItemState` (`decay_start`, `oxygen`, `energy: Option<EnergySlot>` holding
`Option<Box<Item>>`) and `Item::ids` so per-object survival state and
nested batteries travel with the item across grid, equipment slots,
storage, drops and death (`lose_items`). Added `sn_sim::survival::consume`
(`ConsumeRules`, `EatableInfo`, `Eaten`, `Used`, `UsedItem`, `Refused`,
`eat`, `eat_item`, `use_tech`, `use_item`, `add_health`, `initial_state`),
`sn_dotnet::int_literals`, and `sn_assets::consume_rules` reading the
constants and tech types of `Survival.Eat`, `Survival.Use` and
`Eatable.GetFoodValue`/`GetWaterValue` from `Assembly-CSharp.dll`.
Also confirmed in `the_consume_rules_are_read` that
`Survival.GetWeaknessSpeedScalar` has 0 callers in `Assembly-CSharp.dll`.

**Review of the previous agent's uncommitted draft:** kept `ItemState`,
`EnergySlot`, `int_literals`, `consume_rules` and the core `consume.rs`
logic; replaced `use_item`'s `(Option<Used>, Option<Item>)` return with
` Option<UsedItem>` (`Healed(Item, f64)` vs `Full(f64)`) and factored test
types in `sn-assets::survival` to fix two `-D clippy::type-complexity`
errors; extended the 1,000-step transfer test to also exercise `lose_items`
(death); added invalid-input refusal tests and the opt-in real-data test
`the_consume_rules_are_read`.

**Verification:** `cargo test -p sn-sim survival` (15 passed: 7 hunger + 8
consume covering food 99 vs 99.5 gate, 200/100 caps, clamped full-stat
consumption, negative/zero values, `vitals_ok`, Bladderfish +15 O2,
first-aid >0.1 heal threshold vs full retention, decomposing floor -25,
invalid input refusal, and 1,000 grid/slot/locker/drop/death transfers
preserving distinct per-item ages, oxygen and nested battery ids/charges
with 0 duplicates). `cargo test -p sn-assets --lib survival` (4 passed).
`SUBNAUTICA_DIR=D:\SteamLibrary\steamapps\common\Subnautica cargo test -p
sn-assets --test real_data the_consume_rules_are_read -- --ignored
--nocapture` → `food_gate: 99.0, vitals_ok: 20.0, first_aid: 4514,
first_aid_health: 50.0, heal_success: 0.10000000149011612, oxygen_food:
2515, oxygen_on_eat: 15.0, decay_floor: -25.0`, `GetWeaknessSpeedScalar`
callers: 0 (log `out/m9s2-real.log`). `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` clean,
`cargo fmt --all`.

## 2026-10-27 — M9s1: hunger rules, headless, numbers from the game's code

**What/why:** first step of M9s. `sn-sim::survival` (new module dir,
`hunger.rs`) implements `Survival.UpdateHunger`/`UpdateStats`/`ResetStats`/
`OnHealTempDamage`: food/water drain per scheduled call, shortfall-based
Starve damage, the after-drain regeneration gate, warning crossings (one
per stat, empty > critical > low), freeze/no-survival suppressing effects
but not the schedule, respawn resetting stats not phase. Each call returns
ordered effects (warnings, damage, heal) instead of the previous draft's
summed totals, so a caller's health sees the game's order. No balance
defaults in code: `sn_assets::hunger_rules` reads all 13 numbers from the
player's DLL (constructor initialiser, the ordered float constants of
`UpdateStats`/`UpdateHunger`/`OnHealTempDamage`, `ResetStats`), checks
repeated uses agree and that each equals its `SurvivalConstants`
declaration, and errors with the IL offset otherwise. New `sn-dotnet::
float_literals` (every `ldc.r4` with offset) supports this. Live play is
unchanged (drain is not wired; M9s8/M9s11).

**Review of the previous agent's uncommitted draft:** kept its API shape,
validation and the Start/advance phase semantics; replaced summed
damage/healing with ordered per-call effects, added positive-damage gate,
the 1e-9 s due tolerance (with `+1e-12` on the ratio, many 0.02 s steps
could reach a call one step late), the `start` constructor (reset then the
immediate call) and the rule loader; fixed text lost from its gameplay.md
note (two words had lost their first letter). Our call: f64 state, though
the game computes in f32 (noted as a residual in gameplay.md).

**Verification:** `cargo test -p sn-sim survival` (7 passed: 60 s from
100/100 loses 2.380952/3.333333; both empty for 10 s = 23.809524 damage;
exact/over thresholds; heal gate after drain incl. overfill; freeze, mode,
respawn; 3001×0.02 s and uneven steps give 6 calls and the same phase;
NaN/negative/huge/invalid rules refuse without mutation).
`cargo test -p sn-assets --lib survival` (3 passed: synthetic assembly
read; changed repeated/fixed/extra constants and declaration mismatch are
errors with offsets; truncated/flipped bytes never panic).
`SUBNAUTICA_DIR=D:\SteamLibrary\steamapps\common\Subnautica cargo test -p
sn-assets --test real_data the_hunger_rules_are_read -- --ignored
--nocapture` → interval 10, 2520/1800, 100/200, 25, 150, 0.041666668,
50.5/90.5, 20/10, 0.25; 60 s drain 2.380952/3.333333; two reads equal
(log `out/m9s1-real.log`). `cargo test --workspace` (758 passed, 0 failed,
65 ignored), `cargo clippy --workspace --all-targets -- -D warnings` clean,
`cargo fmt --all`. Not tested: Unity frame timing of the first call
versus our fixed tick.


## 2026-10-27 — Plan M9s: survival (research and docs only)

**What/why:** promote After Phase E's survival item into Phase E as M9s,
after M9f, with fifteen individually verifiable steps. Plan pure sn-sim
rules before runtime loading, server owner/world actions, save extensions,
client use/tool presentation and live UI. M9e crafting and M9f persistence
are in progress elsewhere; this change documents their integration contracts
without editing code or claiming those milestones complete. Live hunger is
gated on a working acquisition/consumption loop. No dependency, crate,
architecture or gitignore change.

**Research:** read AGENTS.md, DESIGN's Phase E/M9d/M9u/M9e plans and gaps,
After Phase E, and gameplay.md. Directly read the full local ILSpy source
(read only, not fields.py) for Survival/SurvivalConstants, Eatable,
Inventory/InventoryItem, QuickSlots, PlayerTool, GUIHand/ArmsController,
Knife/HarvestType/TechData, MedicalCabinet, Oxygen/OxygenManager,
Fins/UnderwaterMotor/UpdateSwimCharge, Battery/EnergyMixin, SupplyCrate,
LootSpawner/SpawnStoredLoot, food/water bars and TooltipFactory. Added our
own prose findings in gameplay.md, each marked confirmed with source or
hypothesis. Numbers' runtime origins are explicit (serialized fields,
TechData/defaults, DLL constants/IL); no source code, content dumps or game
files copied into the repository.

**Findings/dead ends:** FirstAidKit use lives in Survival, not a separate
class; Fins does not implement speed modifiers. A full-source caller search
found no caller for the weakness scalar, so adding a hunger speed penalty
would be speculative. Random stored loot and child-item supply crates are
different paths; prefab wiring remains to measure. Initial searches under
ignored out/ needed --no-ignore, and PowerShell does not expand shell brace
lists. The terminal's cp1252 output also needed Python -X utf8. A first
patch-context attempt matched nothing and changed nothing. A documentation
audit caught non-ASCII characters lost in new pipeline input; corrected with
Unicode escapes and checked that pre-existing Unicode remains intact.

**Verification:** git diff --check (no whitespace errors); a Python document
audit checks M9s row placement after M9f, plan placement after M9e, all fifteen
step ids, required Done when headings, and preserved existing Unicode.
Final git status --short must list only these three Markdown files before
the local commit, and be clean after the requested merge of local main.
Exact repeatable commands: git diff --check; git show --stat --oneline HEAD;
git status --short (expect no entries after commit/merge). Runtime gameplay,
prefab census, proposed CLI scenarios and cargo test --workspace are
**not tested**: this is research + plan only, no Rust code changed. The
proposed CLI commands are labelled not implemented in DESIGN.

## 2026-10-27 — M9e1: fabricator rules, headless

**What/why:** direct source research (local ILSpy decompile, read only)
settled the plan's hypotheses. `sn-sim::craft` validates known tech,
grid ingredients and power, consumes with the existing item selection,
and retains partial primary/linked outputs when pickup runs out of room.
Five synthetic tests cover missing/unknown/power/full/amount/linked,
timing, malformed recipes and atomic refusal. `sn-assets::craft_recipe`
maps runtime TechData; `sn-inspect craft` and ignored `fabricator_recipes`
check every craft node. Research in gameplay.md and ui.md; DESIGN and
dotnet.md corrected. No dependencies or ignore changes.

**Found/dead end:** the first install run failed its planned count of
100 craft nodes. Existing `sn-inspect code --trees`, its P1 test and the
decompiled tree all agree: 100 total nodes, 87 craft, 12 categories,
1 root. Corrected the check and the plan, then reran successfully. The
plan's up-front room check also differed from the source: the game checks
each result during pickup, and attempts pickup automatically in range.
Default animation 2.7 s, logic completion 2.8 s; energy 5. There is no
busy hand refusal message (progress only). Initial compile caught a
missing re-export/private module use; corrected before verification.

**Verified** (PowerShell; logs `out/m9e1-*.log`):
- `cargo fmt --all`, `cargo fmt --all -- --check`: clean.
- `cargo test --workspace` (SUBNAUTICA_DIR unset): 748 passed, 0 failed,
  64 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `$env:SUBNAUTICA_DIR='D:\SteamLibrary\steamapps\common\Subnautica'`;
  `cargo run --release -p sn-inspect -- craft`: 100 total nodes, 87 craft,
  56 absent craftTime, 0 unresolved, 46 starting blueprints (46 unique),
  every recipe's ingredients/amount/linked/time/energy logged; RUN OK.
- With the same environment: `cargo test -p sn-assets fabricator_recipes
  -- --ignored --nocapture`: 1 passed.
- `git diff --check`: pass; status contains only our Rust and docs.

**Not tested:** original-game screen comparison, client crafting,
network crafting, serialized pod power/repair interactions, Linux/macOS.
These remain M9e2–5 work. No game assets committed. Refused requests
validate before spending energy; the original's direct GhostCrafter call
spends energy before resource validation (the UI normally prevents it).

## 2026-10-27 — Plan: M9e (crafting at the lifepod's fabricator)

**What/why:** with M9u done, DESIGN's road (§ 4.3, Phase E) has M9e next.
Wrote "M9e plan" in DESIGN (below the M9u plan): why now, what the game
has (class and field names checked on the install with `sn-inspect code
--fields Fabricator`, `GhostCrafter`, `Crafter`, `CrafterLogic`,
`CrafterGhostModel`, `uGUI_CraftingMenu`; their behaviour marked
*hypothesis* until M9e1 reads the decompiled code), steps M9e1–M9e5
each with a "Done when", out of scope and risks. The Phase E row of M9e
points to it. Plan only: nothing implemented.

**Verified:** docs only; `cargo test --workspace` 743 passed; `git
status --short` clean after the commit.

## 2026-10-27 — M9u9: close-out (two players with the game's UI; M9u done)

**What/why:** the last step of M9u (DESIGN). No code change: the merged
client (Perf P1a–P1e into M9u8) was smoke-checked, three two-player runs
check what each client's UI shows, and the docs were brought up to date:
README ("What works" UI paragraph, "What doesn't work yet", "Play
together"), `docs/formats/ui.md` (a status paragraph, stale "not yet"
notes from before M9u8b fixed, § Two players), DESIGN (the "Not 1:1
after M9u" list collected from the M9u8b3–b5 entries, M9u and M9u9 ✅).
No new dependencies, no `.gitignore` change.

**Verified** (Git Bash; release build; `SUBNAUTICA_DIR` set for the game
runs; logs in `out/m9u9/`):
- Merge smoke check (same flags as the MODLOG entries): tooltip run as
  M9u8b3b3: three tooltips and one clear, first at pointer (−463, 246),
  top-left (−452.650, 225.299), size 469 × 186.15, sorting (1, 0)+2;
  drag/swap run as M9u8b4: the same three moves applied (world events
  10–12) and "nothing to do"; slot run as M9u8b4: the same five moves
  (events 11–15), Tank denied over Foots, the click while dragging
  ignored; notification run as M9u8b5a: `Add` × 3, count 0 → 3 at
  5.035 s, progress 0.748 / 0.499 / 0.248, count 3 → 0 at 12.035 s; 0
  WARN/ERROR in all four. P1 swim route (docs/P1-PERFORMANCE.md flags,
  `SN_CLIENT_PERF=1`, two runs): sampled max 23.134 / 26.809 ms, p99
  15.963 / 16.329 ms, 3,624 / 3,623 frames; the same end scene as P1e
  (102 batches [4, 4, 20, 74], 2,950,338 triangles, 13,171 object
  entities, 201 slot objects, 0 warnings). No regression from the merge.
- Two players, host `--host PORT --name host`, joiner started 8 s later
  with `--join 127.0.0.1:PORT --name joiner`, both `--view 400
  --resolution 960 540`; 0 WARN/ERROR in all six logs; server hash
  checks 74, 58 and 70, 0 differ:
  - (A) `a-host.log` / `a-joiner.log`: host `--test-supply Tank,Fins
    --use-item 3 --panel 6=take-all,7=hover0,8=drag0,9=over-locker,
    10=release,12=hover0,12.2=press,12.4=release,25=hover0,40=close
    --shot 45`; joiner `--test-supply Tank,Fins --use-item 3 --panel
    8=drag-locker0,9=over-grid,10=release,11=hover0,13=hover-empty
    --shot 20`. Both log the same locker after world events 11 (empty),
    13 (Tank), 14 (Tank, Fins) and 15 (Fins: the joiner took the Tank by
    drag); the host's inventory line does not change at event 14; the
    joiner's inventory is only the Tank. Tooltips on both (host Tank,
    Fins, Flare; joiner Tank).
  - (B) `b-*.log`: both `--start -197.3 -35.2 -10.2`; host `--pick-up 3
    --panel 5=open,11=close --drop 14 --shot 32`, joiner `--pick-up 10
    --panel 12=open --shot 18`. Host: 3 pick-ups, 3 icons, `Add` × 3,
    count 0 → 3 → 0; drops #6 (event 7). Joiner: object 6 drawn (asked at
    event 8), picks it up with 2 more placed mushrooms (events 8–10), 3
    icons, `Add` × 3, count 0 → 3 → 0. On both: object 6 removed at event
    11, 5 keys gone. Neither client shows the other's icons or counts.
  - (C) `c-*.log`: host `--start -197.3 -35.2 -10.2 --pick-up 3 --kill
    14 --shot 36`, joiner at the same place `--shot 26`. Host: `LoseItems`
    2 dropped, "You died and lost some belongings." shown 23.021 s, gone
    after 6.006 s; joiner: 0 messages, objects 6 and 7 drawn at event 11
    as on the host.
- `cargo test --workspace` (no game): 743 passed, 0 failed, 63 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass;
  `cargo fmt --all -- --check`: clean.

**Not tested:** two machines (all runs on 127.0.0.1); a real mouse or
keyboard; "Inventory full" in the client; the locker's view while the
other player changes it with both PDAs open on it at the same moment (in
(A) the host's PDA was open on the locker during the joiner's drag and
logged the change, but the joiner's PDA was not open during the host's
moves); against the original game on screen; Linux/macOS. **Not 1:1:**
the DESIGN list "Not 1:1 after M9u". No game assets committed.

## 2026-10-27 — Format: `cargo fmt --all` (commit ad1e740)

**What/why:** `cargo fmt --all -- --check` reported drift in four
tooltip files from M9u8b3 (`apps/sn-inspect/src/main.rs`,
`apps/sn-inspect/src/tooltip.rs`, `crates/sn-ui/src/tooltip.rs`,
`crates/sn-ui/src/tooltip/tests.rs`). Formatting only.

**Verified:** `cargo test --workspace` 743 passed, 0 failed; clippy
(`-D warnings`) pass; `cargo fmt --all -- --check` clean.

## 2026-10-27 — M9u8b5b: the notification's glow (`UI/Icon` `NOTIFICATION`)

**What/why:** the second half of M9u8b5 (DESIGN). `game_ui.wgsl`:
`UI/Icon`'s `NOTIFICATION` variant from its D3D11 programs (blobs 144 and
176; flag 128, with or without slice 9): rgb × (1 + 0.75 · strength ·
the overlay's red at the ring's u); the image pipeline gains group 2 (the
overlay, white when unused) as the bar pipeline has; `GpuItem` /
`UiDrawCmd` carry strength, `_PDATime`, `_Size` and the overlay's key.
`sn_assets::Assets::resource_texture` (`Sprites/NotificationOverlay`);
the client puts it on `iconMaterial` and sends it with the builds.
`sn_ui::notifications`: `glow_u`, `glow` (the reference the WGSL
mirrors) and `god_rays` (`SetGodRaysIntensity` on both materials, the
global `_PDATime`); `notify.rs` sets them from `Progress`. No new
dependencies, no `.gitignore` change.

**Verified** (Git Bash; logs in `out/m9u8b5/`):
- `cargo test --workspace` (no game): 740 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `target/release/sn-inspect ui --notifications`: 15 of 15, RUN OK
  (`Sprites/NotificationOverlay`: class 28 in `resources.assets`).
- `target/release/sn-client --view 400 --resolution 960 540 --start
  -197.3 -35.2 -10.2 --pick-up 3 --panel 6=open --shot 8.3 m9u8b5b-glow`
  (`glow.log`): 0 WARN/ERROR (no GPU validation errors); census while
  notified `UI/Icon NOTIFICATION` 3, `UI/Icon NOTIFICATION SLICE_9_GRID`
  3, drawn as `UI/Default` only the loading bar's `UI/Bar` (as before).
  The same at 1920 × 1080 (`m9u8b5b-glow-1080.png`): the three new icons
  brighter than the rest; 0 WARN/ERROR.

**Not tested:** the ring's look against the game (the screenshots show
the icons brightened; the ring's motion is not checked), the overlay's
wrap mode, the WGSL itself by a test (only its Rust reference),
Linux/macOS. **Not 1:1:** `_PDATime`'s phase (our own sum, not the
PDA's time since the game started). No game assets committed.

## 2026-10-27 — M9u8b5a: the inventory's notifications (count and "+")

**What/why:** the first half of M9u8b5 (DESIGN, split after reading the
code). `sn_ui::notifications`: `NotificationManager` (`Add`, `Remove`,
`GetCount`, `RegisterTarget`, `UnregisterTarget`, `LateUpdate`) with
targets and the `Progress` calls it makes, the label's state
(`SetNotificationAlpha` / `SetAlpha` / the made frame's white), the
anchors, the slot offset; 5 unit tests. `sn_assets`:
`notification_label` (`uGUI.prefabNotificationLabel` as a scene, its
`background` / `text` nodes and colours) and `UiScene::set_anchors` (a
script's `SetAnchor`; the only `ui_scene.rs` change). Client
`game_ui/notify.rs`: pick-ups (new ids in the grid or slots from none of
our stores) add `(Inventory, id, 4)`, leaving the grid or a slot removes,
every tab icon is a target (visible by its active chain and inherited
alpha), `Progress` shows the "+" label, the toolbar's inventory icon
shows the count. `sn-inspect ui --notifications`. No new dependencies,
no `.gitignore` change.

**Dead end:** the first version added on the frame the session reported
our `PickedUp` events (`plays`); the predicted stores show the item a few
frames earlier, so nothing was new by then. Now any item that comes into
the grid or a slot from outside our stores is a pick-up.

**Verified** (Git Bash; `SUBNAUTICA_DIR` set for the game runs; logs in
`out/m9u8b5/`):
- `cargo test --workspace` (no game): 739 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `target/release/sn-inspect ui --notifications`: 14 of 14, RUN OK (the
  prefab's 2 nodes; the count label's pivot at the toolbar icon's upper
  right + (−41.5, −3.5) and the "+" at the item icon's + (−10, −10), both
  to 1e-3; label 36 × 36.97; vertex alpha 255 and 180 for sin(π/4); the
  replay: closed 3 s count 1 and progress 1 each frame, open: count 0 after
  241 frames = 4.017 s).
- `target/release/sn-client --view 400 --resolution 960 540 --start
  -197.3 -35.2 -10.2 --pick-up 3 --panel 6=open --shot 12
  m9u8b5a-cleared`: 0 WARN/ERROR; 3 acid mushrooms → `Add` × 3, count
  0 → 3 at 4.954 s, "+" at 1 while the PDA was closed; opened at 7.956 s;
  progress 0.749 / 0.499 / 0.249 at 8.951 / 9.953 / 10.952 s; count 3 → 0
  at 11.951 s (3.995 s seen). With `--shot 8 m9u8b5a-count`: the "3" on
  the inventory tab and a "+" on each new icon (`out/m9u8b5a-count.png`).
- the same with `--panel 6=open,7=drop --shot 9 m9u8b5a-drop`: the drop
  removed #6's (count 3 → 2), 0 WARN/ERROR.

**Not tested:** a pick-up straight into a slot (its label offset), the
PDA closed within a second of seeing (`UnregisterTarget`'s start-over),
saved notifications (`Serialize` is not ported), Linux/macOS, against the
running game on screen. **Not 1:1:** the glow (`NOTIFICATION`) waits for
M9u8b5b; the other tabs' counts (Blueprints, Log, …) are not ported (0);
a pick-up is recognised by its new id, not by `Inventory.Pickup` itself
(so `noMessage` pick-ups would count; none happen in what we run). No
game assets committed.

## 2026-10-27 — M9u8b4: drag and drop on the inventory tab

**What/why:** finished the previous agent's unfinished drag (kept its
plan, its scripted steps and the `Stores` copy idea; rewrote the
controller). `sn_ui::drag`: the press/drag/release rule (`FPSInputModule`:
the click on the release, a release on the dragged item ignored),
`UpdateFeedbackState`'s slot tints, `StableLerp` and the snaps; 5 unit
tests. Client `game_ui/drag.rs`: the source locked at 0.3, the dragged
icon and the `ItemActions/Swap` hint grafted under the PDA's root and
drawn last, the snap to slots and swap targets, the swap target dimmed,
the drop's `Request::Move` found by `add_or_swap` on a copy of the stores
(no protocol or server change), no hover or tooltip while dragging and
no re-lit slots after a drop until the pointer enters something else;
2 unit tests. The left button now goes through the drag (its click on
the release); right and middle clicks still act on the press.
`--panel` steps `dragN`, `drag-lockerN`, `drag-slotS`, `over-grid`,
`over-locker`, `over-slotS`, `press`, `release`. `sn-inspect ui
--inventory` gained the drag checks. `sn_assets::GROUP_ITEM_ACTIONS`
added. Also fixed mojibake in the M9u8b3b3 DESIGN row / MODLOG title and
literal `
`s in README. No new dependencies, no `.gitignore` change.

**Dead end:** the first placement check read the layout made before the
drag's state was applied (the worker rebuilds its rules each frame), so
it measured the template's (0, 0); the drag now owns its state, applied
with the tab's before the layout, and the check reads 0.000.

**Verified** (Git Bash; `SUBNAUTICA_DIR` set for the game runs; logs in
`out/m9u8b4/`):
- `cargo test --workspace` (no game): 734 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass.
- `target/release/sn-inspect ui --inventory`: 74 of 74, RUN OK (Tank
  and Fins over each of the 7 slots: tints by the rule; Foots, Head and
  Tank snaps at their icons, radius 62 = the icons' `_Radius`; a locked
  icon's alpha 1 → 0.3; `ItemActions/Swap` 64 × 64 at 100 px/unit).
- `target/release/sn-client --view 400 --resolution 960 540 --use-item 3
  --test-supply Tank,Fins --panel 6=take-all,7=drag0,8=over-locker,
  9=release,10=drag1,11=over-locker,12=release,13=drag-locker0,14=hover0,
  15=release,16=drag0,17=over-grid,18=release,19=drag-locker0,19.5=hover1
  --shot 20 m9u8b4-drag-swap`: 0 WARN/ERROR; grid → locker
  `Move{6, PodLocker, Store}` and `Move{4, PodLocker, Store}`, locker
  onto a grid item `Move{6, Grid, Item(7)}` (snap, Swap hint), a drop on
  its own grid "nothing to do"; each applied by the server (world events
  10–12); dragged icon 0.000 off its place.
- the same with `--test-supply Tank,Tank,Fins --panel 6=take-all,7=close,
  8=open,10=drag2,10.5=over-slotTank,11=over-slotFoots,12=release,
  13=drag0,13.5=over-slotFoots,14=over-slotTank,15=release,16=drag0,
  17=over-slotTank,18=release,19=drag-slotTank,20=over-grid,21=release,
  22=drag-slotFoots,22.5=over-slotFoots,23=release,24=hover0,24.2=press,
  24.4=release --shot 26 m9u8b4-slots-end`: 0 WARN/ERROR; Fins over Tank
  denied, over Foots allowed and snapped, `Move{8, Slots, Slot(Foots)}`;
  Tank over the filled Foots denied, `Move{6, Slots, Slot(Tank)}`; the
  second Tank onto the filled Tank slot: Swap snap and hint,
  `Move{7, Slots, Slot(Tank)}`; slot → grid `Move{7, Grid, Store}`;
  release on its own slot ignored; press + release on a grid Tank clicks
  (equip, `Move{6, Slots, Store}`) — all applied by the server (events
  11–15). Shots `out/m9u8b4-drag-swap.png`, `out/m9u8b4-slot-swap.png`
  (green slot, hint), `out/m9u8b4-denied.png`.

**Not tested:** a real mouse drag (the threshold path; scripted drags
start past it), the drag threshold's value (10, Unity's default, a
*hypothesis*), comparison with the running game on screen, the hint's
size against the game, Linux/macOS. **Not 1:1:** right and middle clicks
still act on the press (the game's on the release; the game also drags
with them); the slot tints are worked out each frame (the game only on
events; same result in the cases met); a swap target that `CanSwap`
allowed but the move refused is set back to alpha 1 at the drop (the
game leaves it dimmed); `CanSwap` is approximated by `AddOrSwap` on a
copy; no bar on the dragged icon (bars not ported). No game assets
committed.

## 2026-10-27 — M9u8b3b3: the tooltip in the client

**What/why:** reviewed and completed the previous agent's unfinished input,
placement and canvas-sorting edits. The worker builds the hovered item's
tooltip with sn-ui's tested text/layout/placement rules; caches prefab facts
per class; receives current actions, class and battery charge from the sim;
inherits the hit graphic's nested alpha and PDA sorting +2. Scripted hover
projects icon centres through the same screen ray as mouse input. Clear
disables the tooltip canvas at alpha zero. No dependencies or ignore changes.

**Verified** (PowerShell; SUBNAUTICA_DIR set to the installed game; logs
out/tooltip-*.log):
- cargo test --workspace: 727 passed, 0 failed (game environment unset).
- cargo clippy --workspace --all-targets -- -D warnings: pass.
- cargo build --release -p sn-client -p sn-inspect: pass.
- target/release/sn-inspect.exe ui --tooltip: 65/65, RUN OK.
- target/release/sn-client.exe --view 400 --resolution 960 540 --use-item 3
  --panel 6=take-all,7=hover0,8=hover1,9=hover-empty,10=hover2
  --shot 12 m9u8b3b3-tooltip: 0 WARN/ERROR; three tooltips and one clear.
  First hover: pointer (-463,246), top-left (-452.649,225.298), size
  469?186.15 (485.466?192.685 on the canvas), scale 0.000219246,
  alpha 1, sorting (1,0)+2. Margin offsets 10.351 and -20.702 are
  exactly (10,-20) times scale/canvas-scale. Flare and water texts and
  switch-container hints logged from the runtime language file.
- target/release/sn-client.exe --view 400 --resolution 960 540 --use-item 3
  --panel 6=take-all,7=hover0,9=hover-empty --shot 11 m9u8b3b3-clear:
  0 WARN/ERROR; clear logs alpha 0, active false; screenshot saved.
  Both PNGs are in out/ (365848 and 360873 bytes).
- git diff --check: pass; status contains only our source/docs.

**Not tested:** real mouse input, comparison with the running game's screen,
Linux/macOS. **Not 1:1:** existing b3b1 omissions (food decay, oxygen/tool
charge and other unported tooltip components); canvas deactivated on Clear
instead of only zero alpha (same visible result). No game assets committed.

## 2026-10-27 — M9u8b3b2: the tooltip's layout and placement

**What/why:** the second step of M9u8b3b (DESIGN): the tooltip's size
and where it goes beside the pointer, headless.
- `sn-ui::tooltip`: `TooltipLayout` (`OnLayout` with no icons: the width
  from the texts' unbounded preferred widths, at most 429, the heights at
  that width, the texts' places, the root's size; `state` gives the tree's
  changes), `update_position` (`UpdatePosition`: the scale from the
  pointer's depth, beside the pointer by the margin, flipped or pinned at
  the canvas's edges), `PointerOnCanvas`, `TooltipPose`; 2 unit tests.
- `sn-inspect ui --tooltip`: the layout and placement checks.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8b3b/`):
- `sn-inspect ui --tooltip` (`ui-tooltip-b3b2.log`): RUN OK, 65 of 65.
  Four items' prefixes are wider than 429 unwrapped and wrap at 429
  (tooltip 469 wide); the others 277 to 364 wide. On the PDA canvas
  (1195 × 970, scale 0.00021181, 0.1874 m away) the tooltip's scale is
  0.0002174, so it is 1.026 times its size in the canvas's units. The
  pointer at the centre puts it right of and below the pointer; near the
  right edge to the left; near the bottom above; in the corner both.
- `cargo test --workspace` 727 passed, 0 failed; clippy clean.
- Not tested: against the game's own layout on screen. TMP's preferred
  sizes are M9u4c's port, checked there, not here.

## 2026-10-27 — M9u8b3b1: the tooltip's texts (plan for M9u8b3b)

**What/why:** M9u8b3b (the inventory item tooltip) was researched in the
decompiled `uGUI_Tooltip`, `TooltipFactory`, `FPSInputModule` and
`CursorManager` and split into b3b1 (texts), b3b2 (layout and placement)
and b3b3 (the client) in DESIGN. This first step makes the texts.
(Started by an agent that was cancelled; its uncommitted work was
reviewed against the decompile and kept, with the fixes below.)
- `sn-ui::tooltip` (new): `TooltipConfig`, `TooltipStrings` (the action
  words, the four buttons' texts, the formats), `cs_format` (the subset
  of C#'s `string.Format` the English entries use: `{0:+#;-#;+0}`,
  `{0:P0}`), `WriteTitle` / `WriteDescription` / `WriteAction`,
  `item_commons`, `item_actions`, `inventory_item`; 4 unit tests with
  made-up strings.
- `sn-assets`: `tooltip_config` (`uGUI_Tooltip`'s nodes and
  `scaleFactor`), `item_tooltip_values` (an item prefab's `Eatable` and
  `Battery`, and the scripts whose lines we leave out),
  `ItemTooltipValues::facts`; `UiScene::text_params`.
- `sn-inspect ui --tooltip` (new): the prefab dumped, then the checks.
- `docs/formats/ui.md` § The tooltip: the research.
No new dependencies.

**Found:** `Battery._charge` is not stored (`[NonSerialized]`);
`OnAfterDeserialize` sets it to `_capacity`, so a new battery shows 100 of
100. The first run failed on reading it; fixed to the rule.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8b3b/`):
- `sn-inspect ui --tooltip` (`ui-tooltip-b3b1.log`): RUN OK, 23 of 23.
  Flare: the bind hint and drop; FilteredWater: water +20, eat, drop;
  NutrientBlock: food +75, eat, drop; FirstAidKit: health +50, use,
  drop; Titanium: drop; Battery: the charge line and no description,
  drop; the Tank in its slot: unequip. All as written in DESIGN before the
  run, each line equal to one built from the language file.
- `cargo test --workspace` 725 passed, 0 failed; clippy clean.
- Not tested: `{0:P0}` against the game (we write the percent sign with no space; Mono's en-US
  spacing is a hypothesis); the Tank's oxygen line, tools' `EnergyMixin`
  charge and food decay are not ported.

## 2026-10-27 — P1e: staged terrain/grass LOD uploads and intra-slot object spawn budget

**What/why:** Fixed the remaining movement-boundary frame spikes in the 35 s
swimming/turning scenario (`--start -5 -10 -5 --look 200 -10 -5 --hold-forward 1
--turn 8 --shot 35`). Each LOD-0 terrain batch completion in `terrain::stream`
previously uploaded all `TerrainPart`s (`668k–848k` vertices, single base layer
up to `261,530` vertices) and `GrassPart`s (`521k–736k` vertices, single grass
layer up to `583,375` vertices) in a single frame (`render::mesh_alloc`
`103.1–143.7 ms`), while `objects::stream_objects` spawned all `2,118–4,914`
fine-detail (`level == 0`) entities of that batch in one frame (`8.4–20.9 ms`)
because `SPAWN_BUDGET` was checked only between `(batch, level)` slots.

- `apps/sn-client/src/terrain.rs`: split oversized `TerrainPart` and `GrassPart`
  meshes on the worker thread into chunks of at most `PART_TRIANGLE_BUDGET =
  12,000` triangles (`<= 36,000` vertices, preserving shared batch bounding
  boxes and counting one logical mesh per layer in `StreamStats::shown_meshes`),
  and stage in-place LOD transitions across frames in `TerrainStreamer::staging`
  under `STAGED_VERTEX_BUDGET = 48,000` vertices per frame, swapping the batch's
  entities on the frame after its GPU buffers finish allocating so the old LOD
  remains on screen without holes. Added unit test
  `oversized_parts_split_into_vertex_budgeted_chunks_and_share_bounds`.
- `apps/sn-client/src/objects.rs`: track `spawn_cursor` and `slot_objects` per
  `(batch, level)` slot in `BatchObjects` and lower `SPAWN_BUDGET` from `1,500`
  to `600` entities per frame so dense fine-detail slots are spread across
  frames while `settled()` requires `spawn_cursor[l] >= instances.len()`.

Three matched before/after release runs at `--view 400 --resolution 1600 900`:
- Fixed camera (`600` frames): before (P1d) p50 `10.230 / 9.847 / 9.773 ms`,
  p95 `12.436 / 11.884 / 10.998 ms`, p99 `14.109 / 13.694 / 11.826 ms`, max
  `17.956 / 15.741 / 12.434 ms`; after p50 `9.852 / 9.895 / 9.859 ms`, p95
  `10.763 / 11.163 / 10.848 ms`, p99 `11.434 / 11.796 / 11.505 ms`, max
  `12.659 / 16.137 / 11.759 ms`. All nine fixed-camera static snapshots across
  P1d-before, P1d-after, and P1e-after match byte-for-byte (`103` batches,
  `4,393,498` terrain triangles, `589` terrain meshes, `25,311` grass tufts,
  `1,970,238` grass triangles, `17,194` object entities, `0` warnings).
- Player + PDA (`--shot 12`, excluding capture/exit): before (P1d) p50
  `10.812 / 11.726 / 10.566 ms`, p95 `12.423 / 25.528 / 11.997 ms`, p99
  `15.852 / 29.413 / 14.340 ms`, max `27.872 / 36.735 / 28.438 ms`
  (`objects::stream_objects` max `2.460 / 2.264 / 2.296 ms`); after p50
  `10.714 / 10.791 / 10.660 ms`, p95 `12.389 / 12.437 / 12.201 ms`, p99
  `14.301 / 13.974 / 14.628 ms`, max `22.645 / 26.272 / 28.515 ms`
  (`objects::stream_objects` max `0.503 / 0.488 / 0.518 ms`). All runs perform
  the six locker `switch` moves and refuse the pod drop with `0` warnings.
- Swim route (`--shot 35`, excluding capture/exit): before (P1d) p50
  `8.967 / 8.887 / 10.194 ms`, p95 `11.179 / 11.097 / 17.421 ms`, p99
  `12.642 / 12.590 / 20.997 ms`, max `168.571 / 171.400 / 169.033 ms`
  (`render::assets` max `127.600 / 126.900 / 144.364 ms`, `render::mesh_alloc`
  max `123.300 / 122.900 / 143.738 ms`, `objects::stream_objects` max
  `14.316 / 13.904 / 20.892 ms`); after p50 `8.989 / 8.984 / 8.967 ms`, p95
  `11.783 / 11.517 / 11.562 ms`, p99 `16.358 / 16.317 / 16.494 ms`, max
  `25.256 / 26.508 / 23.447 ms` (`render::assets` max `11.021 / 11.635 /
  11.083 ms`, `render::mesh_alloc` max `8.014 / 7.957 / 7.901 ms`,
  `objects::stream_objects` max `1.703 / 2.015 / 1.766 ms`), with `0` frames
  `>= 33.333 ms` across `3,630–3,635` frames and identical end-of-route scene
  counts (`102` batches, `2,950,338` terrain triangles, `16,852` grass tufts,
  `13,171` object entities, `0` warnings).

**Verify:** `cargo test --workspace` (724 passed, 0 failed, 63 ignored without
`SUBNAUTICA_DIR`), `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`.

## 2026-10-27 — P1d: render-stage spike attribution and object GPU upload budget

**What/why:** Extended `SN_CLIENT_PERF=1` (`apps/sn-client/src/perf.rs`,
`main.rs`, `objects.rs`, `terrain.rs`, `game_ui.rs`) to stop sampling at
`play_shot` before screenshot capture/exit, write `out/p1/frames.csv` and
`out/p1/spans.csv` on exit, time `Render` schedule sub-stages
(`render::assets`, `render::images`, `render::mesh_alloc`, `render::meshes`,
`render::specialize`, `render::queue`, `render::prepare`, `render::render`),
and correlate slow frames with main-thread work counters across Bevy's
pipelined extract boundary. Attribution across player and a 35 s swimming/
turning route (`--start -5 -10 -5 --look 200 -10 -5 --hold-forward 1 --turn 8
--shot 35`) identified two dominant gameplay stall causes:
1. Unbounded object asset bursts in `objects::stream_objects`: `library.scenes`
   and `library.batch` buffered whole-scene/batch updates and `stream_objects`
   drained all ready textures and meshes in one frame (152.9 MB textures +
   193,562 mesh vertices at `play=1.36-1.62 s` in player mode -> `render::assets`
   47.6-56.7 ms -> 64.7-70.8 ms frame spike; 45.0 MB textures + 401,807 mesh
   vertices at `play=3.94-3.98 s` in swim mode -> `render::assets` 47.1-49.0 ms
   -> 75.0-78.7 ms frame spike).
2. Unstaged LOD-0 terrain + grass batch uploads in `terrain::stream` during
   movement across streaming boundaries (1.19M-1.58M vertices uploaded in one
   frame at `play=7.6-9.0 s` and `32.4-33.5 s` in swim mode -> `render::mesh_alloc`
   103.1-141.7 ms -> 125.5-174.8 ms frame spikes; addressed next in P1e).

Fixed cause #1 in `apps/sn-client/src/objects.rs`: the object worker now streams
updates through `Out(&tx)` as each asset is decoded instead of holding a full
scene/batch buffer, and `ObjectStreamer` queues updates in `pending: VecDeque`
with per-frame GPU upload budgets (`TEXTURE_UPLOAD_BYTES_PER_FRAME = 8 MiB`,
`MESH_UPLOAD_VERTICES_PER_FRAME = 32,000`, `SPAWN_BUDGET = 1,500`) while
requiring `pending.is_empty()` in `settled()`. Three matched before/after release
runs at `--view 400 --resolution 1600 900`:
- Fixed camera (`600` frames): before p50 `9.776 / 9.789 / 9.736 ms`, p95
  `10.859 / 10.541 / 10.712 ms`, p99 `11.469 / 11.335 / 11.166 ms`, max
  `11.910 / 11.984 / 12.223 ms`; after p50 `10.230 / 9.847 / 9.773 ms`, p95
  `12.436 / 11.884 / 10.998 ms`, p99 `14.109 / 13.694 / 11.826 ms`, max
  `17.956 / 15.741 / 12.434 ms`. All six static scene snapshots match
  byte-for-byte (`103` batches, `4,393,498` terrain triangles, `589` terrain
  meshes, `25,311` grass tufts, `17,194` object entities, `2,599` object
  meshes, `598` materials, `1,079` textures, `0` unsettled frames, `0` warnings).
- Player + PDA (`--shot 12`, excluding capture/exit): before p50
  `10.694 / 11.366 / 10.704 ms`, p95 `12.190 / 26.040 / 12.123 ms`, p99
  `13.816 / 29.635 / 13.425 ms`, max `70.822 / 66.462 / 69.706 ms`
  (`render::assets` max `56.673 / 48.557 / 55.165 ms`); after p50
  `10.812 / 11.726 / 10.566 ms`, p95 `12.423 / 25.528 / 11.997 ms`, p99
  `15.852 / 29.413 / 14.340 ms`, max `27.872 / 36.735 / 28.438 ms` (and
  `21.936 ms` on an isolated warm run; `render::assets` max reduced ~9x to
  `5.719 / 6.519 / 5.863 ms`, `render::images` max `46.940-48.638 ms ->
  5.668-5.867 ms`, `render::mesh_alloc` max `36.749-38.850 ms -> 3.868-5.237 ms`).
  All six runs perform the six locker `switch` moves and refuse the pod drop.
- Swim route (`--shot 35`, excluding capture/exit): `render::images` max drops
  from `17.646 / 18.530 / 17.458 ms` to `4.244 / 3.608 / 5.216 ms` and the
  `play~3.95 s` object burst spike drops from `76.129 / 74.958 / 77.408 ms` to
  `24.290 / 25.000 / 31.679 ms`; whole-run max (`168.571 / 171.400 / 169.033 ms`
  vs before `166.824 / 166.956 / 174.818 ms`) remains governed by the four
  LOD-0 terrain batch uploads (`render::mesh_alloc` 120-144 ms, targeted in P1e).

**Verify:** `cargo test --workspace` (723 passed, 0 failed, 63 ignored without
`SUBNAUTICA_DIR`), `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`.

## 2026-10-27 — P1c: reject inactive UI pruning after matched repeats

**What/why:** reverted the unfinished inactive-hierarchy mesh/material/glyph
pruning. Three alternating eager/pruned rounds with the existing protocol
still do not demonstrate a net gameplay frame-time win. Both first fixed
runs have p99 about 74 ms, confirming shared instability; player p99 remains
worse in the candidate. Full p50/p95/p99/max table and log paths are in
`docs/P1-PERFORMANCE.md`. Worker build cost falls roughly in half, and the
candidate's installed-data oracle passed 32 states / 2,004 visible draws,
but neither proves the required frame-time benefit. Deferred texture/glyph
preparation is a possible activation cost, **not proven**. No dependencies,
whitelist, simulation, or architecture changes. No game data tracked.

**Verify:** `python out/p1/matched.py` documents the archived run sequence
(do not rerun with existing names); exact executable flags are in each log
and the evidence note. `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` run before this commit;
passed (722 tests, 63 ignored; clippy clean; fmt clean). Restored the bounded profiler snapshot counter after
clippy caught P1a using an integer API newer than the declared Rust 1.85 MSRV.
Capture/exit contamination of historical player
maxima is retained explicitly and will be removed from later attribution.

## 2026-10-27 — P1b: construct object texture images on the asset worker

**What/why:** `Library::texture` now runs the existing `textures::to_image`
conversion on its worker and sends the resulting CPU `Image`. The main world
only inserts it into `Assets<Image>`; Bevy still uploads to the GPU. The exact
conversion, mip selection, colour space, sampler and failure behavior are
unchanged. This removes copying/decoding an entire completed prefab burst
from `stream_objects`. No dependencies, game rules or UI changes.

**Measured:** same release commands/settings as P1a; three runs before and
after, evidence `out/p1/p1-{baseline,textures}-{fixed,player}-N.log`.
Before: player `stream_objects` worst 39.214-49.650 ms. After: 0.727-0.779 ms across all three runs. The same 129 late
texture conversions are now worker work, not frame work. Player frame worst
97.468-119.382 ms before, 64.023-69.920 ms after, including
capture/exit. Fixed-camera medians remain within noise: before 9.419-9.544,
after 9.302-9.619 ms; this is a streaming-stall fix, not a steady FPS claim.
Full suite/clippy **not tested yet**; required before P1 completion.

**Verify:** `cargo build -p sn-client --release`; set `SUBNAUTICA_DIR` and
`SN_CLIENT_PERF=1`; run P1a's player command with a new capture name three
times. Expect identical six accepted locker moves/refused pod drop, 129 late
texture conversions on the asset worker and main `stream_objects` worst
below 2 ms on this machine. Fixed-camera P1a command: same terrain/object/
texture counts, zero unsettled frames, warnings and shader errors.

## 2026-10-27 — P1a: opt-in client CPU/frame evidence and benchmark repair

**What/why:** P1 plan added to DESIGN before implementation. Set
`SN_CLIENT_PERF=1` to collect bounded wall-time distributions for client
systems, UI worker/build/render preparation and effects; frame histogram,
p50/p95/p99/max; separate loading/warm-up and benchmark/player samples.
Recent Bevy render CPU diagnostics are captured every 120 frames when
`--gpu-timings` is on. README documents the scopes and commands. No dependencies,
whitelist changes or rendering/simulation changes.

**Benchmark blocker:** the first release `--benchmark` panicked because
`game_ui::pda_pose` required `BodyDrive`, absent without a player. Its input is
now optional, yielding no PDA pose in free-camera mode. Added a synthetic
regression test. This is a harness repair, not a measured FPS improvement.
Dead-end evidence: `out/p1/p1-before-fixed-1.log` (exit 101); do not treat it
as a baseline. Dev is already opt-level 1 (our code), 3 (dependencies);
README's normal solo commands use dev, but unoptimized dependencies are not
the cause. Dev/release runtime A/B: **not tested**.

**Measured** on i9-10900K / RTX 3080, Vulkan, release; evidence only in ignored
`out/p1/` and `out/baselines/`. Three runs each, no other client process observed:
- Fixed camera: 600 frames after 120 settled warm-up frames, view 400,
  1600 × 900, fixed seeds/time. p50 9.419-9.544 ms, p95 10.631-10.746,
  p99 11.246-11.380, max 12.368-12.518. All zero unsettled frames.
- Player `--use-item 3 --panel 6=take-all,7=drop,9.5=close --shot 12 NAME`:
  play >= 1 s, p50 10.371-10.449, p95 11.669-11.957,
  p99 12.983-15.460, max 97.468-119.382 ms (includes capture/exit work).
- Player main-thread `stream_objects` max 39.214-49.650 ms; late texture
  conversions total 35.088-43.215 ms (129 calls). UI worker build mean
  3.973-4.008 ms; fixed-camera 3.624-3.649 ms. Animation mean
  0.295-0.299 ms, LOD 0.136-0.142; terrain stream 0.008-0.009.
- GPU recent fixed-camera samples in the first run: opaque 0.870 ms,
  shafts 0.835, bin unpacking 0.459, transparent 0.446. These are individual
  recent pass averages, not additive full-frame GPU timings.

**Reproduce:** `cargo build -p sn-client --release`; set `SUBNAUTICA_DIR` and
`SN_CLIENT_PERF=1`; use the fixed-camera command in README with `--view 400
--time 12 --benchmark 600` and a fresh name; repeat three times. For the player,
use the script above plus the same view/resolution/seeds/time/water settings.
Expect `perf:` distributions/histograms on exit, six accepted locker moves,
the refused pod drop, no warnings. Full commands are recorded in the logs.
Release build and six real-data runs passed; workspace tests/clippy and the
new unit test **not tested yet** (required before P1 completion).

## 2026-10-20 — M9u8b3a: hover and clicks on the game's inventory tab; our panel removed

**What/why:** the next step of M9u8b (DESIGN). M9u8b3 was split: b3a
(hover, clicks, our panel removed) and b3b (the tooltip, its own system).
- `sn-ui::inventory_tab`: `hit` (the icons' and slots' rects on the
  canvas), `set_hovered` (`OnPointerEnter`/`Exit`: `HighlightSlots`,
  `ExtinguishSlots`, `tintHighlighted`), `on_pointer_click`; 1 unit test.
- `sn-inspect ui --inventory`: hits at every icon and slot centre, an
  empty cell, hover tints, nothing drawn over the icons.
- `sn-client`: `inventory.rs`'s Bevy panel removed (`setup`, `draw`,
  `hover`, the item boxes). Every mouse click while the PDA is focused
  goes to the UI worker (`ScreenClick`, was `ToolbarClick`). So does the
  pointer, for the hover. The worker hit-tests the toolbar, then the tab
  (`screen_click`) and sends item clicks back (`UiBuild::item_clicks` →
  `GameUiFrame::item_clicks`). `inventory::input` acts on them with
  M9d5b's `click` (the game's item action). `--panel`'s item steps click
  at the items' icon centres through the same path. Each item carries the
  player's slots that take it (`TabEntry::compatible`, from the sim's slot
  table).
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- `sn-inspect ui --inventory` (`ui-inventory-b3a.log`): RUN OK, 53 of
  53.
- Client `--use-item 3 --panel 6=take-all,7=drop,9.5=close --shot 11
  m9u8b3a-locker` (`b3a-locker.log`, 0 WARN/ERROR/panic): six clicks
  hit-tested on the canvas to `Item(4)`, `Item(5)`, `Item(0–3)`, each
  "Left click on … (Storage): actions [switch] → switch"; the right click
  hit `Item(4)` in the grid → "none", "no drop in the lifepod
  (CanDropItemHere)": M9d5b's results.
- Screenshot `out/m9u8b3a-tab.png` (`--panel 6=take-all --shot 7.5`):
  the game's tab alone, the six icons in the grid, the locker empty.
- `cargo test --workspace` 721 passed, 0 failed; clippy clean; `ui --pda`,
  `--draw`, `--icons` unchanged.
- Not tested: hover and clicks with a real mouse. In the scripted runs
  the cursor never rested on an icon, so no hover was logged; hover is
  checked only headless.

## 2026-10-20 — M9u8b2b: the icons' round backgrounds and the doll's circles

**What/why:** the second half of M9u8b2 (DESIGN). The item icons'
backgrounds are 2-pixel strips that only `UI/Icon`'s `SLICE_9_GRID`
program turns into the rounded, shaded tile. The doll's slots use
`UI/Circle`. Both drew as `UI/Default`.
- `sn-ui::icon`: `slice9_uv` and `circle_uv`, Rust references of the two
  fragment programs' sample coordinates (read from M9u7c's and M9u5's
  D3D11 disassembly); 2 unit tests with values worked by hand.
- `game_ui.wgsl`: the same two mappings (flags 32 and 64; their values
  in `st`; the texture's height from `textureDimensions`).
- `game_ui.rs`: `UI/Icon` with only `SLICE_9_GRID` and `UI/Circle` take
  those flags; the census is logged again whenever its shader set changes
  (the PDA's draws come after the first build).
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- Client `--use-item 3 --panel 6=take-all,7=close,9=open,10=tab4 --shot
  11.5 m9u8b2b-census` (`b2b-census.log`, 0 WARN/ERROR/panic, no GPU
  validation error): "census changed: … UI/Circle 7 … UI/Icon SLICE_9_GRID
  6 …; drawn as UI/Default {UI/Bar (LoadingBar): 1}" (the loading bar,
  not in scope).
- Screenshots `out/m9u8b2b-grid.png` and `-locker.png`: round blue icons
  with a shaded rim, and the doll's slots as circles.
- `cargo test --workspace` 720 passed, 0 failed; clippy clean; `ui
  --inventory`, `--pda`, `--draw`, `--icons`, `--icon-mesh` unchanged.
- Not tested: the WGSL against the Rust reference by execution (it is a
  line-by-line mirror); the look against the game on screen.

## 2026-10-20 — M9u8b2a: the inventory tab's item and slot icons

**What/why:** the next step of M9u8b (DESIGN). M9u8b2 was split into
b2a (the icons by the game's rules) and b2b (the shaders). `UI/IconBar`
and `GRAYSCALE` were cut from b2 and recorded in DESIGN: our sim has a
battery's charge but not tools' charge, food decay or disabled items.
- `sn-ui::item_icons` (new): the cell rules, `ContainerIcons::sync`
  (`OnAddItem`, `OnRemoveItem`, `OnChangeItemPosition`), the slots'
  `SetItem`/`ClearIcon`; 5 unit tests. `sn-ui::inventory_tab`:
  `sync_items`, `MakeIcon`, the slots' `iconRect`; `Uninit` clears.
- `sn-assets::item_icons` (new): `background_sprites` (`Resources`),
  `resource_sprite`, `tech_icon` (size and background type),
  `item_icon_nodes`.
- `sn-inspect ui --inventory`: the icon checks.
- `sn-client`: each frame the player's container, the locker and the
  equipment go to the UI worker (`TabContents`); the worker syncs the
  icons when they or the PDA's flags change.
No new dependencies.

**Found:** the background sprites are 2-pixel strips (Normal 2 × 33,
with 33 the icon radius). They only look right through `UI/Icon`'s
`SLICE_9_GRID` program, so until b2b they draw as flat blue squares.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- `sn-inspect ui --inventory` (`ui-inventory-b2a.log`): RUN OK, 42 of 42.
- Client `--use-item 3 --shot 6.5 m9u8b2a-locker` and `--use-item 3
  --panel 6=take-all,7=close,9=open --shot 11 m9u8b2a-grid` (`b2a-*.log`,
  0 WARN/ERROR/panic): "locker icons: 6 shown", then after take-all
  "inventory icons: 6 shown", "locker icons: 0 shown"; the screenshots
  show the icons in the locker's top row, then in the grid's.
- `cargo test --workspace` 718 passed, 0 failed; clippy clean; `ui --pda`,
  `--draw`, `--icons`, `--icon-mesh` unchanged.
- Not tested: equipped items in the client (none of the starting items
  equips); against the game on screen.

## 2026-10-20 — M9u8b1: the inventory tab's containers and doll (plan for M9u8b)

**What/why:** M9u8b (the inventory tab) was planned from the decompiled
`uGUI_InventoryTab`, `uGUI_ItemsContainer`, `uGUI_Equipment`,
`uGUI_EquipmentSlot` and `uGUI_ItemIcon` and split into M9u8b1–b5 in
DESIGN. This first step runs the tab's `Awake`s, `Start`, `OnOpenPDA` and
`OnClosePDA`, without icons. It explains what M9u8a4b left open: the "round
gauges and arrow block" seen on every tab are the doll's slot art.
`uGUI_Equipment.Awake` hides all 38 slots, and we did not run it.
- `sn-ui::inventory_tab` (new; 4 unit tests); `UiState::uv_rect`
  (`RawImage.uvRect`, applied when drawn).
- `sn-assets`: `inventory_tab_config` (containers, grids, the doll's
  named slots, labels, the language texts), `INVENTORY_TAB_KEYS`.
- `sn-inspect ui --inventory` (new).
- `sn-client`: the worker builds the tab (the locker's size and label from
  the lifepod, the player's slots and container size from the game's
  code) and calls it from the PDA's flags; `PdaFlags::with_locker`.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- `sn-inspect ui --inventory` (`ui-inventory.log`): RUN OK, 22 of 22. My
  first run failed one check of my own: I compared the shown slots in the
  player's list order, but the game shows them in hierarchy order. Now
  compared as sets.
- Client `--panel 3=open --shot 6 m9u8b1-equipment` and `--use-item 3
  --shot 6.5 m9u8b1-locker` (`b1-*.log`, 0 WARN/ERROR/panic): the doll
  shows only the player's slots under "EQUIPPED"; with the locker the
  doll is hidden and the 4 × 8 grid is under "STORAGE".
- `cargo test --workspace` 713 passed, 0 failed; clippy clean; `ui
  --pda`, `--draw`, `--icons` unchanged.
- Not tested: against the game on screen. The slots' `UI/Circle` art is
  still drawn as `UI/Default` (M9u8b2). Our Bevy panel still covers part
  of the tab.

## 2026-10-20 — M9u8a4b: the PDA's tab bar and tab clicks

**What/why:** the last step of M9u8a (DESIGN): the PDA screen's toolbar
as the game makes it, and clicks on its tabs.
- `sn-ui::toolbar` (new): `uGUI_Toolbar`'s `Initialize` (icons placed
  by the rule, the bar's width, the marker hidden), `Select` (the
  backgrounds' sprites, the marker at the icon's `localPosition` + offset),
  the click hit test; `ToolbarConfig::lay_out`. `sn-ui::pda`:
  `attach_toolbar`, `SetTabs` (at `Initialize` and `OnClosePDA`),
  `Select` from `OpenTab`, `on_toolbar_click`. `sn-ui::scaler`:
  `UiCamera::screen_ray`, `Placement::ray_point`; `sn-ui::draw`:
  `invert`, `to_ancestor` made public. 6 new unit tests.
- `sn-assets`: `pda_toolbar` (the stored fields, the two background
  sprites), `toolbar_icon_nodes` (`ToolbarIcon` + `Background` +
  `Foreground`, made by code), `GROUP_TAB`.
- `sn-inspect ui --pda` (new, `pda_tabs.rs`): the headless check below.
- `sn-client`: the UI worker grafts the six icons and attaches the
  toolbar; a mouse click on the PDA (not on our panel) and `--panel …
  tabN` go to the worker, which hit-tests it on the canvas at the frame's
  pose and calls `OnToolbarClick`.
No new dependencies.

**Found:** the bar is 778 wide (6 × 143 − 5 × 16); its sprites
`TabBackgroundNormal`/`Selected` are 143 × 54, not Resources sprites
(`SLICE_9_GRID` off); `SetBackgroundSprite` resizes without keeping the
aspect (no visible difference with these sprites). On the way I noticed
that M9u7c's notifier sets `_FillRect` from the fitted foreground size,
where the game uses the desired size (`SetSize`'s); our `UI/Icon` shader
does not read `_FillRect`, so nothing drawn changes; not fixed here.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- `sn-inspect ui --pda` (`ui-pda.log`): RUN OK, 173 of 173 checks.
- `sn-client --view 400 --resolution 960 540 --panel 3=open,4.5=tab1,
  5.5=tab4,6.5=tab0,7.5=tab3,8.5=close,10.5=open,12=close --shot 14
  m9u8a4b-after` (`a4b-tabs.log`): 6 icons grafted, bar 778; each click
  logged with its pixel, canvas point (e.g. icon 1 at canvas
  (−190.5, 400.0), bar x −190.5 = −389 + 71.5 + 127) and the tab opened
  (Journal, Log, Inventory, Gallery); the reopen opened Gallery
  (`tabPrev`); 0 WARN/ERROR/panic.
- Screenshots `out/m9u8a4b-inventory.png`, `out/m9u8a4b-log.png` (1920 ×
  1080): the six tab icons on their slanted backgrounds, the open tab's
  background highlighted with the marker under it; after the Log click
  the highlight and marker moved and the inventory tab's content went.
- No regression: `ui --icons` 10/10, `--icon-mesh` 12/12, `--draw` 12 of
  12 hashes equal, `--popup` 13/13; `cargo test --workspace` 709 passed,
  0 failed; clippy clean.
- Not tested: a click with the real mouse (only scripted clicks, which
  take the same ray path from a projected pixel); the bar against the game
  on screen. Our M9d5b panel still shows on every tab, and two round
  gauges and an arrow block on the right show on every tab too (also before this
  change; not identified yet).

## 2026-10-18 — M9u8a4a: the PDA's screen on the PDA; its glass no longer see-through

**What/why:** the user saw the PDA's screen almost transparent outside the
pod (M9u8a3 drew the model only). DESIGN's M9u8a4 was split into a4a (the
screen canvas) and a4b (the tab bar and clicks).
- `sn-ui::pda` (new): `uGUI_PDA`'s states (`Awake`, `Initialize`,
  `OnOpenPDA`, `OnPDAOpened`, `OnClosePDA`, `OnPDAClosed`, `OpenTab`; the
  tabs' `content` groups or GameObjects; `UpdateCycleBtnText` with the
  keyboard); 4 unit tests.
- `sn-assets::pda_screen` (new): `pda_screen_tree`, `pda_screen_config`
  (the script's nodes, each tab's `content`, the toolbar's gamepad labels).
- `sn-client`: the UI worker grafts `uGUI_PDAScreen(Clone)` into the scene,
  places its canvas at its stored size and scale and runs `uGUI_PDA`'s calls
  from the PDA's flags; `pda_pose` (after the transforms propagate) gives the
  render pass `ScreenAnchor`'s pose in the camera's space every frame
  (`NamedNodes`, `RigDesc::named`). The PDA mesh's materials are the view
  model (`MaterialDesc::view_model`); blended, they write depth
  (`ObjectKey::depth_write`, our object material's first pipeline key).
No new dependencies.

**The see-through glass, found by experiments** (throwaway builds and
probes, not kept): (1) the glass's texture alpha over the screen is 0.832
(area-weighted), not 0: my first probe sampled the top-row-first decode
upside down; (2) forced opaque, the screen showed the dark navy fill; (3)
drawing its alpha as grey showed the glass drawn in the pod but missing
under water, with or without the water fog and water surface; (4) cause:
the terrain's blended layers sort by batch centre + rank, after objects
near the camera, and with the glass writing no depth they painted over it;
the sky (and the fog and water surface) do the same per pixel after the
transparent pass. **Dead end:** moving the sky, fog, stars and water
surface before the transparent pass (the game's order) fixed the sky but
not the terrain, and leaves the terrain's blended layers unfogged; reverted.
The fix kept: the view model's blended parts write depth (`materials.md`).

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`, 0 WARN/ERROR/
panic in all):
- `sn-client --view 400 --resolution 960 540 --panel 3=open,6=close --shot 9`
  (`a4-tab.log`): 402 nodes grafted, canvas 0.2531 × 0.2055 m; `OnOpenPDA`
  at 5.095 s: canvas 1, content 1, toolbar 1, tabs shown [Inventory] of 8;
  `OnPDAOpened` 0.521 s later; the pose at rest (0.0217, −0.0012, 0.1874) m,
  +z 0.89° from the camera's (M9u8a2: the same); `OnClosePDA`: content 0,
  no tab; `OnPDAClosed` 0.319 s later: canvas 0.
- Screenshots `out/m9u8a4-pod.png`, `-water.png`, `-surface.png` (1920 ×
  1080, `--panel 3=open --shot 6`): the screen dark blue glass with the
  canvas on it in all three; the world faintly through it, as its 0.83 alpha
  gives; no "?" labels.
- `cargo test --workspace` 703 passed, 0 failed; clippy clean.
- Not tested: against the game on screen; the toolbar's icons and tab
  clicks (M9u8a4b); the screen's draw counts (its hidden groups stay in the
  draw list at alpha 0, so a count says little).

## 2026-10-18 — M9u8a3: the PDA in the hand (client)

**What/why:** the third step of M9u8a (DESIGN): the client opens and
closes the PDA by the game's rules (`sn_sim::pda`), draws its model in
the left hand while it is active, runs the arms' animation and the
camera's locked mode, and replaces M9d5b's own open/close flags.
- `inventory.rs`: `Panel::pda` (an `sn_sim::pda::Pda`); Tab opens it
  (`GUIHand`) and closes it once focused (`ManagedUpdate`); the locker's
  PDA keeps its distance to `EscapePod.storagePivot` (M9d5b used the
  locker collider's centre); `Activated` frees the cursor, `Close` locks
  it; the panel (our stand-in for the inventory tab until M9u8b) shows
  from `Open` to `Close` and takes clicks once focused; `--panel`'s clicks
  wait while the PDA opens; death closes an opening or open PDA once and
  ignores PDA input until the controls come back (`Player.OnKill`).
- `player.rs`: the PDA in use locks the look (the pitch eased level by
  `Look::ease_level`; the mouse before `Activated` turns the look-around
  offset), turns the hand off and fills `BodyFrame`'s PDA fields;
  `storage_pivot()`; `locker_centre()` removed (no user left);
  `--hold-back <SECONDS>` (hold S, to walk away from the locker).
- `objects.rs` / `animation.rs`: `RigDesc::switched` (nodes the game
  switches on and off): the player's body is built with the PDA node
  active and that node marked; it is spawned hidden and listed on the
  rig's base (`SwitchedNodes`). `body.rs`: shows it while
  `BodyDrive::pda_model`; `arm_states` logs the player's `Base Modes` and
  `upperbody` transitions. Remote players' PDAs stay hidden (not ported).
- `game_ui.rs`: the HUD's `pda_open` is `PDA.isInUse`.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`, 0 WARN/ERROR/
panic in all six):
- `sn-client --view 400 --resolution 960 540 --panel 3=open,6=close --shot
  8 m9u8a3-after` (`client-tab.log`): `PDA.Open` at 3.020 s, `Activated`
  0.511 s later by `PDA.time`, `Close` at 6.020 s, `Deactivated` 0.300 s
  later (+ a frame); `Walking → pda`, `New State → first_check → open`,
  `open → idle` 0.946 s after the open; `idle → close`, `close → New
  State` 0.382 s after the close (M9u8a2, 50 Hz: 0.980 / 0.400 s).
- `--use-item 3 --panel 6=take-all,7=drop,9.5=close --shot 11
  m9u8a3-locker` (`client-locker.log`): M9d5b's results: 6 left clicks
  "switch", the right click "no drop in the lifepod"; the PDA opened 0.25
  s after `storage_open`, `idle` 0.940 s and `New State` 0.377 s after.
- `--use-item 3 --hold-back 6 --shot 10` (`client-walkaway.log`): opened
  2.56 m from `storagePivot` (limit √(2.56² + 1) = 2.75 m), closed by
  distance at 2.79 m 0.12 s after S was held, `storage_open` off.
- `--use-item 3 --kill 6 --panel 20=open,21=close` (`client-death.log`):
  one close at the death ("the controls went off"), the model hidden
  0.300 s later; after the respawn Tab-style open and close work.
- Screenshots at 1920 × 1080: `out/m9u8a3-midway.png` (0.35 s into the
  opening: the hand bringing the PDA up), `out/m9u8a3-open.png` (open:
  the PDA fills the view; its screen is the bare `PDA_screen` glass, the
  screen canvas comes in M9u8a4; the health icon shows its numbers).
- **Bug found on the way:** my first death rule called `Close` every
  frame while the controls were off, and the game's `Close` runs again
  while closing, so it logged a close per frame. Now it follows
  `Player.OnKill`: one close, then input ignored.
- Not tested: the arms and the model against the game on screen;
  keyboard and mouse play by hand.

## 2026-10-18 — M9u8a2: the arms with the PDA (`walk --pda`)

**What/why:** the second step of M9u8a (DESIGN): run the player's
animator with the PDA's rules, headless, and check the arms' states and
the screen anchor's place before the client draws them.
- `sn-inspect walk --pda` (new, `walk_pda.rs`): stands in the pod, opens
  the PDA, holds it 2 s, closes it, waits 1 s; leaves through the hatch,
  swims 5 s, does it again under water. Checks each cycle's `Activated` /
  `Deactivated` times, the model's active span, the `upperbody` layer's
  transitions and their times, each phase's states (written in DESIGN
  before the run), and logs the screen anchor in the eye's space.
- `walk`'s `Run` steps an `sn_sim::pda::Pda` before the body (the locked
  mode's look easing; `using_pda` and `pda_in_use` into the body);
  `BodyRun` watches the four arm layers too and gives the PDA's and the
  anchor's poses; `check_body` takes its "never" list.
- `sn-assets::PlayerAnimation` gains the PDA's and `screenAnchor`'s posed
  nodes.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set; logs in `out/m9u8a/`):
- `sn-inspect walk --pda`: RUN OK. Both cycles: `Activated` 0.520 s after
  the open, `Deactivated` 0.340 s after the close (rule: 0.5 / 0.3 s + one
  or two 0.02 s steps); the model active exactly in between; `upperbody`
  `New State → first_check → open → idle` with `idle` starting 0.980 s
  after the open (expected about 0.93 s), `idle → close → New State` with
  `New State` 0.400 s after the close (expected about 0.375 s); base layer
  `Walking`/`pda` in the pod, only `Swim` under water; the other arm
  layers in their defaults. At rest, the screen anchor at (0.0217,
  −0.0012, 0.1874) m from the eye, its +z 0.89° from the eye's; camera
  pitch 0, offset z 0.1800 m. 1162 steps, 0 NaNs, worst |q| − 1 1.2e-7.
- No regression: `walk` RUN OK (12 checks ok), `dive` RUN OK (31 ok);
  `cargo test --workspace` 699 passed, 0 failed; clippy clean.
- Not tested: the arms against the game on screen (M9u8a3 shows them).

## 2026-10-18 — M9u8a1: the PDA's rules and data (plan for M9u8a)

**What/why:** M9u8a ("the PDA itself") was planned from the decompiled
`PDA`, `uGUI_PDA`, `uGUI_CanvasScaler`, `uGUI_Toolbar`, `uGUI_PDATab`,
`ArmsController`, `MainCameraControl` and `Sequence`, a throwaway probe of
the main scene and `sn-inspect anim scene:main --states`, and split into
M9u8a1–a4 in DESIGN. This first step is headless: the PDA's open and
close and the camera's locked mode as pure rules, and the data the
client will need. Facts in `docs/formats/gameplay.md` § The PDA.
- `sn-sim::pda` (new): the game's `Sequence` in floats with `Activated` /
  `Deactivated`, `PDA.state`, `Open` (refused in use, with input ignored
  or in a cinematic; `activeSqrDistance`), `Close` (runs again while
  closing, as the game's), `PerformUpdate` / `ManagedUpdate` (only while
  the model is active; the button when focused; the distance close),
  `PDA.time`.
- `sn-sim::body`: the locked mode of `MainCameraControl.OnUpdate`
  (`BodyFrame::pda_in_use`, `locked_look`): `camRoot`'s and
  `cameraUPTransform`'s pitch eased level, the look-around offset on
  `cameraOffsetTransform` (±60°, pulled back), its z moved up to
  `camPDAZOffset`, no bob; `CameraPose` gains the offset's angles and z
  (in `rotate`, `eye`, `camera`, `camera_pitch`); `using_pda` comes from
  the frame (`BodyFrame::using_pda`) and moved from `FIXED_PARAMETERS` to
  `RULE_PARAMETERS`. `Look::ease_level`.
- `sn-assets`: `PlayerBody::pda` (`PlayerPda`: the `PDA` script's node,
  its mesh node, `screenAnchor`'s node, `prefabScreen`);
  `LifepodLocker::storage_pivot`. `sn-unity::EscapePod` reads up to
  `storagePivot`.
- The client and `sn-inspect walk` pass "PDA not in use" until M9u8a2–a3.
No new dependencies.

**Verified:**
- `cargo test --workspace`: 699 passed, 0 failed (new: 5 `pda` tests,
  `the_pda_locks_the_camera`, the longer `EscapePod` in
  `escape_pod_and_spawners`); `cargo clippy --workspace --all-targets`
  clean; `cargo fmt` applied.
- Real data (`SUBNAUTICA_DIR` set; `cargo test --release -p sn-assets
  --test real_data -- --ignored the_pda_in_the_hand script_layouts_agree
  hand_targets_follow`): 3 passed. The PDA at
  `…/hand_L/attachL/PlayerPDA`, inactive, materials `PDA`, `PDA_border`,
  `PDA_screen` (all MarmosetUBER), `ScreenAnchor` at (−0.20517, 0, 0),
  rotation (0, 1, 0, 0), scale 0.1; 10 objects agree with the generic
  reader (new: `EscapePod` with `storagePivot`); `storagePivot` at
  (−130.009, 0.782, −49.769), 0.000 m from the locker's hand-target node.
- **Found on the way:** M9d5b's "the PDA's place is the player's
  position" is the game's own rule, not a stand-in; its other stand-in
  (the hand collider's centre for `storagePivot`) is replaced in M9u8a3.
  A test of mine expected `Close` to refuse while closing; the game's does
  not, so the test was wrong and now checks the game's behaviour.
- Not tested yet: anything on screen (M9u8a3–a4).

## 2026-10-17 — M9u7d1–d2: the unlock pop-up (`uGUI_PopupNotification`)

**What/why:** M9u7d's plan took "a pick-up that unlocks a blueprint" as
the pop-up's input. The decompiled code says otherwise: `Inventory.Pickup`
analyses with `verbose: false`, so a pick-up never shows the pop-up; only
verbose `KnownTech.Add`s do (crafting's story goals, scanning, databoxes;
none ported). And `NotificationManager` has no HUD view (the PDA's item
icons and tab counts). DESIGN's M9u7d was rewritten (2026-10-17):
`NotificationManager` moves to M9u8b; M9u7d1 ports the pop-up, M9u7d2
shows it in the client through a test flag. Facts in `docs/formats/ui.md`
§ The unlock pop-up.
- `sn-ui::popup` (new): `Enqueue`, `Show`, `Hide`, `ShowNext`,
  `OnUpdate`'s phases and value (`Trapezoid`, `Step`,
  `Mathf.Approximately`), `SetSkin`, the default skin's `OnShow` and
  `SetTransition` (`GetRectPositions` at `MiddleLeft`, `EaseOutSine`);
  sounds named, not played.
- `sn-assets::popup` (new): `popup_config` (the pop-up's and skins'
  nodes and fields), `analysis_popups` (`PDAData.analysisTech`'s message,
  picture and sound by tech type, first entry wins as in `Analyze`),
  `analysis_entry` (`OnAnalyze`), `popup_layout`, `fmod_asset_name`;
  `PlayerData::pda_object`; `Debug` for `ObjectRef` (logs).
- `sn-client`: the UI worker runs the pop-up every frame (idle: only
  `PDA.time` moves); `--analyze <TechType>[,…]` queues `OnAnalyze` for
  each once the HUD is active; the log names each entry, phase and sound
  and checks each entry's time up.
- `sn-inspect ui --popup` (new); a real-data test
  `the_unlock_popup_is_read`.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --popup`: 13 of 13; 1200 frames, 0 differing from the replay (value
  0.0000000, position ≤ 0.00004); shown power cell 0.0000, decoy 5.5014,
  wiring kit (`ShowNext`) 8.0445, double tank 13.5458 s; 240 draws, 0
  differing (four items while shown, moving with the skin ≤ 0.0001; none
  while hidden; other nodes unchanged).
- Client, `sn-client --resolution 1920 1080 --start -197.3 -35.2 -10.2
  --analyze PowerCell,CyclopsDecoyModule --shot 15 m9u7d-after`
  (`out/m9u7d/client-analyze.log`): "NEW BLUEPRINT SYNTHESIZED" / "Power
  cell" (defaultSprite) at 1.574 s, done after 5.5137 s; "Cyclops Decoy
  Tube Upgrade" (its picture) right after, done after 5.5010 s (both ok:
  5.5 s + under a step); 0 WARN/ERROR/panic. The first went from `Zero`
  to `One` without `In`: a 0.25 s start-up frame (the game's rule does the
  same with such a step). With `--analyze CyclopsDecoyModule --shot 1.0
  m9u7d-popup`: `out/m9u7d-popup.png` shows it at the left.
- **Bugs found on the way:** `Popup::idle` called the pop-up idle on the
  frame it was done and had just shown the next entry, so the client
  stopped updating it and the second entry never slid in (`ui --popup`
  updates every frame and missed it; a unit test now covers it). The
  replay first summed `PDA.time` in doubles and differed from the game's
  float sum by up to 4e-4 in value after 8 s; the replay now steps it as a
  float.
- No regression: `ui --instantiate` 10 of 10; `--messages` 7 of 7;
  `--death` 24 of 24; `--reticle` 22 of 22; `--compass` 644 / 0;
  `--depth` 91,230 / 0; `--bars` 300 / 0; `--start` 21 of 21;
  `--tmp-sprites` 371 of 371; `--draw` 12 of 12 hashes equal; `--live`
  start hash `efa1606eb7c79aff`; `--meshes` 0 failing; `--pulse` exit 0;
  `--icon-mesh` 12 of 12; `--icons` 10 of 10. Real-data tests
  (`sn-assets`): 56 passed.
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 693 passed, 0 failed
  (new: 5 pop-up tests). fmt and clippy
  (`-D warnings`) clean.

**Not tested:** the `Journal` skin, `OnEncyclopediaAdd` and
`OnResourceDiscovered` (not ported: no encyclopedia or map room);
`Hide` by itself; a resize or UI scale change while shown; a negative
duration. The game's own sources of the pop-up are not ported. Not
compared against the original game on screen.

## 2026-10-17 — M9u7c1–c2: the picked-up item icons (`uGUI_IconNotifier`)

**What/why:** a picked-up item showed no icon. The game's icon is a node
made by code with its own graphic (`uGUI_Icon`, a sprite's packed mesh)
and shader (`UI/Icon`), which the scene could not draw. Facts in
`docs/formats/ui.md` § Picked-up icons.
- `sn-unity`: `SpriteMesh::decode` (a sprite's vertex positions and
  indices, checked).
- `sn-ui::icon` (new): `uGUI_Icon.OnPopulateMesh` (packed mesh or quad),
  `RectTransformExtensions.Fit`, `Sprite.uv` from `uvTransform`
  (*hypothesis*: the stored UV stream is zeros), `_FillRect`.
- `sn-ui::notifier` (new): `uGUI_IconNotifier` (`Play`, `Update`'s queue
  and pool, `Animate` with the curves, `Oscillation` and the release).
  `sn-ui::state`: `LocalTransform::size_delta`; `hud::Random::new`.
- `sn-assets`: `UiScene::from_code` (nodes made by code, grafted as
  M9u7a's; `UiNode::game_object` is now optional), `UiGraphicKind::Icon`
  (drawn with its own material and its sprite's mesh and page),
  `icons.rs` (`notifier_config`, `preloaded_shader`, `icon_material` =
  `new Material(UIIcon)` from the shader's defaults, `icon_sprite`,
  `notifier_icon_nodes`); `sprite_render_data` shared with `sprite_info`.
- `sn-client`: the frame's own accepted `PickedUp` events become
  `Play(techType, From)` (the tech's name → `SpriteManager.Get`); the UI
  worker runs the notifier, grafts the icons, logs each one's life;
  `UI/Icon`'s variant without keywords is drawn by `UI/Simple`'s path
  (other variants named in the log). `Request::Frame` boxed (clippy).
- `sn-inspect ui --icon-mesh` and `ui --icons` (new); a real-data test
  `the_icon_notifier_is_read`.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --icon-mesh`: 12 of 12; 21 icons (20 tech types and `Unknown`)
  each draw one item with `UI/Icon`, their sprites' vertices and
  triangles about one centre, UVs within 0.15 px of the packed rects;
  `SetAlpha(0.5)` → vertex alpha 128; 55 stored draws unchanged.
- `ui --icons`: 10 of 10; 400 frames, 0 differing from the replay
  (position ≤ 0.00017 canvas units, scale ≤ 0.000002, alpha ≤
  0.000001); 10 icons shown, ≥ 0.3 s apart; at most 3 out, 4 made; 80
  draws, 0 differing.
- Client, `sn-client --resolution 1920 1080 --start -197.3 -35.2 -10.2
  --pick-up 3 --shot 9 m9u7c-after` (`out/m9u7c/client-pick3.log`): 3
  acid mushrooms picked up → 3 `Play`s, shown 0.304 and 0.311 s apart,
  each released when its steps reached 0.7092 / 0.7107 / 0.7105 s (ok:
  0.7 s + under one step); 0 WARN/ERROR/panic. With `--shot 3.45
  m9u7c-pick`: `out/m9u7c-pick.png` shows the icon at the screen's
  centre.
- **Fix on the way:** the client's first release check measured from the
  frame after the shown one to the release frame and called two releases
  short (0.6998, 0.6954 s); t steps from the shown frame to the frame
  before the release, which the log now sums.
- No regression: `ui --instantiate` 10 of 10; `--messages` 7 of 7;
  `--death` 24 of 24; `--reticle` 22 of 22; `--compass` 644 / 0;
  `--depth` 91,230 / 0; `--bars` 300 / 0; `--start` 21 of 21;
  `--tmp-sprites` 371 of 371; `--draw` 12 of 12 hashes equal; `--live`
  start hash `efa1606eb7c79aff`; `--meshes` 0 failing. Real-data tests
  (`sn-assets`): 55 passed.
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 688 passed, 0 failed
  (new: 1 sprite mesh, 4 icon, 5 notifier tests). fmt and clippy (`-D
  warnings`) clean.

**Not tested:** the `To` animation in the client (batteries, crafting:
not ported; covered by `ui --icons`); a resize while an icon is shown;
a UI scale other than 1; the order of `uGUI_IconNotifier.Update` and
`PDA.UpdateTime` in the game (*hypothesis*). The icon starts when the
server's event comes, not on the click's frame. Not compared against
the original game on screen.

## 2026-10-17 — M9u7b: the HUD's text messages (`ErrorMessage`)

**What/why:** the game's messages ("You died.", "Inventory full"…) were
not drawn. Facts in `docs/formats/ui.md` § Text messages.
- `sn-ui::messages` (new): `ErrorMessage` ported: `_AddMessage` (merging
  into "(x{n})"), the pool of copies made four at a time, `OnLateUpdate`
  (expiry with the game's skipping loop, the `offsetY` slide, the flight
  and the fade with `EaseOutSine`, `GetYPos`), in the game's frame order
  (hand calls before `PDA.UpdateTime`, coroutines after).
- `sn-assets`: `messages_config` (stored fields, the times from the
  constructor and `OnLateUpdate`'s constants from the IL, the template
  read as a scene with its font colour), `message_layout`,
  `message_times`; `UiScene::layout_for`, `preferred_size`,
  `stored_rect` (`draw`'s layout step moved into a shared helper).
- `sn-assets::hand`: `AimAction::Full` (a click with no room).
- `sn-client`: the player sends the frame's message keys
  (`YouDied`/`YouDiedLostStuff` at controls back, from `LoseItems`'s
  result; `InventoryFull` on a click with no room); the UI worker runs
  the rules, grafts the copies and logs each message's life; `convert`
  reads the scene's grown tree.
- `sn-inspect ui --messages` (new); a real-data test
  `the_messages_are_read`.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --messages`: 7 of 7; 900 frames, 0 differing from the replay
  (position error ≤ 0.0075 canvas units, alpha ≤ 0.000043); 90 draws, 0
  differing; pool 4 → 8; "(x4)"; two messages ending together go frames
  547 and 548 (the game's loop).
- Client drowning, `sn-client --resolution 1920 1080 --start 0 -3 0
  --look 20 -3 20 --look-down 60 --hold-forward 3 --shot 61.5
  m9u7b-youdied` (`out/m9u7b/drown.log`): `LoseItems: nothing to drop` →
  "You died." added at controls back; 0 WARN/ERROR/panic;
  `out/m9u7b-youdied.png` shows "You died." at the top left (140, 140 in).
  The same with `--shot 68 m9u7b-gone` (`out/m9u7b/drown-long.log`):
  shown 63.754 s, gone 69.764 s, 6.0106 s (ok: rule 6 s + under a
  frame); 0 warnings.
- No regression: `ui --instantiate` 10 of 10; `--death` 24 of 24;
  `--reticle` 22 of 22; `--compass` 644 / 0; `--depth` 91,230 / 0;
  `--bars` 300 / 0; `--start` 21 of 21; `--tmp-sprites` 371 of 371;
  `--draw` 12 of 12 hashes equal; `--live` start hash
  `efa1606eb7c79aff`. Real-data tests (`sn-assets`): 54 passed.
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 678 passed, 0 failed,
  59 ignored (new: 6 messages tests). fmt and clippy (`-D warnings`)
  clean.

**Not tested:** "Inventory full" in the client (filling the inventory in
a scripted run is not set up; the rule is covered by the `hand::aim`
test and `ui --messages`); "You died and lost some belongings." in the
client (the drowning carried nothing); a resize while a message is
shown (the layout is read again on the next frame that needs it); a UI
scale other than 1. Not compared against the original game on screen.

## 2026-10-17 — M9u7a: UI objects made at run time

**Plan:** M9u7 was split in `docs/DESIGN.md` into a (objects made at run
time), b (`ErrorMessage`), c (picked-up icons) and d (notifications and
the unlock pop-up). Research in `docs/formats/ui.md` § Messages.

**What/why:** the game's message screens instantiate templates, pool
them and reorder siblings at run time; the scene could only draw the
stored tree.
- `sn-ui::layout::Tree::graft` (a copy appended under a parent as its
  last child, no index moves) and `set_as_last_sibling`.
- `sn-assets::UiScene::graft` (the template's tree, layout, draw parts,
  graphics, texts and sprites offset and merged; the root named
  `<name>(Clone)`), `set_as_last_sibling`, `node_count`; `UiText` is
  `Clone`.
- `sn-inspect ui --instantiate` (new).
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --instantiate`: 10 of 10 (see `docs/formats/ui.md` § Objects made
  at run time): the copy's vertices equal the original's moved by the
  offset (largest error 0.000000), sibling order and `SetAsLastSibling`
  as expected, the 55 stored draws unchanged.
- No regression: `ui --draw` 12 of 12 hashes equal; `ui --live` start
  hash `efa1606eb7c79aff`; `--start` 21 of 21; `--bars` 300 / 0;
  `--depth` 91,230 / 0; `--compass` 644 / 0; `--reticle` 22 of 22;
  `--death` 24 of 24; `--tmp-sprites` 371 of 371.
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 672 passed, 0 failed,
  59 ignored (new: 3 layout tests). fmt and clippy (`-D warnings`)
  clean.

**Not tested:** objects with no stored template (`new GameObject` with
components made by code), which `uGUI_ItemIcon` needs: moved to M9u7c,
where its children are built. A template with its own canvas (none of
the message templates has one).

## 2026-10-17 — M9u6e2: the game's UI by default; our HUD removed (closes M9u6)

**What/why:** the game's own HUD now covers everything our M9c HUD
showed, so it is the default and ours is gone.
- `sn-client`: `--game-ui` is the default (still accepted);
  `--no-game-ui` turns it off, and then no HUD is drawn. `hud.rs`
  removed (bars, depth, hand text, fade); its `Hud` and `HandHover`
  moved to `player.rs` as the player's feed to the game UI, without
  `seconds_left` and `dead` (only our HUD read them). `GameUiPlugin`
  creates the feed itself, so a fly-camera run (no player) works. The
  baseline report records `game_ui=`.
- README, the help text, `docs/DESIGN.md` (M9u6, M9u6e, M9u6e2 done).
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `grep "hud::\|crate::hud"` over `apps/`: none in the client (the
  remaining hits are `sn_ui::hud` and `sn_assets`' own `hud` module).
- Default run `sn-client --resolution 1920 1080 --shot 8
  m9u6e2-default`: 98 `game UI` log lines, 0 WARN/ERROR/panic;
  `out/m9u6e2-default.png` shows the game's bars, depth and reticle.
- `--no-game-ui --shot 8 m9u6e2-nogameui`: 0 `game UI` lines, 0
  warnings.
- `--free-cam --resolution 1280 720 --benchmark 120`: the game UI read,
  0 warnings, no panic (overwrote `out/client-benchmark.png`).
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 669 passed, 0 failed,
  59 ignored. fmt and clippy (`-D warnings`) clean.

**Not tested:** `--baseline` (only the report line was added); a
`--join` client with the default UI.

## 2026-10-17 — M9u6e1: death, respawn and the out-of-oxygen overlay

**Plan:** M9u6e was split in `docs/DESIGN.md` into e1 (the three screens'
rules) and e2 (`--game-ui` on by default, `hud.rs` removed). Research in
`docs/formats/ui.md` § Death, respawn and overlays.

**What/why:** under `--game-ui` the out-of-oxygen darkening, the death
fade and the respawn screen are now the game's own screens driven by
their scripts' rules; our M9c black fade no longer runs there.
- `sn-ui::death` (new): `uGUI_PlayerDeath` (`TriggerDeathVignette`, its
  `Invoke`s, `Update`'s lerps, `ResetOverlay`), `uGUI_Fader` (`FadeOut`
  uses `fadeInTime`, as the code does), `uGUI_SceneRespawning` (`Show`,
  `Hide`), `uGUI_Overlays` (`Set` through the curve). A frame runs the
  `Update`s, then the due `Invoke`s, then the frame's calls (Unity's
  order, *hypothesis*).
- `sn-assets::death_config` (new): the nodes, the fader's fields, the
  overlay's curve, `Loading`; `vignette_times` reads the delays, rates
  and alpha limits from the IL. `curve_field` made `pub(crate)`.
- `sn-dotnet`: `invokes` (`MonoBehaviour.Invoke` calls with constant
  arguments), `multipliers_f32` (every `ldc.r4; mul`; `multiplier_f32`
  now uses it), `field_compares_f32` (`ldfld f; ldc.r4 v`); `LDFLD`,
  `Il::ldfld` for tests.
- `sn-client`: the player's frame events (`Died` → `OnKill`,
  `MoveToRespawn` → `Show`, `Restored` → `Hide`) and the suffocation
  value go to the UI worker; `body::death_events` reads the local
  player animator's `DeathFade` / `DeathCut` after the animators run;
  the worker runs the rules and logs each step of the timeline against
  the rules' times and the overlay each second. `send_inputs` (with
  `request_build`, `receive_build`) moved to `PostUpdate` so a frame's
  inputs include that frame's animator events. Under `--game-ui`,
  `hud::update` no longer runs (our bars were already hidden; now our
  fade too) and `hud::hide_bars` is gone.
- `sn-inspect ui --death` (new).
No new dependencies.

**Findings:**
- `player_death` plays `quick_death`, whose `DeathFade` is at 0 s: the
  vignette starts on the death's next frame, not after a 1.7 s clip as
  first assumed for `ui --death`'s script (that run keeps `drown_death`'s
  1.675 s, the timing being scripted there).
- `uGUI_Fader.FadeOut()` uses `fadeInTime` (1 s) and `FadeIn()` uses
  `fadeOutTime` (0.5 s).
- With the game's numbers and `quick_death`, the vignette starts fading
  out (6 s after the event) just before the respawn screen's `Hide`, so
  both fade together: in the client run the vignette was at 0.95 at
  `Hide`, 0.12 when the respawn screen was clear, and reset 0.9 s later.
- In our ported draw order (M9u4d) the respawn screen draws under the
  black vignette and the out-of-oxygen overlay (`ui --death`: draw items
  45, 50 and 56 of 57). After a drowning both are at 1 while the respawn
  screen shows, so "Loading..." is covered and the screen stays black;
  the screen is only seen through them as they fade. Kept as the data
  gives it; not compared with the game on screen.

**Dead end:** the first screenshot runs used the UI log's times (from
the app's start) for `--shot`, which counts play time from the player's
start (about 5 s later): "black" showed the respawned player and
"respawning" the second dive. Re-shot at play times.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --death`: 24 of 24 checks; 9,840 frame checks over 1,800 frames
  of a scripted drowning (our vitals) equal to the replay; black 0.783 s
  after the event, fade out 6.000 s after it, fader clear 1.000 s after
  `Hide`; 5 frames drawn, each screen drawn exactly while its alpha is
  above 0 (the respawn screen: 2 draws); `CutToBlack` black at once; a
  death by damage shows the overlay for one frame. Values read: Invoke
  0 / 6 s, rates 5 / 2, limits 0.98 / 0.02, fader 1 / 0.5 s unscaled,
  "Loading...", curve (0, 0) (0.8, 1) (1, 1).
- Client drowning, `sn-client --game-ui --resolution 1920 1080 --start
  0 -3 0 --look 20 -3 20 --look-down 60 --hold-forward 3 --shot 80
  m9u6e1-respawned` (`out/m9u6e1/drown.log`, UI times from the app's
  start): suffocating from 49 s; the overlay each second equals the
  curve at the logged value, 85 of 85 seconds (checked by a script of
  our own against the keys); `OnKill` 57.064 s; `DeathFade` 57.075 s
  (layer 8, weight 0.105); black 57.876 s; `Show` 62.088 s; `BeginFadeOut`
  6.006 s after the event (ok); `Hide` 63.103 s; fader clear 1.014 s
  after it (ok); `ResetOverlay` 65.025 s; 0 WARN/ERROR/panic.
- Screenshots (play times; the death is at 53.0 s of play in every
  run): `out/m9u6e1-dying.png` (47 s: dimmed, overlay 0.48, O2 0),
  `out/m9u6e1-black.png` (55 s: black), `out/m9u6e1-respawning.png`
  (58.5 s: respawn fader at 1, black: covered, see Findings),
  `out/m9u6e1-respawned.png` (61 s: clear, O2 45, the HUD back).
- No regression: `ui --draw` 12 of 12 hashes equal; `ui --live` 0
  failures (start hash `efa1606eb7c79aff`); `--start` 21 of 21;
  `--bars` 300 / 0; `--pulse` 13,440 / 0 and 300 / 0; `--depth`
  91,230 / 0; `--compass` 644 / 0; `--tmp-sprites` 371 of 371;
  `--reticle` 22 of 22.
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): 669 passed, 0 failed,
  59 ignored (new: 5 death rules, 1 IL reader). `cargo test --release -p
  sn-assets --test real_data -- --ignored`: 53 of 53 (new:
  `the_death_screens_are_read`). fmt and clippy (`-D warnings`) clean.

**Not tested / not 1:1:** the other death triggers (fire, explosion:
no such damage yet) and `CutToBlack` in the client (covered by `ui
--death`); permadeath (`EndGame`); `UnstuckPlayer` and the ending's
vignette; Unity's frame order and an event at a clip's time 0 firing on
the state's first update are *hypotheses*. Not compared with the game on
screen.

## 2026-10-14 — M9u6d2: the hand reticle's rules

**What/why:** under `--game-ui` the hand's hover is now the game's own
`HandReticle`: the icon (Default dot, Hand, PackUp) with its 0.1 s fade,
the texts with the mouse-button sprite, hidden while the PDA is open.
Our stand-in hand text no longer shows under `--game-ui`.
- `sn-ui::reticle` (new): `LateUpdate`, `SetIconInternal`, the icons'
  `Sequence` fades, the scale lerp, the progress, `GetText`'s format.
- `sn-assets`: `reticle_config` (nodes, icons, fields, formats);
  `format_button` (`GameInput.FormatButton` for the keyboard's default
  binding, every step read from `Assembly-CSharp.dll` and
  `Unity.InputSystem.dll`); `hand::aim` gives the icon and button each
  `OnHandHover` sets; `PickupInfo::pack_up_icon` (`usePackUpIcon`; the
  rules fingerprint's value changes with it).
- Fix in `ui_start`: the hidden reticle icons are now inactive, as
  `Awake` leaves them (it looked for the unstored field `go`).
- `sn-dotnet`: `static_collection`, `const_string`,
  `Assembly::attribute_named_args` (`AttrValue`); `Builder::
  attribute_with`, `constant_string` for tests.
- `sn-client`: the hover (with hatches: `CinematicModeTrigger`'s text,
  `Hand` icon) goes to the UI worker, which runs the reticle and logs
  each icon change, each fade's length against 0.1 s, and the texts each
  second; `hud::update_hand` runs only without `--game-ui`.
- `sn-inspect ui --reticle` (new).
No new dependencies.

**Dead ends:**
1. The first client run showed the icon `None` throughout: I took the
   window's mouse grab as the game's cursor lock, and a scripted run
   never grabs it. The game locks the cursor during play and frees it for
   the PDA, so the rule now uses the PDA alone.
2. The first fade log left out the change frame's own step (0.076–0.174 s
   on uneven frames); it now counts from the frame's start and checks
   "0.1 s plus less than one frame".

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --reticle`: 22 of 22 checks. 10 icons, one per type; FormatButton
  `<Mouse>/leftButton` → "Left Button" → `MouseButtonLeft` → a sprite
  tag; 6,480 frame checks over 240 frames (pickup, full inventory,
  pack-up, cursor freed, outcrop, nothing) equal to the replay; 5 fades
  of 0.1000 s; drawn: the sprite iff the text has the button, a square
  45.864 px quad in (173, 248, 255, 255); the icons drawn equal the
  replay's.
- Client `sn-client --game-ui --resolution 1920 1080 --start -283.5
  -63.3 -114.4 --use-item 3,4,5,7,8 --shot 7 m9u6d2-outcrop`: "Break
  limestone outcrop (sprite)" with Default → Hand, fade 0.1107 s (ok,
  last step 0.0238 s); "Pick up Copper ore (sprite)"; Hand → Default
  after the pickup; 0 WARN/ERROR/panic. `out/m9u6d2-outcrop.png` shows
  the hand icon and "Pick up Copper ore (mouse icon)".
- Client in the pod, `--use-item 3 --panel 8=open,10=close --shot 14
  m9u6d2-locker`: "Climb Ladder (sprite)" (hatch), "Open storage
  container (sprite)", Hand → None while the panel is open (at once),
  None → Hand after it closes; every fade ok; 0 warnings.
  `out/m9u6d2-locker.png`.
- **Not tested in the client:** a full inventory (covered by `ui
  --reticle` and the `hand::aim` test), a pack-up item in the world.
- No regression: `ui --draw` 12 of 12 hashes equal; `ui --live` 0
  failures (start hash now `efa1606eb7c79aff`, from the start-state fix);
  `--start` 21 of 21; `--compass` 644 / 0; `--depth` 91,230 / 0;
  `--bars` 300 / 0; `--tmp-sprites` 371 of 371.
- `cargo test --workspace`: 663 passed, 0 failed, 58 ignored (new: 6
  reticle, 1 attribute, 1 collection reader; hand test extended).
  `cargo test --release -p sn-assets --test real_data -- --ignored`: 52
  of 52. fmt and clippy clean.

## 2026-10-14 — M9u6d1: TMP inline sprites

**What/why:** the hand reticle's texts (M9u6d2) end in the key's icon,
a TMP `<sprite name="…">` tag; TMP texts can now lay out and draw
sprites. Facts in `docs/formats/ui.md` § TMP inline sprites.
- `sn-ui::text::sprite` (new): sprite assets (face, glyph and character
  tables), `simple_hash`, `SearchForSpriteByHashCode`, the tag's
  resolution, the sprite's scale and lines, `find_matching_material`
  (the game's `FindMatchingMaterial`, read from the IL).
- `sn-ui::text`: the `<sprite>` tag parsed (`name`, `index`, `tint`,
  `color`, `anim` rejected); a found sprite is one character in
  `prepare`, the layout and the preferred size (bold cleared, no
  padding, always visible, U+E000 + index); `text_meshes_with` puts its
  quad in a sub-mesh per sprite asset (UVs from the glyph rect, TMP's
  colour rule with `Color32.Multiply` as the IL computes it).
- `sn-assets`: sprite assets' tables, sheet and materials read; the
  runtime default sprite asset is `InputSymbols` (as `LanguageSDF` sets
  it), the stored `EmojiOne` replaced; the UI scene draws sprite
  sub-meshes with the matching material and the sheet as a texture.
- `sn-client`: the SDF text shader reads `.a` from an install texture
  (flag 4; `InputSymbols`' sheet is `Alpha8`) and `.r` from our font
  atlases.
- `sn-inspect ui --tmp-sprites` (new).
No new dependencies.

**Dead end / correction:** the plan expected a `TextMeshPro/Sprite`
port. `InputSymbols`' three materials are all `TextMeshPro/Distance
Field` (the symbols are a distance field), so none was needed.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --tmp-sprites`: 371 of 371 checks. 180 of 180 stored hashes equal
  ours, every character has its glyph, 164 of 164 `InputSymbols` names
  resolve. `<sprite name="MouseButtonLeft" color=#ADF8FFFF>` at 24 px in
  `Aller_Rg SDF`: scale 0.917280 (formula in `f64` 0.917280), quad
  44.0294 × 44.0294 (formula equal), UVs at the glyph rect's corners
  (735, 147, 48, 48 in 1024 × 512), colour (173, 248, 255, 255). Text
  materials named `… Outline` / `… Shadow` get `InputSymbols Outline` /
  `Shadow`, others its `material`.
- No regression: `ui --draw` 12 of 12 hashes equal; `ui --live` 0
  failures, start hash `7f52d63da4ad1e96` unchanged; `ui --compass` 644
  / 0 differ; `ui --depth` 91,230 / 0 differ.
- Client: `sn-client --game-ui --resolution 1920 1080 --shot 12
  m9u6d1-client`: exit 0, 0 WARN, ERROR or panic lines (the shader
  change compiles). No in-game text holds a sprite yet, so a sprite was
  **not seen drawn** in the client (that comes with M9u6d2).
- `cargo test --workspace`: 656 passed, 0 failed, 58 ignored (new: 3
  sprite layout and mesh tests, 1 tag test, 1 material-match test, 2
  hash and search tests). `cargo test --release -p sn-assets --test
  real_data -- --ignored`: 52 of 52. fmt and clippy clean.

## 2026-10-14 — M9u6c2: the compass

**What/why:** the HUD's compass (`uGUI_Compass`) was never drawn; it now
shows while a compass is equipped, turns with the camera, and its
direction labels move and fade as the game's script places them.
- `sn-ui::compass` (new): `OnPopulateMesh`'s projected quad (UV ÷ z,
  1/z), `UpdateAngle`'s material values, `UpdateLabels`' place and
  alpha, with Unity's `Quaternion.Euler`, `TRS`, `Perspective` and
  `MultiplyPoint3x4`.
- `sn-ui::depth`: `UpdateCompass` sets the direction while enabled
  (only on a change); new `HudInputs::yaw`. Fix: the visibility is now
  written every frame (see Dead ends).
- `sn-assets`: `UiGraphicKind::Compass` (fields, labels resolved to
  nodes, `texture`), drawn by the scene with its own mesh and material;
  `DepthConfig::ring`. Test aids: `ItemsState::add_supply`,
  `item_rules_with` / `rules_with` (extra pickup prefabs read as the
  supplies' are).
- `sn-session`: `Server::add_test_supplies` (a new world's locker,
  before anyone joins; the tick-0 hash redone).
- `sn-client`: `uGUI/Compass` as flag 16 of `game_ui.wgsl` (from its
  compiled programs; `_Angle`, `_AlphaFrom`, `_AlphaTo` in the
  `_MainTex_ST` slot, no new pipeline); the camera's yaw (`euler_y`) and
  "a compass in the equipment" sent each frame and logged each second.
  Debug flags: `--test-supply TECH[,…]` (logged; on a join it only
  extends the rules so the fingerprints match), `--turn DEG_PER_S`,
  `--panel …=unequip`.
- `sn-inspect ui --compass` (new); `ui --depth` now expects the ring
  drawn with its labels.
No new dependencies.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --compass`: 644 of 644 checks. The 8 stored label places equal
  ours at the stored `debugDirection` 0.646 within 0.000051 px (and the
  `f64` replay's within 0.000052). 36 headings: vertices within 1.7e-5,
  label places within 5.3e-5 px, label alphas exact. The equip timeline
  draws the ring and 8 labels iff equipped and the PDA is closed.
- `ui --depth` 91,230 / 0 differ; `--bars` 300 / 0; `--pulse` 13,440
  and 300 / 0; `--start` 21 of 21 named; `--live` 0 failures, start
  hash `7f52d63da4ad1e96` unchanged; `--draw` 12 of 12 hashes equal;
  `--meshes` 0 fail.
- Client: `sn-client --game-ui --resolution 1920 1080 --test-supply
  Compass --use-item 3 --panel 6=take-all,7=close,8=open,9=equip,
  10=close,20=open,21=unequip,22=close --turn 30 --shot 26
  m9u6c2-unequipped`. The compass went into the locker, was taken,
  equipped ("equip") and later unequipped ("unequip"). It was shown in
  every logged second while equipped with the PDA closed, and hidden
  while the PDA was open and after the unequip. Direction = yaw ÷ 360 in
  10 of 10 shown seconds. 71 of 71 draws, 0.14 ms CPU per frame, worker
  3.2 ms per frame, 0 WARN, ERROR or panic lines. Screenshot run
  (`--panel 6=take-all,7=close,8=open,9=equip,10=close --turn 30 --shot
  18 m9u6c2-compass`): `out/m9u6c2-compass.png` shows the ring with SE
  and S at a heading near 160°.
- `cargo test --workspace`: 649 passed, 0 failed, 58 ignored (new: 5
  compass, 1 depth, 1 yaw, 1 server test). `cargo test --release -p
  sn-assets --test real_data -- --ignored`: 52 of 52 (the depth test
  now also checks the labels' stored places). fmt and clippy clean.

**Dead ends:**
- The first `ui --compass` run never drew the ring when the compass was
  on from the first frame. The start state (M9u6s) already applies the
  first `SetVisible(false)`, but the rule started from the script's
  `_visible = true`, so it saw no change. The rule now always states the
  visibility.
- The first client run's test supply was refused: our item rules only
  know pickups placed in the world and the supplies, and no compass is
  placed. The rules now read the test supplies' prefabs too.

**Not tested / not 1:1:** no compass can be had in normal play yet
(crafting is M9e). The camera drone's compass is not ported. The ring's
back-face culling is not done (it faces the camera). `sn-server` has no
`--test-supply`. Not compared with the game on screen.

## 2026-10-14 — M9u6c1: the depth readout's rules

**Plan:** M9u6c was split in `docs/DESIGN.md` into c1 (`uGUI_DepthCompass`,
the depth) and c2 (`uGUI_Compass`: its shader, mesh, direction and
labels, and a compass in the equipment). Research in `docs/formats/ui.md`
§ The depth and compass.

**What/why:** the HUD's depth showed its stored state; it now follows
`uGUI_DepthCompass.LateUpdate` each frame.
- `sn-ui::depth` (new): `Initialize` (meter suffix, label texts, the
  shadow's sprite and multiplicative blend, the texts' colour),
  `UpdateDepth` (mode, the graphics shown per mode, the text when
  `floor(depth)` changes), `UpdateCompass`'s visibility part (half moon
  sprite, `SetVisible` on the compass and its labels).
- `sn-ui::hud::HudInputs`: new `depth` and `compass` inputs.
- `sn-assets::depth_config` reads the script's nodes, sprites, colour
  and the language strings (`fields_at` made `pub(crate)`).
- `sn-client`: sends the depth (`sn_sim::vitals::depth_of`, as our HUD
  did); `compass` is always false (no compass can be equipped yet);
  logs the depth each second.
- `sn-inspect ui --depth` (new): a scripted timeline checked every frame
  against a replay.
No new dependencies.

**Findings:**
- `OnDepthClassChanged` replaces its argument with `Safe` (IL: `ldc.i4.1;
  stloc.0`), so the danger sprites and colour never show. Not read.
- The first `ui --depth` run failed 570 frames: the compass labels drew
  while the compass flag was on. The script does enable them
  (`SetVisible`); the check was wrong, not the rule. Their places and
  alphas are M9u6c2.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --depth`: 91,230 of 91,230 checks over 4,200 frames, 0 differ;
  draw mean 1.93 ms.
- `ui --bars` 300 / 0 differ, `ui --pulse` 13,440 and 300 / 0 differ,
  `ui --start` 21 of 21 named, `ui --live` 0 failures.
- Client: `sn-client --game-ui --resolution 1920 1080 --start 0 -3 0
  --look 20 -3 20 --look-down 60 --hold-forward 3 --panel
  12=open,15=close --shot 30 m9u6c1-dive`. In 29 of 29 shown seconds the
  depth text equals `floor(depth)` (3 → 19 m); `None` with the text kept
  while the PDA is open (3 s). 62 of 62 draws, 0.13 ms CPU per frame,
  0 WARN, ERROR or panic lines. `out/m9u6c1-dive.png` shows "19 m".
  A first run at `--start -950 -616 1018` stayed at 616 m and a second
  without `--look-down` stayed at 3 m (the player swam level), so
  neither tested the floor.
- `cargo test --workspace`: 641 passed, 0 failed, 58 ignored (4 new unit
  tests). `cargo test --release -p sn-assets --test real_data --
  --ignored`: 52 of 52 (new: `the_depth_compass_is_read`). fmt and
  clippy (`-D warnings`) clean.

**Not tested / not 1:1:** vehicles, the camera drone, the intro and the
rocket (the other modes) are not ported. The compass is M9u6c2. Not
compared with the game on screen. Observed, not investigated: after the
PDA closed in the client run, the held W no longer moved the player
(depth fixed at 19.359 m on the sea floor).

## 2026-10-14 — M8c7b1: atmosphere volumes select object skies

**What/why:** the user authorized the next visual work and reiterated that
commits/pushes require explicit permission. Implemented the first proposed
slice: static atmosphere-volume selection for object lighting. Previously
Auto SkyAppliers used only the batch/2D-map biome at the prefab root, so
cave and interior sky overrides were ignored.

- `sn-assets`: read the installed AtmosphereVolume layout through the existing
  checked script reader; retain active visual volumes with enabled colliders.
  Preserve the full prefab affine chain for box/sphere/capsule placement;
  reject unsupported, sheared-box, collapsed and nonfinite geometry explicitly.
- `sn-sim`: headless inclusive primitive point queries and priority selection.
  Native instance IDs are unavailable: use an entity lifetime tie key and log
  equal-priority overlaps with different biome overrides.
- Client: stream volume metadata with its instance, including invisible
  volume-only prefabs; removing the instance removes its volumes. Auto mesh
  and effect bindings reselect when volumes load/unload or new bindings appear.
  Materials remain cached per sky so outside objects keep their own lighting.
- Resolve the query position from the owning SkyApplier component instead of
  the prefab root, preserving transform order. Placeholder spawns carry that
  position and their volume transforms into the parent placement.
- Updated DESIGN, README and lighting/water format notes. No new dependencies,
  architecture change or whitelist changes. Game install read-only.

**Verified:**
- `cargo test --workspace`: 637 passed, 0 failed, 57 ignored. New synthetic
  coverage: primitive boundaries, invalid points, priorities/ties, empty
  overrides, removal fallback, collider scaling/reflection, rejected shear,
  owner-position transform order and an ECS load/unload regression. The ECS
  regression checks existing meshes/effects, late bindings, outside-material
  isolation and restoration to the cached fallback after the volume despawns.
- `$env:SUBNAUTICA_DIR='<INSTALL>'; cargo test -p sn-assets --test real_data atmosphere_all_placed_prefabs_and_colliders -- --ignored --nocapture`:
  passed. All 1,369 placed prefab families checked; 151 visual-volume families,
  2,409 supported placements (1,425 boxes, 984 spheres, no capsules), zero
  unsupported placement transforms. Collider centers are enclosed and far-away
  probes excluded. Fixtures and numerical expectations contain no game assets.
- `$env:SUBNAUTICA_DIR='<INSTALL>'; cargo test -p sn-assets --test real_data optimized_key_terminals_bind_skin_and_select_distinct_materials -- --ignored --nocapture`:
  passed, including atmosphere reads after hidden terminal bones are rebuilt.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `git diff --check`: pass.
- `cargo build --release -p sn-client`: pass; rebuilt executable available.
- Three final GPU runs each outdoors and in the thermal room: 960×540,
  400 m view, 60 settled warm-up frames, 120 measured frames, fixed camera,
  seeds and game clock, High water, grading off. All six exit 0, zero loader
  warnings, zero shader validation errors, zero unsettled measurement frames.
  Camera, terrain, grass, mesh/texture counts, fog and light counts match
  their before cases. Metadata entity counts increase: Shallows 17,063→17,194;
  interior 6,111→6,177. Some volume-only prefab definitions are now retained.
- Shallows: 132 active volumes, 461 enclosed unique query positions, zero
  different-biome tie ambiguities; cave sky bindings appear. Interior:
  67 active volumes, 410 enclosed positions, 18 tie ambiguities; Precursor
  interior sky bindings appear. The log labels these unique positions "roots";
  they are positions of the owning appliers, not necessarily prefab roots.
- Final outdoor mean times: 11.132 / 11.480 / 10.215 ms; medians
  10.464 / 10.816 / 10.108 ms; p99 17.382 / 16.040 / 12.659 ms.
  Before means: 11.289 / 30.175 / 11.566 ms; p99 17.599 / 65.492 / 14.208 ms.
  Before outdoor run 2 overlapped compilation and is contaminated. Interior
  final means: 7.383 / 6.958 / 6.908 ms; p99 8.522 / 8.473 / 8.047 ms;
  before means 7.432 / 8.014 / 7.629 ms; p99 8.745 / 12.952 / 9.677 ms.
  These short runs record costs, not a controlled speedup or proof of the
  full-resolution/full-view 16.7 ms target. Waves/animation remain live.
- Before executable was the preexisting release binary; its source revision
  was not independently rebuilt. Its SHA256 and the starting HEAD are recorded
  in ignored `out/atmosphere/provenance.txt`, alongside the final binary hash.
  Logs and summary: `out/atmosphere/`. Final captures/reports:
  `out/baselines/run-atmosphere-final-{shallows,interior}-{1,2,3}/`.

**Dead end/correction:** the first GPU implementation introduced three terminal
loader warnings by treating reconstructed hidden avatar bones as serialized
GameObjects. These use our own `#avatar:` identities. The reader now skips
their nonexistent components while retaining their transforms. The installed
terminal regression and all final GPU runs pass with zero loader warnings.
Earlier `run-atmosphere-after-*` captures are preliminary, before this correction;
use `run-atmosphere-final-*` for the final result.

**Exact verification command** (after setting SUBNAUTICA_DIR, choose a fresh NAME):
```powershell
target/release/sn-client.exe --benchmark 120 --baseline NAME --baseline-revision working-tree-atmosphere-final --warmup 60 --resolution 960 540 --start -41 -1223 112 --look -41 -1223 132 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```
Expect a saved report/120 CSV samples/capture, zero loader warnings and settled
loading; `atmosphere:` logs include the interior override, 67 active volumes
and 18 different-biome equal-priority ambiguities. For interactive review,
replace `--benchmark 120 --baseline NAME --baseline-revision working-tree-atmosphere-final --warmup 60`
with `--free-cam`. Outdoor case: replace start/look with
`--start 0 -10 0 --look 60 -40 60`; expect 132 active volumes and cave overrides.

**Not tested:** matched original-game visuals, native PhysX scaling/boundary
measurements, exact native runtime-ID tie ordering, moving appliers/volumes and
script-driven activation. Fog rasterization, AtmosphereDirector fades, fixed/
custom anchors, cube reflections and power state remain later slices. This
completes M8c7b1's implementation checks, not full M8c7b or 1:1 acceptance.
Only our source/docs appear in git status. Implementation was completed
without commits or pushes; the user subsequently authorized committing and
pushing this change on 2026-10-14.

## 2026-10-14 - Lost River lake footprints under nonuniform scale

**What/why:** after authorizing commit/push of the waterfall-cap correction,
the user supplied a client screenshot and reported extra/missing green
surfaces at several locations (exact coordinates unknown). The prior fix
was committed/pushed as `536db41`; the user confirmed the new lake fix looks good and authorized its commit/push.

The installed lake audit found a concrete transform-order error: flattening
parent and child TRS by multiplying their scale components applies a
nonuniform parent scale along the child's rotated axes. Sixteen of 328
placed lake mesh surfaces were distorted; worst vertex displacement
17.380856 m. Border-fading planes with rotated children are affected.

**Change:** rigid effects spawn under one placement parent per instance,
with their local transform retained. Bevy propagates the full affine
product. Removing the tracked parent removes its effect children. Animated
effects keep their existing rig attachment; waterfall cap height correction
is applied to the local child before parenting. No core TRS change,
dependencies, shader changes or whitelist changes.

**Verified:**
- `cargo test --workspace`: 631 passed, 0 failed, 56 ignored. New synthetic
  propagation test: rotated/nonuniform footprints, reflected shear, dynamic
  parent update, parent/child removal.
- `$env:SUBNAUTICA_DIR='<INSTALL>'; cargo test -p sn-client audit_brine -- --ignored --nocapture`:
  1 passed. All 328 placed lake surfaces, 16 old errors over 1 cm,
  old maximum 17.380856 m. Correct affine product's maximum error against
  sequential point transformation: 0.00012207031 m. The test lives in the
  client so it checks Bevy's actual GlobalTransform math.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `git diff --check`: pass.
- `cargo build --release -p sn-client`: pass; rebuilt client available.
- GPU checks: green worst-case area before/after and a blue waterfall
  regression, 120 measured frames/60 settled warm-up, 960x540, view 400.
  All processes exit 0; zero loader warnings or shader validation errors.
  Green effect parts/meshes stay 162/7; terrain triangles 3,097,194, object
  meshes/materials/textures 3,551/395/676 unchanged. Blue effects 60/5;
  terrain triangles 2,093,114, meshes/materials/textures 1,196/233/448.
  Tracked entity counts fall (green 2,697 to 2,603, blue 606 to 574) because one
  tracked placement parent now owns several effect children. Rendered
  effect-part counts are unchanged. Green mean frame times 6.72 to 7.49 ms,
  p99 7.68 to 13.93 ms; blue regression mean 9.01 ms. Short single runs do not
  isolate performance effects from scheduler/animation variation.
- Evidence/logs: `out/waterfall-fix/lake-*.log`; baseline reports/captures
  `out/baselines/run-lake-footprint-{before,after}/` and
  `out/baselines/run-lake-blue-regression/` (all ignored).

**Exact human verification command** (after setting SUBNAUTICA_DIR):
```powershell
target/release/sn-client.exe --free-cam --start -525 -810 215 --look -525 -824 155 --view 400 --resolution 960 540 --time-scale 0 --slot-seed 1 --lifepod-seed 1 --aurora intact
```
Expect the green border-fading planes to retain their authored footprints;
for automated output, replace `--free-cam` with `--benchmark 120 --warmup 60`.
Expect a completed benchmark and zero loader warnings.

**Dead ends/corrections:** the first diagnostic scanned only baked cell
placements and found no lakes. These are batch objects; the audit was
corrected to read BatchObjectsCache and apply the same batch-root relocation
as the client. No speculative individual surface offsets were introduced.

**Human verification (2026-10-14):** user reports "looks good to me" and
authorizes commit/push. This confirms the client result in their review.

**Not tested:** matched reproduction of the user's screenshot (coordinates
unknown) and original-game visual parity.
This fixes a proven source of extra/missing area, not a claim that all green
surface issues are resolved. Other non-effect/nested TRS flattening remains
outside this scoped change. Game install read-only; only our source/docs
appear in git status.

## 2026-10-14 ? Lost River green/blue waterfall cap height correction

**What/why:** the user reports waterfall starting points extending too high.
Read-only research found an unported waterfall startup rule: top/bottom
local Y scale equals configured height divided by root local Y scale.
The previous shared template stretched these sections with the middle.
A Tree Cove placement with Y scale 11.9215 therefore extended its top
approximately 38.15 m above the join instead of 3.2 m.

- `sn-unity`: checked waterfall component reader, both heights and three
  Transform references, synthetic truncation coverage.
- `sn-assets`: resolve configured caps to their nodes; expose per-placement
  scaling without changing shared geometry. Synthetic positive, reflected,
  collapsed and middle/position invariants. Unexpected nested layouts fail
  explicitly; all three installed families use direct root children.
- Client: apply the rule on effect and ordinary mesh paths, and when a
  placeholder instantiates a waterfall. Log cap scale invariants.
- Inspector: print configured heights and apply the rule to prefab exports
  in ignored `out/`. Format findings and README updated.

No dependencies, architecture changes or whitelist changes. Game install
read-only; no commits or pushes.

**Verified:**
- `cargo test --workspace`: 630 passed, 0 failed, 55 ignored; no game needed.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `git diff --check`: pass.
- `cargo build --release -p sn-client -p sn-inspect`: pass.
- `$env:SUBNAUTICA_DIR='<INSTALL>'; cargo test -p sn-assets --test real_data waterfall_caps -- --ignored --nocapture`:
  1 passed. Three families, six resolved caps; top/bottom configured heights
  1; thickness constant at placement Y scales 0.25, 1 and 12.
- Matched before/after blue and green GPU runs: 120 measured frames after
  60 settled warm-up frames, 960?540, view 400, fixed clock and seeds.
  After runs exit 0; zero loader warnings or shader validation errors.
  Object and terrain stats equal before/after. Blue: 8 corrected caps,
  root Y 1.886?11.922; green: 42 caps, root Y 0.221?4.66. All resulting cap
  world Y scales equal 1 within float rounding (worst 0.99999994).
  Mean frames before/after: blue 10.87/5.38 ms, green 10.48/5.28 ms.
  Build/check workloads overlapped the earlier runs: these pairs do not
  establish a performance improvement.
- Logs: `out/waterfall-fix/`; reports/captures:
  `out/baselines/run-waterfall-{blue,green}-{before,after}/` (all ignored).

**Exact client verification commands** (set SUBNAUTICA_DIR as above):
```powershell
target/release/sn-client.exe --benchmark 120 --warmup 60 --start -1244 -920 430 --look -1244 -930 397 --view 400 --resolution 960 540 --time-scale 0 --slot-seed 1 --lifepod-seed 1 --aurora intact
target/release/sn-client.exe --benchmark 120 --warmup 60 --start -1190 -630 -65 --look -1187 -643 -109 --view 400 --resolution 960 540 --time-scale 0 --slot-seed 1 --lifepod-seed 1 --aurora intact
```
Expect a completed 120-frame benchmark, zero loader warnings and waterfall
cap log lines with configured height/world Y scale approximately 1.
For a human comparison, replace `--benchmark 120 --warmup 60` with
`--free-cam` to stay in each scene.

**Dead ends/corrections:** the first test compile assumed the prefab database
had struct entries; it is a map, so the test now reads its path values.
The automated first log summary missed cap records because PowerShell
wrapped the output; corrected it to accept whitespace across lines.

**Not tested:** original-game visual parity and user confirmation that both
starting heights now look right. Zero-root-Y handling deliberately keeps
geometry collapsed instead of generating invalid render matrices.

## 2026-10-14 — M9u6b2: the bars' animations

**What/why:** the HUD bars' pulse, and the health, food and water
icons' punch and coin turn, now follow the game's scripts and clips.
Until now the pulse halos were drawn at their stored alpha.
- `sn-unity`: `AnimationClip` keeps its editor curves (`editor`:
  rotation, Euler, position, scale and float curves, per component),
  which legacy clips are sampled from. New `LegacyAnimation` reader
  (class 111).
- `sn-ui`:
  - `clip`: Unity's `AnimationCurve` (clamped, Hermite, steps) and a
    clip's scale and `CanvasGroup` alpha tracks sampled into a `UiState`;
  - `tween`: `CoroutineTween`, `MathExtensions.CoinRotation`, `Spring`
    and `Oscillation`, and `Mathf.Approximately`;
  - `hud`: each bar's `pulseTween`, its clip sampled each frame, the
    punch, and the coin turn, in Unity's frame order. New inputs: PDA
    open, healed, ate, drank. `UnityEngine.Random` is replaced by our
    own generator, used in the same ranges.
- `sn-assets::hud_config`: reads the pulse curves, the bar's
  `Animation` and its clip, with tracks resolved to nodes by path, and
  the icon's fields and stored scale. A clip animating anything else is
  an error.
- `sn-client`: sends `pda_open` (`inventory::Panel.open`), and logs each
  bar's pulse period and clip time every second.
- `sn-inspect ui --pulse` (new): a scripted timeline checked against a
  replay. `ui --start` now applies the HUD's first frame too, so the
  halos are named by their rule instead of "stored state".

**Findings** (`docs/formats/ui.md` § The bars' animations):
- The health punch fires on healing (`AddHealth`), not on damage as the
  plan said.
- `pulseReferenceCapacity` is stored as 100 on all four bars, so oxygen
  pulses below 15.
- `CoinRotation` steps by whole seconds (`FloorToInt`); ported as
  written.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --pulse`: 13,440 of 13,440 halo alphas equal the replay. The
  measured periods are within one frame of the curves at oxygen 6, 3
  and 1 of 45 and at health 12 (0.64, 0.52, 0.44 and 1.26 s). 300 of 300
  icon widths during punches equal. The turn reaches 180°.
- `ui --bars`: 300 checks, 0 differ.
- `ui --start`: 21 of 21 visible draws named.
- Client dive: the pulse starts below oxygen 15. 62 of 62 draws,
  0 warnings. `out/m9u6b2-dive.png` shows only the oxygen bar glowing.
- `cargo test --workspace`: 628 passed, 0 failed, 54 ignored.
  `cargo test --release --workspace -- --ignored`: 54 of 54 pass with
  the game, including all 2,294 clips still parsing to their last byte.
  fmt and clippy are clean.

**Not done:** the client sends no punch events (no healing, eating or
drinking in our sim yet). The Loop wrap at normalized time 1 is a
hypothesis; it is invisible with these clips. Not compared with the game
on screen.

## 2026-10-14 — M9u6s: the other screens' start rules

**What/why:** the stored `uGUI.prefab` drew screens the game hides at
start (the feedback panel, the "Loading failed" dialog, pop-ups, the
vehicles' and cameras' screens, every reticle icon), and they covered
the HUD in every screenshot. Each script's start was ported from the
decompiled code (read only):
- `sn-assets::ui_start`: `UiStartState` now holds a full `UiState` (the
  new rules also set a colour, renderer alphas and a material value).
  The start rules are in a table (`RULES`: script, fields, effect, the
  method and condition), plus the intro's texts and skip bar, the
  compass and the hand reticle's icons. A rule whose script's node is
  inactive is not applied, and that is logged. The rules assume a new
  game's start: no vehicle, camera, compass or radiation; those systems
  are not ported.
- `sn-inspect ui --start` (new): the start state's draws by screen,
  visible or not, each with the scripts above it. Each visible draw is
  named with the rule that shows it, and the run fails if one isn't.
  Draws over 5 % of their canvas are listed.
- **Client fix (found by the screenshot):** `receive_build` kept only
  the newest build each frame. The worker sends each texture once, so
  textures in a superseded build were lost, and every image drew as a
  flat square. The resize log line was missing as well, because the
  resize build was the one superseded. This started now because fewer
  draws made the worker fast enough to send two builds in one frame. Now
  every received build's textures and resize are kept.
- Unit tests: colour reading and the rule table; the start-state test
  was updated. Opt-in test `the_start_state_hides_what_the_game_hides`.

**Verified** (`SUBNAUTICA_DIR` set):
- `ui --start`: 151 visible draws in 8 screens before; after, 66 draws,
  23 visible, 23 of 23 named. Those are the bars and their background
  (12), the player depth (4), the reticle's `Default` icon (1),
  `DepthClear` (1, not drawn by our pass) and the bars' pulse halos and
  wave (5, stored state until M9u6b2). The only large draw is
  `DepthClear`.
- `ui --live`: every change kind returns to the start hash. The start
  state is now 66 draws, hash `7f52d63da4ad1e96`. The stored-state
  hashes (`c976ea767c630cd5` unplaced, `9ce107c885f3a955` placed) don't
  go through the start rules. M9u6b1 didn't record them, so they are
  not compared. 120 frames: mean 2.0 ms.
- `ui --bars`: 300 bar checks, 0 differ.
- Client dive (`--game-ui --shot 30 m9u6s-dive`): 72 textures on the
  GPU, 62 of 62 draws, 0.12 ms CPU per frame, 0 warnings or errors.
  `out/m9u6s-dive.png` shows the bars with their icons, the depth with
  its half moon and the reticle. It is not compared with the game.
- `cargo test --workspace`: 616 passed, 0 failed, 53 ignored. M9u6b1's
  entry said 51, but that was counted before its new test; its commit
  has 52. `cargo test -p sn-assets --test real_data -- --ignored`: 48 of
  48 pass. fmt and clippy are clean.

**Not done:** the pulse halos and wave (M9u6b2). The rules' game-state
conditions (vehicle, radiation, compass) are fixed at their new-game
values until those systems exist. The feedback hint that the game shows
after 70 s (`hintDelay`) is not ported.

## 2026-10-14 — M9u6b1: the HUD's bars drawn and driven

**Plan:** M9u6b was split in `docs/DESIGN.md` into b1 (shader, values,
texts) and b2 (the bars' animations). A new step, M9u6s (the other
screens' start rules), was added and moved out of M9u6e, because the
stored-state screens cover the bars in every screenshot.

**What/why:** the four round bars now follow the game's rules each
frame.
- `sn-ui`:
  - `state`: `UiState`, moved here from `sn-assets` so the rules stay
    pure. Sprites are ids. Per-node material values were added.
  - `bar`: `uGUI_CircularBar`'s material, as its setters write it, and
    `mesh::graphic_quad`.
  - `hud`: the rules (`uGUI_SceneHUD`'s elements by game mode; the four
    bars' `SetValue`, texts and caching; the oxygen overlays' scroll and
    alpha).
- `sn-assets`:
  - `UiGraphicKind::CircularBar`, drawn by the scene;
  - material values applied to a graphic's own material;
  - `hud_config`, `object_fields`, and `survival_start` (read from
    `Survival.ResetStats`' IL with the new `sn_dotnet::method_store_f32`).
- `sn-client`:
  - `game_ui_bar.wgsl` (`Hidden/UI/CircularBar` from its 8 + 8 compiled
    programs), as a third pipeline kind with the overlay texture;
  - the per-frame path: the main world sends `HudInputs`, the worker runs
    every frame's rules and draws the latest state, and per-frame logs
    are summarised.
- `sn-inspect ui --bars`.
- Fix: `mesh::color32` now rounds half to even, as `Mathf.Round` does
  (no hash moved).

No new dependencies.

**Verified (2026-10-14):**
- `ui --bars`, a 75 s dive with vitals from `sn_sim`, drawn every frame:
  - 300 of 300 checks equal a replay of the decompiled `SetValue`
    (`_Value` in the drawn material, the text, its quad count, and the
    oxygen overlay ST and alpha);
  - draw mean 2.6–2.9 ms.
- `ui --draw`: 12 of 12 hashes equal across two runs. `ui --live`: 0
  failures, mean 2.7 ms.
- Client, `--game-ui --resolution 1920 1080 --start -950 -616 1018 --look
  -942 -621 1026 --shot 30 m9u6b1-dive`:
  - the bars' per-second log follows the simulation (oxygen 45 → 0 in
    27 s, its text the rounded oxygen, food 51, water 91);
  - 176 of 176 draws; pass CPU 0.24–0.33 ms per frame;
  - worker 3.3–4.4 ms per frame;
  - 0 WARN, ERROR or panic lines.
- Tests:
  - 10 new unit tests: 6 for the HUD rules, 2 for the bar material, 2
    for the state (moved);
  - 1 more `sn-dotnet` assertion;
  - 1 new opt-in real-data test (`the_hud_bars_are_read`).
- `cargo test --workspace` without the game: 614 passed, 51 ignored. The
  13 opt-in UI tests pass. fmt and clippy (`-D warnings`) pass.

**Dead ends:**
- The first `ui --bars` run showed food and water at 0. That is what
  the main scene stores; `Survival.Start` → `ResetStats` sets 50.5 and
  90.5, now read from the IL.
- A Python edit script opened `crates/sn-assets/src/lib.rs` for writing
  before reading it and emptied the file. It had no uncommitted
  changes, so it was restored with `git checkout` and the edit was
  redone. Bash heredocs with long Python failed twice more; the scripts
  were written as files in `out/m9u6b/`.
- The first WGSL sampled after the program's branches, which WGSL
  forbids. The samples now come first.

**Not tested / not 1:1:**
- The screenshot shows the bars under the feedback panel, the
  confirmation dialog and the pop-ups (stored state); M9u6s comes next.
- The pulses, coin turn and punch are not ported (M9u6b2).
- `IsActive` is "the player runs": the intro, piloting and the camera
  drone are not ported.
- Float precision against the game's Mono runtime is a hypothesis.
  Health after a 20-point hit shows "81" in our port.
- `_BorderColor` made linear is a hypothesis.
- Not compared with the original game.

Reproduce (PowerShell):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo build --release -p sn-inspect -p sn-client
target/release/sn-inspect.exe ui --bars
target/release/sn-inspect.exe ui --draw
target/release/sn-client.exe --game-ui --resolution 1920 1080 --start -950 -616 1018 --look -942 -621 1026 --shot 30 m9u6b1-dive
cargo test -p sn-assets --test real_data the_hud_bars -- --ignored
cargo test --workspace
```
Expected: `ui --bars` ends with `300 bar checks, 0 differ`. The client
logs `game UI bars at N s: …` each second and `176 of 176 draws`.

## 2026-10-14 — M9u6a: the live UI (M9u6 split into a–e)

**Plan:** M9u6 (the HUD's rules) was split into five steps in
`docs/DESIGN.md`: a, the live UI; b, the four bars; c, depth and
compass; d, the hand reticle; e, death, respawn and overlays, with
`hud.rs` removed. Each step's scripts were read from the decompiled
`Assembly-CSharp` (read only, in `out/`) before the plan was written.

**What/why:** until now the UI was drawn once per window size. The HUD's
rules change it every frame, so this step adds the part they write to:
- `sn-assets::ui_scene`: `UiScene` reads a UI tree once. `UiScene::draw`
  rebuilds the draw list from a `UiState`, the scripts' changes through
  Unity's API: `SetActive`, `CanvasGroup.alpha`,
  `CanvasRenderer.SetAlpha`, `Graphic.enabled` and `color`,
  `Image.sprite` and `fillAmount`, TMP `text`, and the RectTransform's
  position, z, rotation and scale. `UiStartState` converts into a
  `UiState`. `ui_draw_placed` is now a scene drawn once, so the older
  checks run through the new code.
- `sn-ui::tween`: `Mathf.SmoothDamp` (decompiled
  `UnityEngine.CoreModule`) and UWE's `Sequence`. `sn-ui::draw`: a
  graphic's own renderer alpha.
- `sn-inspect ui --live`; the client's worker keeps the scene.
- `Assets::script_fields` is now public (the rules read their scripts'
  fields).

No new dependencies.

**Verified (2026-10-14):**
- Before the change, `ui --draw` and `ui --canvas` were run and their
  hashes saved in `out/m9u6a/`. After it, all 12 and all 4 hashes are
  equal.
- `ui --live` exits 0 with 0 failures:
  - stored state `83d22425b9e33ca7`, placed `f1365274ab2fbbf7` (the
    same as before);
  - 7 change kinds on drawn HUD nodes each change the draw list as
    expected, and the start hash comes back after each;
  - 120 frames with every HUD text, image alpha and bar icon changed:
    mean 2.66–2.81 ms, p95 2.4–3.1 ms (target ≤ 4 ms, written before
    the run);
  - a fresh scene gives the warm scene's hash.
- Client, `--game-ui --resolution 1920 1080 --shot 25 m9u6a-1080`:
  - 171 of 171 draws, as in M9u5d; the pass costs 0.242 ms CPU per
    frame;
  - 0 WARN, ERROR or panic lines;
  - screenshot `out/m9u6a-1080.png`.
- 12 new unit tests: 6 for `ui_scene` (each override kind, merging
  states, the start state, `Color32` rounding), 5 for `tween`, 1 for the
  renderer alpha.
- 1 new opt-in real-data test, `the_live_hud_returns_to_its_start_state`.
  It passes, and the 11 other opt-in UI tests still pass.
- `cargo test --workspace` without the game: 606 passed, 50 ignored.
  fmt and clippy (`-D warnings`) pass.

**Dead ends:**
- The first `ui --live` run missed the target: mean 4.15 ms. A
  throwaway timer (removed) showed texts took ~3 of ~3.9 ms. Half of
  that was cloning, padding and interning each text's material on every
  draw. These are now kept per (material, font). Same hashes, mean 2.7
  ms.
- The first sprite check set `halfMoonCompassNormal`, which is the
  half moon's stored sprite, so nothing changed. The check now picks
  the first half-moon sprite that differs from the stored one
  (`halfMoonNormal`).
- The 42 ms first frame is not the scene: it made 8 new digit glyphs
  (M9u3's ~5 ms each). The same state drawn again took 2.6 ms.

**Not tested / not 1:1:**
- The renderer alpha × inherited alpha, and `cullTransparentMesh`
  testing the product, are hypotheses (the batching is native).
- A text's fallback sub-meshes take their text's renderer alpha; in the
  game they have their own renderers.
- No HUD image is filled in the start state, so `fillAmount` on real
  data is not checked (unit tests only).
- Nothing sends a per-frame state yet. That comes with M9u6b's first
  rules. The client draws the start state as before.

Reproduce (PowerShell):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo build --release -p sn-inspect -p sn-client
target/release/sn-inspect.exe ui --live
target/release/sn-inspect.exe ui --draw
target/release/sn-inspect.exe ui --canvas
target/release/sn-client.exe --game-ui --resolution 1920 1080 --shot 25 m9u6a-1080
cargo test -p sn-assets --test real_data the_live_hud -- --ignored
cargo test --workspace
```
Expected: `ui --live` ends with `failures 0`, and its first two hashes
are `83d22425b9e33ca7` and `f1365274ab2fbbf7`. `ui --draw` shows 12 of
12 equal hashes. The client logs `171 draws` and `171 of 171 draws`.

## 2026-10-13 — M7g4e2: entrance water-field pixel lighting

**Checkpoint:** user authorized committing/pushing M7g4e1; `37872c2`
pushed to `origin/main`. The user subsequently authorized committing and
pushing this follow-up, with manual visual testing explicitly **not tested**.

**What/why:** connected light and sky inputs to the existing effect pass
and enabled only the exact entrance-field 148/466 program. Each draw receives
four game-unit lights, selected sky SH/exposure and frame ambient/water
parameters. Range/spot attenuation and directions are calculated at vertices,
then interpolated into pixel lighting. The decoded biased normal/wrap,
self illumination, conditional SH and alpha/fog/refraction order are retained.
Unused deformation-ST/clip.z slots carry this non-deformed UV variant's
coefficients. Global/lifepod sky changes are read each frame. Separate lit
and unlit shader variants avoid extra lighting varyings for existing effects.
No dependencies, whitelist or architecture changes.

**Limits:** Unity's exact native renderer-light ranking remains unknown;
we use deterministic estimated brightness at a bounding sphere. Range
lookup is the existing opaque-light analytic approximation. Water inputs
are camera-local, and original per-object SH orientation/image parity is
**not tested**. These are explicit limitations, not a 1:1 claim. Sand,
smoke noise and bubble vertex lighting/default bump remain gated.

**Verified:** workspace 594 passed, 50 opt-in ignored; fmt/clippy
`-D warnings` passed, release build passed in an isolated target directory.
Updated contract tests validate exact field selection, coefficient packing,
default/override handling and missing refraction rejection. Two added tests
check bounded/stable light selection, range/affected filtering, black padding
and range falloff endpoints/continuity. Existing crash/culling/animation tests
remain passing. Final five before/after GPU pairs each exit 0 with 300 settled
frames after 120 warmup at 1600 × 900. No shader errors or panics; all object
warning counts 0. Terrain and object stats match exactly except expected
material sky classifications. Field logs select the effect and receive
four local slots per draw (Lost River 1 draw, Lava Castle 2, prison 1);
brine/door regressions receive 0 lit draws. The prison scene emits the same
Bevy clustering resize advisory in both control and new runs; it is not a
shader validation failure and transient scene lighting during resize is
not visually tested.

| Scene | Mean ms before → after | p95 ms before → after | Objects / cached meshes | Final peak GiB |
|---|---|---|---|---:|
| Lost River entrance | 6.000 → 5.800 | 6.816 → 6.222 | 2,736 / 1,866 | 1.292 |
| Lava Castle entrance | 7.794 → 7.604 | 8.450 → 8.313 | 6,119 / 5,392 | 1.989 |
| Prison entrance | 8.885 → 8.939 | 9.511 → 9.646 | 13,032 / 6,837 | 1.744 |
| Tree Cove | 5.571 → 6.421 | 6.393 → 8.264 | 6,335 / 1,301 | 1.212 |
| Cache door/terminal | 5.671 → 5.640 | 6.096 → 6.095 | 8,201 / 1,835 | 1.242 |

One pair per camera, live animation and another user client running: these
are not isolated performance measurements. Cove's recent effects GPU scope
was 0.061781 → 0.062234 ms despite its higher total frame time; no speedup
or definitive total-frame regression attribution is claimed. Raw results
stay ignored under `out/m7g4e` and `out/baselines/run-m7g4e2-final-*`.

**Dead ends/corrections:** the ordinary release build could not replace the
user's running exe (Windows access denied). Left that client running and
copied our build cache into ignored `out/m7g4e/e2-target`, excluding the exe.
Clippy requested a query type alias; added it. First GPU comparisons prompted
separate lit/unlit pipelines rather than passing the extra varyings everywhere.
Unity 2019.4's documented spot reciprocal-quarter-angle cosine replaced the
initial fade-width estimate. Readiness remains numerical/GPU; manual visual
testing is **not tested** (deferred at the user's request). Default `target/release/sn-client.exe` still contains
the prior build while the user's process holds it open; use the isolated exe.

Reproduction (PowerShell):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sn-client --target-dir out/m7g4e/e2-target
out/m7g4e/e2-target/release/sn-client.exe --benchmark 300 --baseline field-lighting-check --baseline-revision local-M7g4e2 --warmup 120 --resolution 1600 900 --start -950 -616 1018 --look -942 -621 1026 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```
Expected: checks exit 0, field logs `pixel_lighting: true` as an effect
material and `1 parts ... 4 local slots`, with no shader errors/panics.
Use a fresh baseline name to repeat. Other camera arguments:
Lava Castle `--start -35 -1198 70 --look -31.68 -1200.82 77.63`, prison
`--start 208 -1445 -251 --look 215 -1448 -257`; brine and terminal use
the previous milestone's reproduction table. The control exe was copied
from the successful M7g4e1 release build before editing M7g4e2.

## 2026-10-13 — M7g4e1: lit particle input contract and reference terms

**What/why:** started the next visual milestone. Read the installed
119/437, 142/460, 148/466 and 154/472 programs and all four target materials;
documented exact saved/compiled mappings, scalar inputs, vertex attenuation,
stage order, biased normal/wrap, self illumination and conditional SH.
Added exact-set diagnostics so logs distinguish recognized lighting from
missing renderer inputs, plus a synthetic pixel-lighting numerical oracle.
Four families remain on stand-ins: **GPU lighting is not implemented yet**.
No dependencies, architecture or whitelist changes. No commit or push.

**Findings/dead ends:** opaque clustered lighting cannot simply replace
the four renderer-specific Unity vertex-pass light slots: attenuation is
computed per vertex even in the pixel variants. Saved sand normal-map/
two-sided/quality keywords and bubble light-mode-off do not identify active
compiled terms. Pixel SH is added after textured albedo; bubble SH enters
vertex colour before textures, so one shared combine would be wrong.
Smoke needs vertex noise and bubbles need a neutral refraction default.
Exact engine brightest-light ranking remains unconfirmed. Initial searches
used shell wildcards with rg on Windows; explicit files resolved them.

**Verified:** 592 workspace tests passed, 50 opt-in tests ignored on current
HEAD `3c07188`; four new tests cover exact mapping/default overrides,
missing/non-finite inputs, self-illumination endpoints/alpha, SH absolute
value/exposure/desaturation/gating, and wrap/normal-bias behaviour. Fmt,
clippy `-D warnings`, release build and diff checks passed. Four GPU runs
(120 settled frames, 60 warmup, 1600 × 900) exited 0 with no WARN/ERROR,
shader errors or panics; all expected target contracts logged. Mean/p95 ms:
water field 5.639/6.164, sand 6.468/6.928, smoke 7.423/7.875, bubbles
8.875/9.531. These validate loading/diagnostics, not the future lit shader.
Original-game comparison and new lit GPU output are **not tested**.

Reproduce (PowerShell):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo test --workspace
cargo test -p sn-client lit_particle
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sn-client
target/release/sn-client.exe --benchmark 120 --baseline lit-input-field-check --baseline-revision local-M7g4e1 --warmup 60 --resolution 1600 900 --start -950 -616 1018 --look -942 -621 1026 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off
```
Expected: checks exit 0, four targeted tests pass; field logs `lit program
148/466 (pixel)` with five coefficients and `renderer light inputs not
connected`. A fresh baseline name is needed when repeating. Other diagnostic
camera arguments: sand `--start -290 -175 958 --look -293 -177 971`, smoke
`--start -140 -1140 10 --look -143 -1148 28`, bubbles
`--start 350 -1452 -280 --look 357 -1456 -286`. Logs/reports stay ignored in
`out/m7g4e` and `out/baselines/run-m7g4e1-input-*`. Continue by feeding light/
sky data to the effect pass before enabling any of these exact sets.

## 2026-10-13 — M9u5d: the client draws the game's UI text (M9u5 done)

**What/why:** the last step of M9u5. With `--game-ui`, TMP texts now
draw with the game's own shader.
- `game_ui_text.wgsl` ports `TextMeshPro/Distance Field` from its D3D11
  programs: the vertex's SDF scale, perspective filter, weight, bias and
  underlay terms, the uv2 unpacking and the clip mask; the fragment's
  face, outline, softness, underlay and soft clip. The output is
  premultiplied.
- In `game_ui.rs`: TMP draws carry their material values; a second item
  buffer and pipeline kind draw them; the worker sends each font atlas
  (R8, from `GlyphAtlases`) and sends it again when glyphs are added; the
  main world replaces the atlas image in place.
- `README.md` now lists the drawn HUD (tested claims only).

No new dependencies.

**Verified (2026-10-13, Lifepod start, `--game-ui --shot 25`):**
- 1920×1080 and 2560×1440 each give 171 of 171 draws: TMP 75,
  `UI/Default` 84, `UI/Simple` 10, `UI/Bar` 2 (drawn as `UI/Default`).
  That is 3,720 vertices.
- Glyphs: 101 (64 in `Aller_Rg SDF`, 37 in `Aller_W_Bd SDF`) in
  0.69–0.74 s on the worker. No atlas filled up.
- The pass costs 0.232 ms and 0.228 ms CPU per frame, mean of 600,
  against M9u5's target of 0.5 ms. 0 WARN or ERROR lines, no panics.
- Screenshots `out/m9u5d-1080.png` and `-1440.png`: sharp text, both
  faces, the shadowed underlay style, the same layout scaled.
- 2 new unit tests: the face alpha crossing 0.5 at the outline (weight
  and outline widen it), and the text normal facing the camera.
- `cargo test --workspace`: 588 passed, 50 ignored. fmt and clippy
  (`-D warnings`) pass.
- M9u5's own list: draw calls, vertices and textures per canvas logged;
  CPU < 0.5 ms for the HUD; screenshots at both sizes; 0 GPU errors.
  All met.

**Dead ends:**
- A Python heredoc tripped the shell's quoting again (nothing was
  written); the edit script was written as a file in `out/m9u5/`.
- A field set but never read (`underlay_on`) was removed; the flag
  carries it.
- One new test failed on my own hand-computed case: weight 0.05 puts the
  edge exactly at 0.45, giving 0.5, not more. I fixed the test value,
  not the formula.

**Not tested / not 1:1:**
- The strings are the prefab's stored ones: placeholders and
  untranslated keys. Translation and each screen's text are M9u6 and
  M9u7.
- `_FaceTex` and `_OutlineTex` are drawn white (none is set in scope).
- The glow and bevel terms are not in this shader, and
  `UNITY_UI_ALPHACLIP` is used by no material.
- The 2 texts under the unplaced `Tooltip` canvas are not drawn.
- Not compared against the original game's screen.

## 2026-10-13 — M9u5c: the client draws the game's UI images

**What/why:** the first step of M9u that draws on screen. With
`--game-ui` (off by default), the client draws `uGUI`'s images as the
game does.
- `apps/sn-client/src/game_ui.rs`:
  - a worker thread reads the install, places the canvases for the
    window's size, applies the start-up state and builds the draw list,
    again on resize. Textures are decoded once, bottom row first.
  - the main world turns textures into images.
  - a render pass of our own after tonemapping draws in the list's
    order, with per-item values in a storage buffer and one pipeline per
    blend state.
- `game_ui.wgsl` ports `UI/Default` (soft clip) and `UI/Simple` (hard
  clip, premultiply) from their D3D11 programs.
- `--game-ui` also hides our bars and depth; our hand text and fade
  stay.
- `sn-assets::ui_start` (new) ports the first-frame state of
  `uGUI_Fader`, `uGUI_SceneIntro`, `uGUI_Overlays` and
  `uGUI_HardcoreGameOver`.
- Smaller additions: `DrawItem::root`, `UiDraw::textures` and
  `UiNode::game_object`; `ui_draw_placed` now takes a `UiStartState`.

New dependency: `sn-client` now uses `sn-ui` (canvas placement; our
own crate). Decompiled `uGUI_Fader`, `uGUI_SceneIntro`,
`uGUI_SceneRespawning`, `uGUI_Overlays`, `uGUI_HardcoreGameOver` and
`Sequence` into the ignored `out/decompiled/full/` (read only).

**Verified (2026-10-13, Lifepod start, `--game-ui --shot 25`):**
- 1920×1080 and 2560×1440 each give 96 of 96 draws, 1,024 vertices, 69
  textures and 54 textures on `ScreenCanvas`.
- By shader: `UI/Default` 84, `UI/Simple` 10, `UI/Bar` 2 (drawn as
  `UI/Default` and logged).
- Skipped and counted: 77 texts (M9u5d), 1 `DepthClear` and 1 draw
  without a placed canvas.
- The pass costs 0.206 ms (1080) and 0.209 ms (1440) CPU per frame,
  mean of 600. Item values take 0.07–0.09 ms per size change, and a
  worker build about 1.05 s.
- 0 WARN or ERROR lines and no panics. Screenshots: `out/m9u5c-1080.png`
  and `out/m9u5c-1440.png`.
- Without `--game-ui`: no game-UI work in the log, 0 warnings, our HUD as
  before (`out/m9u5c-off.png`).
- 4 new unit tests: the soft-clip formula, a canvas corner landing on
  the clip-space corner, Unity's blend modes, and a curve's value at 0.
- `cargo test --workspace`: 586 passed, 50 ignored. fmt and clippy
  (`-D warnings`) pass.

**Dead ends / corrections:**
- The first run's screenshot was entirely black. A log of full-canvas
  draws showed four active, opaque full-screen black panels in the
  stored prefab. That led to porting the scripts' start-up state
  instead of drawing the stored state as the plan said.
- Bevy's `BufferSlice` is not wgpu's: the buffers are dereferenced
  first, as `effects.rs` does.

**Not tested / not 1:1:**
- The other screens' stored state still shows (feedback smileys, the
  red confirmation dialog, pop-ups; their scripts show them only when
  asked; M9u6, M9u7).
- Texts are not drawn (M9u5d).
- Unconfirmed hypotheses: vertex colours made linear,
  `_TextureSampleAdd`, `UI/Simple`'s unclipped `_ClipRect`.
- `_Cull` is not applied, there are no mips, and the `Tooltip` canvas
  (no scaler) is not placed.
- Not compared against the original game's screen.

## 2026-10-13 — M9u5b: the UI camera and canvas placement

**What/why:** the client must place the game's canvases as the game
does before it draws them (M9u5c).
- `sn-ui::scaler` ports `uGUI_CanvasScaler`'s Static mode
  (`UpdateTransform`, `UpdateFrustum`, `SetScaleFactor`) and the UI
  camera (`SNCameraRoot`: at the origin, no rotation, the main camera's
  field of view and planes), with Unity-convention projection and
  `SetSizeWithCurrentAnchors`.
- `sn-assets` adds `Assets::ui_scalers` and `Assets::ui_draw_placed`,
  which lays a tree out at the placed canvas sizes.
- `sn-unity::Camera` now exposes `clear_flags`, and `scene --tree`
  prints it and the camera depth.
- New tool: `sn-inspect ui --canvas`.

Decompiled `uGUI_CanvasScaler`, `ManagedCanvasUpdate`, `GraphicsUtil`,
`MainCamera` and `SNCameraRoot` with `ilspycmd` into the ignored
`out/decompiled/full/` (read only). No new dependencies, no
architecture change.

**Verified:**
- `sn-inspect ui --canvas` (exit 0, 1.1 s; `out/m9u5/canvas-final.txt`):
  - placements: 16 of 16 (4 Static scalers × 1920×1080, 2560×1440,
    1920×1200, 3440×1440). The canvas corners land on the screen corners
    within 0.0001 px; the target written before the run was ≤ 0.5 px.
  - `uGUI` laid out and drawn at each size: 178 draws, 0 non-finite.
    1920×1080 and 2560×1440 give the same hash, as the rule predicts.
  - 4 draws reach past the canvas (named in the log).
  - The PDA screen (Inversed) is logged as not placed (M9u8a).
- `scene main --tree`: the UI camera has clear flags 4 and depth 2; the
  main camera has 1 and 0.
- 7 new unit tests: the reference size, larger, 16:10 and ultrawide
  screens, UI scale and `DontScale`, corners on screen from a turned and
  moved camera at several fields of view, a mismatched field of view
  missing the corners, the anchored modes refused, and
  `SetSizeWithCurrentAnchors`.
- 1 new opt-in real-data test, `the_hud_canvases_fill_the_screen`
  (passes).
- `cargo test --workspace`: 582 passed, 49 ignored. fmt and clippy
  (`-D warnings`) pass.

**Dead ends:** clippy's `needless_range_loop` on the axis loop (fixed
with `enumerate`). One `ui --canvas` run had no `SUBNAUTICA_DIR` (it
exited with "no game folder"); rerun with it set.

**Not tested:** anything on screen (M9u5c); VR; the anchored modes; UI
scales other than 1 against the game.

## 2026-10-13 — M9u5a: materials and text vertices

**What/why:** the first step of M9u5. The client needs each UI draw's
material and real text vertices before it can draw them (M9u5c–d).
- `sn-ui::material`: `UiMaterial` and TMP's material rules, ported from
  the decompiled `TMPro.ShaderUtilities` and `TMP_MaterialManager`
  (`UpdateShaderRatios`, `GetPadding`, `GetFallbackMaterial`,
  `PackUV`).
- `sn-ui::text::mesh`: each character's atlas UVs (`SaveGlyphVertexInfo`)
  and `uv2` (packed face UV and SDF scale), with sub-meshes per font.
  Layout takes each font's material padding and style padding
  (`Pads`, `layout_text_with`). `layout_text` with one padding is
  unchanged.
- `DynamicAtlas::add_glyph` (by glyph index; `add_character` uses it),
  `draw::scale_y_in_root_canvas`, a `uv1` on `mesh::Vertex`, and a
  `material` on `draw::Graphic` and `DrawItem` (both in the hash).
- `sn-assets::ui_draw`: the draw-list builder, moved from `sn-inspect`
  so the client can use it. It now reads `m_Material`,
  `m_sharedMaterial` and the font assets' materials, and makes the
  texts' glyphs into per-font atlases (`GlyphAtlases`).
- `sn-inspect ui --draw` now also logs the census per shader and
  keyword set, the shaders M9u5c–d won't port, the text UV checks and
  the glyph cost.

No new dependencies, no architecture change, no whitelist change.
Decompiled `TMPro.ShaderUtilities` and `TMP_MaterialManager` with
`ilspycmd` into the ignored `out/decompiled/tmp/` (read only).

**Verified:**
- `sn-inspect ui --draw` (exit 0, 2.6 s; `out/m9u5/draw-final.txt`):
  - every draw item has a material, 372 of 372;
  - census: `UI/Default` 208, TMP SDF 105 (7 `UNDERLAY_ON`, 2 clipped),
    `UI/Simple` 12, `UI/Bar` 2, `uGUI/WorldCursor` 1, `UI/Circle` 43,
    `uGUI/DepthClear` 1;
  - 817 text quads: 0 UVs outside the atlas, 0 centres outside a packed
    glyph, 0 reaching past their packed rect;
  - 110 glyphs made in 0.325 s, against a target of ≤ 1 s written into
    DESIGN.md before the run;
  - two runs from scratch: 12 of 12 hashes equal.
- `ui --texts` unchanged: 20 of 20 auto-sizes equal.
- 12 new unit tests (hand-computed): ratios, padding (plain, outlined,
  extra, underlay, bitmap), fallback material, `PackUV`, the clip
  keyword, quad padding, the bold style padding and its clamp, text
  UVs and `uv2`, bold and non-SDF UVs, the colour cap, missing glyphs,
  and the root-canvas scale.
- 1 new opt-in real-data test, `the_hud_draws_with_materials_and_glyphs`
  (passes with `SUBNAUTICA_DIR`; `the_pda_screen_draw_list_is_stable`
  still passes).
- `cargo test --workspace`: 575 passed, 49 ignored. `cargo fmt --all`
  and `cargo clippy --workspace --all-targets -- -D warnings` pass.

**Found:** `UI/Circle` (PDA equipment slots and intro circles) and
`uGUI/DepthClear` were missing from the research. Both are now in
`docs/formats/ui.md`; `UI/Circle` moves to M9u8b, and M9u5c skips
`DepthClear` and logs it. The clip keyword is set by native code (not in
the IL), so it stays a hypothesis.

**Dead ends:** an inline Python heredoc that rewrote the inspector
broke the shell's quoting (nothing was written); the file was written
directly instead.

**Not tested:** anything on screen (M9u5c–d); the game's own atlas
order (ours differs, by design); the sub-mesh order against text
children.

## 2026-10-13 — M9u5 research and split

**What/why:** M9u5 (drawing the game's UI in the client) is too large
for one change. It is split into M9u5a (materials and text vertices,
headless), M9u5b (UI camera and canvas placement, pure), M9u5c (the
client draws images) and M9u5d (the client draws text) in
`docs/DESIGN.md`. The facts found are in `docs/formats/ui.md` § UI
shaders and materials. Docs only. No code, dependencies or whitelist
changes.

**Found** (UnityPy 1.25.4 reinstalled into a fresh system-temp folder
`snrs-oracle3`, because `snrs-oracle2` had lost its package files again;
D3D11 programs disassembled with `d3dcompiler_47`; scripts and dumps in
the ignored `out/m9u5/`): where each UI shader is (`UI/Default` is in
`unity_builtin_extra`, path id 10770); each pass's blend, depth, cull and
keyword variants; the materials of `uGUI`'s bundle and the fonts bundle
with their keywords; what each program computes and which constants it
reads. Notable: `UI/Simple` clips with `_ClipRect` in every variant;
`UI/Default` and TMP draw `RectMask2D` softness; TMP's output is
premultiplied; `WorldCursor`'s `_ALPHAPREMULTIPLY_ON` keyword selects no
program.

**Dead ends:** the first dump used 8-byte blob entries (they are 12:
offset, length, segment) and passed bytes after the `DXBC` container to
the disassembler, which returned nothing; fixed in the script.
`UI/Default` is not in `unity default resources`.

**Not tested:** everything marked *hypothesis* in the new section (the
clip keyword rule, `_TextureSampleAdd`, the UI z test, vertex colour
space). M9u5a–b check them.

## 2026-10-13 — key-terminal material and optimized skeleton correction

**Checkpoint:** user authorized committing/pushing the prior M7g4d work;
`ff4ef2d` pushed to `origin/main`. User also approved the larger terminal
correction. This correction was initially left uncommitted for testing;
the user subsequently authorized committing and pushing it.

**What/why:** fixed the two concrete omissions behind the user's cache
screenshot. `sn-unity` retains mesh bone-name hashes and reads the checked
startup prefix of `PrecursorKeyTerminal`. `sn-assets` applies its selected
face material and reconstructs its optimized generic avatar skeleton,
including flattened exposed nodes such as the glyph. Mesh palette hashes
resolve in bind-pose order. Parents-first sorting preserves cached node
references and object identities; hidden nodes receive separate synthetic
identities. Existing animation, rigid effect parenting and GPU skinning
then use the default closed pose. No shader offsets, hard-coded geometry,
new dependencies, architecture layers or whitelist changes. Scope is active,
enabled key terminals; other optimized families are not enabled by this fix.

**Verified:**
- Purple/orange/blue templates: each 240/240 clip bindings resolved, 0
  missing, seven mesh palette hashes mapped to seven bones; three distinct
  face materials selected. Purple now uses the purple material rather than
  the saved orange one. Original serialized hierarchy 9 nodes, restored 16.
- Opt-in real-data test: all three default closed poses ran for 60 steps;
  30 bound pose slots, 1,903 finite body vertices. Glyph centre to nearest
  body vertex 0.215570 m in each (not a triangle-surface gap or visual oracle).
- Synthetic tests: restored hidden parent/exposed child, composed placement,
  cached references after sorting; invalid arrays/parents, ambiguous names
  and non-finite poses rejected; startup selection and all truncated prefixes
  return checked errors. `cargo test --workspace`: 563 passed, 48 ignored.
  `cargo test -p sn-assets` repeated after the unrelated-prefab early return.
- `cargo fmt --all -- --check`, clippy with `-D warnings`, release client/
  inspector build and `git diff --check` passed.
- Final GPU runs: cache door, Gun control room and Tree Cove; each exit 0,
  300 settled measured frames after 120 warmup frames, 0 WARN/ERROR/panics
  or object warnings. Door logs a purple effect attached to its rig node.
  Mean/p95 ms: door 5.099/5.393, Gun 8.818/9.347, Cove 5.053/5.482.
  Peak memory 1.434/2.331/1.211 GiB. Cached mesh/material counts remain
  1,835/336, 6,815/857, 1,301/266 respectively. Object despawn-root counts
  drop by one per restored terminal (door 8,201, Gun 20,667): the body and
  glyph now share one tracked rig base instead of separate tracked roots.
  Cove remains 6,335. Raw logs/reports are ignored under `out/m7g4d` and
  `out/baselines/run-m7g4d-terminal-final-*`.

**Corrections/dead ends:** temporary inspector research print initially
used the wrong FileRef member; removed all temporary research printing.
Clippy caught a Copy-type clone in a test; removed. The first real-data
test compilation used `Animator.pose` instead of its `pose()` accessor;
corrected. No game data was added to source. Fresh human confirmation,
original-game visual parity, insertion gameplay and other optimized-avatar
families are **not tested**. Do not call visual acceptance complete yet.

Verification commands (PowerShell, repository root):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo test --workspace
cargo test -p sn-assets --test real_data optimized_key_terminals -- --ignored --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sn-client -p sn-inspect
target/release/sn-inspect.exe anim WorldEntities/Environment/Precursor/Precursor_PurpleKeyTerminal.prefab --states
target/release/sn-client.exe --benchmark 300 --baseline terminal-cache-check --baseline-revision local-terminal-fix --warmup 120 --resolution 1600 900 --start -622 -558 1482 --look -612 -563 1483 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```
Expected: tests/checks exit 0; inspector reports 240 found, 0 missing;
GPU log selects `Precursor_Symbol_Purple`, attaches its rigid effect to
the rig, and has no validation errors. Use a fresh baseline name to repeat.
Gun/Cove runs retain those flags and use the earlier M7g4d camera table.

## 2026-10-13 — M7g4d terminal human-test failure

The user's Blood Kelp cache screenshot shows the orange symbol detached
from the terminal. Earlier numerical shader checks did not prove correct
terminal appearance. Confirmed with `sn-inspect prefab` and `anim --states`:
purple terminal, 9 serialized nodes, 12 avatar skeleton nodes, 0/240 clip
bindings resolved. Read-only UnityPy inspection confirms an optimized
animator (`m_HasTransformHierarchy` false), empty renderer bone references
and null root bone. `sn-inspect code --il PrecursorKeyTerminal Start`
confirms startup replaces the stored orange material using
`keyMats[acceptKeyType]`. The mesh palette mapping remains unconfirmed.

Dead end: initial basename-only prefab queries did not resolve catalog
keys; using the full `WorldEntities/Environment/Precursor/…` key worked.
Moving the symbol alone would not address missing body skinning or material
selection. No rendering fix is claimed here; documented a bounded proposal
in DESIGN and corrected README's readiness caveat. The larger animation
loader extension needs approval under AGENTS.md. No commit/push.

Reproduce the diagnosis (set `SUBNAUTICA_DIR` as below):
```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
target/release/sn-inspect.exe prefab WorldEntities/Environment/Precursor/Precursor_PurpleKeyTerminal.prefab
target/release/sn-inspect.exe anim WorldEntities/Environment/Precursor/Precursor_PurpleKeyTerminal.prefab --states
target/release/sn-inspect.exe code --il PrecursorKeyTerminal Start
```
Expected: the saved face material is orange, renderer lists 0 bones,
animation reports 0 bindings found/240 missing, startup IL references
`keyFace`, `keyMats`, `acceptKeyType` and `Renderer.set_material`.
This follow-up changed docs only; no new code tests were needed.

## 2026-10-13 — M7g4d: remaining unlit UBER mesh effects

**What/why:** resumed the visual track and finished the interrupted slice.
Tech light cones, Gun elevator smoke/dots, control-room inner blocks,
thermal reactor electricity, lava falls and orange key symbols now use
their individually decoded programs rather than the stand-in material.
The effect shader separates projection, near fading, brightness opacity,
alpha power and back culling into independent confirmed terms. Selection
still requires an exact decoded keyword set (14 sets). No dependencies,
architecture changes or whitelist changes; no commit or push.

**Findings/corrections:** `_MyCullVariable` 2 is pass state and also applies
to the elevator/control-room layers, not just brine. The lava alpha power
acts before texture multiplication. The orange symbol has saved speeds
but its program has no `FX_SCROLL`: zero all speeds for this set. The
interrupted session's first release executable preceded that last loader
edit; rebuilt before final verification. No original-game visual oracle
was available: visual parity, refraction Y sign and mesh-local fog remain
**not tested**. Lit sand/smoke/bubble/water-field variants and kelp
billboards remain unsupported; next visual slice is M7g4e's light inputs.

**Code checks:** `cargo test --workspace` passed (560 tests, 47 opt-in
tests ignored); `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo build --release -p sn-client` and `git diff --check` passed.
Synthetic checks cover exact selection/rejection, keyword terms, render
state, saved-speed gating and alpha-power order/ordinary opacity. Existing
late-reflection and already-extracted-mesh crash tests passed too.

**GPU verification:** matched sequential runs against a freshly built
detached `e3a1bf1` control in `out/m7g4d/head`, using its own build directory.
Raw logs and reports are ignored under `out/m7g4d` and `out/baselines`.
All 14 runs exited 0, each measured 300 settled frames after 120 settled
warmup frames at 1600 × 900. All seven final runs had zero WARN/ERROR
lines, shader validation errors or panics and zero object warnings. The
target materials logged as `effect material`. Terrain stats and object
stats except material `per_sky` classifications match each control exactly;
`per_sky` shrinks where a material moves into the custom effect pass.

| Scene | Mean ms before → after | p95 ms before → after | Terrain triangles | Object entities / meshes | Final effect materials |
|---|---|---|---:|---|---:|
| Tech wreck | 12.266 → 11.927 | 13.423 → 12.786 | 3,752,060 | 11,686 / 3,587 | 14 |
| Gun elevator | 6.694 → 6.663 | 7.166 → 7.188 | 1,305,866 | 18,419 / 6,790 | 19 |
| Gun control room | 9.122 → 9.303 | 9.727 → 10.215 | 3,637,338 | 20,672 / 6,815 | 19 |
| Thermal reactor | 7.041 → 6.358 | 8.752 → 6.881 | 3,995,040 | 6,114 / 5,326 | 22 |
| Lava fall | 4.474 → 4.408 | 4.940 → 4.881 | 1,873,002 | 524 / 1,267 | 5 |
| Tree Cove | 5.202 → 5.168 | 5.612 → 5.589 | 3,948,704 | 6,335 / 1,301 | 5 |
| Precursor door | 5.623 → 5.622 | 6.106 → 6.105 | 1,485,976 | 8,202 / 1,835 | 12 |

Peak final process memory ranged from 1.096 to 2.283 GiB. These are one
pair per camera with live effect animation, not proof of a speedup or
deterministic image equality. Original-game visual comparison: **not tested**.

Reproduction (PowerShell, from the repository root):

```powershell
$env:SUBNAUTICA_DIR = 'D:\SteamLibrary\steamapps\common\Subnautica'
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p sn-client
target/release/sn-client.exe --benchmark 300 --baseline m7g4d-elevator-check --baseline-revision local-M7g4d --warmup 120 --resolution 1600 900 --start 420 -76 1222 --look 428 -79 1229 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expected: exit 0, `x_Gun_Elevator_TubeSmk` and
`x_Gun_Elevator_TubeDots` log as `effect material`, no shader validation
errors/panics, report has 300 measured frames and 0 unsettled frames.
Use a fresh baseline name when repeating. For other matched runs retain
all common flags, change the baseline name and use these camera arguments:

| Scene | Camera arguments |
|---|---|
| Tech light wreck | `--start -1222 -348 -373 --look -1229 -351 -380` |
| Gun control room | `--start 356 -68 1075 --look 360 -71 1083` |
| Thermal reactor | `--start -30 -1225 104 --look -40 -1230 112` |
| Lava fall | `--start -845 -1200 80 --look -859 -1207 87` |
| Tree Cove regression | `--start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6` |
| Precursor door regression | `--start -622 -558 1482 --look -608 -561 1483` |

## 2026-10-13 — M9u4d: the draw list

**What/why:** M9u4's last part. Each graphic is placed in uGUI's order,
in its canvas's space, with its inherited alpha and clip rect, ready for
the client to draw in M9u5 (`docs/formats/ui.md` § Draw order). New:
- `sn-ui::draw` (pure): `draw_list` (draw groups and order, transforms,
  `CanvasGroup` alpha, `RectMask2D` clipping and culling, transparent
  culling) and `DrawList::hash`. Clipping is read from the decompiled
  `UnityEngine.UI.dll` (read only, in `out/`).
- `sn-ui::layout::RectTransform` keeps rotation and `z`.
- `sn-assets`: `Assets::ui_draw_parts` (canvases with their sorting
  layer's index, canvas groups, rect masks, `m_Maskable`,
  `cullTransparentMesh`). A plain `Transform` is now a zero-size rect at
  its position, not a default 100 × 100 one. This changed no M9u4a–c
  result.
- `sn-inspect ui --draw`.
No new dependencies.

**Corrections:** one unit test of mine expected a clip rect for a
graphic that the mask correctly culled (its rect no longer overlapped).

**Verified:**
- `sn-inspect ui --draw` (exit 0): 12 of 12 hashes equal over two runs
  from scratch; uGUI 174 draws, PDA screen 192 (2 clipped); 0
  non-finite.
- `ui --rects`, `ui --texts`, `ui --meshes`: unchanged (370 of 370
  stored as 0, 20 of 20 auto-sizes, 0 mesh failures).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored the_pda_screen_draw`: 1 passed.
- 6 unit tests: hierarchy order and canvas space, rotation and scale,
  override sorting, canvas-group alpha and transparent culling, mask
  clipping (padding, intersection, culling, not maskable, an override
  canvas between), the hash.
- Not drawn yet (M9u5).

## 2026-10-13 — M9u4c: TMP text layout

**What/why:** M9u4's third part: where TextMesh Pro puts each character,
and a text's preferred size for the layout system
(`docs/formats/ui.md` § Text layout). Read from the game's
`Unity.TextMeshPro.dll` decompiled with `ilspycmd` into `out/` (read
only, never committed, as in earlier milestones); our own code with the
same control flow. New:
- `sn-ui::text` (pure): `input` (processing array, escapes, `<br>` and
  co., the tag parser), `FontSet` (character lookup through weight
  tables and fallbacks, synthesized control characters), `generate`
  (`GenerateTextMesh`'s layout and the auto-size loop), `preferred`
  (`CalculatePreferredValues`), `Texts` (a `TextSizer`).
- `sn-ui::layout`: `layout_with` and the `TextSizer` trait (TMP texts as
  layout elements).
- `sn-assets`: `Assets::tmp_font_set`, `Assets::ui_texts` (`UiText`).
- `sn-inspect`: `ui --texts` (census and checks); `ui --rects` sizes
  texts now; `ui --fonts` prints the weight tables and `TMP Settings`'
  font-feature flags.
No new dependencies.

**Found / corrections:**
- The plan's check ("text-fitted sizes equal the stored within 0.5
  px") cannot work: those rects are driven and saved as 0 (M9u4a). The
  oracle used instead is auto-size, whose result TMP stores in
  `m_fontSize`.
- Runtime kerning is on in `TMP Settings` (`getFontFeaturesAtRuntime`);
  3 in-scope texts enable kerning. Not ported yet (logged).
- My first test expectations used a space of 2.5 instead of 2.25 (250
  units at 90 px → 22.5); the code was right. I had also first read
  `<i=15>` as an angle; TMP's angle is the `angle` attribute.

**Verified:**
- `sn-inspect ui --texts` (exit 0): auto-size 20 of 20 equal the stored
  sizes (non-trivial ones: 25.3 after 12 and 11 passes, 40 after 11);
  137 texts laid out in 12.8 ms, 0 non-finite.
- `sn-inspect ui --rects` (exit 0): 370 of 370 driven values stored as
  0; unknown 19 (was 168).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored auto_sized`: 1 passed.
- Unit tests (hand-computed on the test font): one line at the top
  left, alignment, wrapping and new lines, truncation, auto-size,
  `<size>`, `<b>` (bold spacing), `<color>`, unknown tags and missing
  characters, kerning and spacing, preferred sizes, a size fitter
  around a text; tag parsing and escapes.
- `cargo test --workspace`: 552 passed, 0 failed, 46 ignored. Clippy
  and fmt: pass.
- Not drawn yet (M9u5); not compared with the game on screen.

## 2026-10-13 — M9u4b: Image and RawImage meshes

**What/why:** M9u4's second part, the meshes the game's images build
(`docs/DESIGN.md`, `docs/formats/ui.md` § Graphic meshes). New:
- `sn-ui::mesh` (pure): `image_mesh` (simple, sliced, tiled, filled,
  each radial fill, preserve aspect, trimmed margins, pixels per unit
  multiplier), `raw_image_mesh`, `SpriteInfo`, `color32`. Written from
  the IL of `UnityEngine.UI.dll`.
- `sn-assets`: `Assets::ui_graphics` (images, raw images, texts, the
  game's own graphics), `Assets::sprite_info` (own texture or atlas
  render data).
- `sn-inspect`: `ui --meshes`; `code --extends <BASE> [<ASSEMBLY>]`.
No new dependencies.

**Corrections:** I had first listed `uGUI_TextGradient` as a graphic.
`code --extends Graphic` shows it is not (it is a mesh modifier); the
list of the game's own graphics now comes from the DLL (8 classes).

**Verified:**
- `sn-inspect ui --meshes` (exit 0): 243 meshes, 2,028 vertices (counts
  per kind in `ui.md`), 0 fail the checks.
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored ui_image_meshes`: 1 passed.
- 8 unit tests worked out by hand: no sprite, a trimmed sprite's
  margin, preserve aspect around the pivot, sliced (9 parts, no centre,
  squeezed borders, pixels per unit multiplier, without borders),
  tiled (repeat UVs, cut tiles, bordered), horizontal and vertical
  fills, radial fills (90/180/360 × 4 origins × both directions cover
  half and all at fill 0.5 and 1; 360 at 0.25 and 0.75), raw images and
  colours.
- Not drawn yet (M9u5); not compared with the game on screen.

## 2026-10-13 — M9u4a: RectTransform rects and uGUI's layout system

**What/why:** M9u4 is split into a–d in `docs/DESIGN.md` (one concern
per change); this is the first part. New:
- `sn-ui::layout` (pure): `Tree` (rects from anchors, pivot and size
  delta; `local_position`; `SetSizeWithCurrentAnchors`) and `layout`
  (`LayoutRebuilder`'s roots and four passes, `LayoutUtility`'s priority
  rule, horizontal and vertical groups, `ContentSizeFitter`,
  `LayoutElement`, `AspectRatioFitter`, `uGUI_CircularBar`, `Image` as an
  element). Unknown text sizes are `NaN` and stay so.
- `sn-assets::ui_layout` (`Assets::ui_layout`): a `UiTree` as a layout
  tree, its fields read through M9u1's layouts.
- `sn-dotnet`: `Assembly::interfaces` (InterfaceImpl), and
  `Builder::implements` for tests.
- `sn-inspect`: `ui --rects`, `code --implements <INTERFACE>
  [<ASSEMBLY>]`, and `code --fields` taking `ASSEMBLY:TYPE` and listing
  interfaces.
No new dependencies.

**Dead ends / corrections:**
- The plan's oracle hypothesis was wrong. The first run showed 118
  "differences", nearly all with the stored value 0. Unity zeroes
  driven values when it saves. The check now tests that instead:
  370 of 370 driven values are stored as 0. So no stored value can
  check our numbers; that is left to unit tests and M9u5's screenshots.
- One unit test of mine was wrong, not the code: a child with one
  ignoring and one non-ignoring `LayoutElement` stays in its group
  (Unity keeps it if any ignorer does not ignore).

**Verified:**
- `sn-inspect ui --rects` (exit 0): driven values 370, all stored as 0;
  168 unknown (text or unported); a second run moves 0 nodes; 97 active
  zero-size rects that we do not drive are listed (see `ui.md`).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored layout_drives`: 1 passed (327 values in `uGUI` and the
  PDA screen, all stored as 0).
- Unit tests: 12 layout tests on synthetic trees, values worked out by
  hand (rects and anchors, a row's placement, alignment, force expand
  and shrinking, uncontrolled children and child scale, ignored and
  inactive children, a fitter around a group, priority and the largest
  value with images sliced and simple, unknown text, layout roots,
  aspect fitters, unported components); 1 for `interfaces`.
- Not compared with the game on screen (M9u5).

## 2026-10-13 — M9u3: TMP fonts made from the game's font files

**What/why:** the M9u plan's fourth step. Every TextMesh Pro font asset
of the game is dynamic, so the game makes each glyph from the font file
while it runs. We do the same, so M9u4 can lay out text and M9u5 can
draw it. New:
- crate `sn-ui` (layer 2, pure), module `font`: `FontFile` (face
  metrics, glyph metrics by FreeType's unhinted fixed-point scaling,
  composite glyphs placed by FreeType's offset rule), our own coverage
  rasterizer and distance field, and `DynamicAtlas` / `Packer`
  (MaxRects, best short side fit, as TMP packs). **New dependency:
  `ttf-parser` 0.25** (TrueType and CFF tables and outlines; pure Rust,
  no dependencies; already in `Cargo.lock` through Bevy; approved in
  the M9u plan). Default features off but `std`.
- `sn-unity::Font` (class 128); `sn-assets::fonts` (`font_assets`: the
  font assets typed with `stored()`, sprite assets, `TMP Settings` from
  `resources.assets`). `sn-assets` now depends on `sn-ui` for the font
  types.
- `sn-inspect ui --fonts [--atlas]` (atlases as PNGs in `out/ui/fonts/`,
  not committed); `code --il ASSEMBLY:TYPE METHOD` reads
  `Managed/ASSEMBLY.dll` (used for TMP's lookup order).
- `docs/formats/ui.md` § Fonts rewritten with the rules found.
`.gitignore` unchanged (no new file types).

**Dead ends / corrections:**
- First run: `…` was 0.27 px too wide. It is a composite of three
  periods. `ttf-parser` adds component offsets in font units; FreeType
  scales each offset and, with `ROUND_XY_TO_GRID`, rounds it to the
  pixel. Our own composite reader fixed it (exact; texel error 1.38 →
  1.04/255).
- My check assumed no asset had multi-atlas on; the CJK asset has it.
  The check was wrong, not the data. CJK also has padding 5 and point
  size 45, not the plan's "padding 9".
- 3 private-use characters looked missing. Reading `TMP Settings` and
  both sprite assets: no font or sprite has them. The IL shows the 6
  texts holding them are always replaced before they are shown
  (`HandReticle.UpdateText`, `uGUI_Toolbar.UpdateCycleBtnText`), so the
  check lists those texts apart and counts the rest.
- Cap and mean lines cannot match without a TrueType hinter. TMP uses
  the stored values while the game runs, so these are reported, not
  counted (12 of 24 differ by under a pixel).

**Verified:**
- `sn-inspect ui --fonts`: `0 checks differ`. Face metrics 136 of 136
  fields equal within 0.01 (8 assets; names exact). Both stored glyphs:
  index, metrics (as `f32`) and glyph rectangles equal; used rectangles
  equal in order, free rectangles equal as sets. Texels over the used
  rectangles: **mean 1.043/255** (target ≤ 3, written in the plan before
  the run), max 13, 60% exact; the no-shift alignment is best (others
  4.4–11.4). English: 118 distinct characters, 0 missing (100 from
  Aller, 18 from Noto Sans CJK SC). 141 screen texts, 0 missing outside
  the 6 replaced ones. Cost: 118 glyphs in 262 ms, 2.2 ms each
  (release).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored fonts_remade`: 1 passed.
- `cargo test --workspace`: 519 passed, 0 failed, 43 ignored. Clippy and
  fmt: pass. 18 new unit tests (`sn-ui`: 17 on a TrueType font written
  in the test, `sn-unity`: 1).
- Not drawn yet (M9u5); not compared with the game on screen.

## 2026-10-12 — M9u2: UI built-in classes, sprites and item icons

**What/why:** the M9u plan's third step. The HUD, the PDA and the
fabricator draw sprites from the game's atlases. Item icons are looked
up by `SpriteManager`'s rule, which this step ports. New:
`sn-unity::ui` readers (`RectTransform`, `CanvasGroup`,
`CanvasRenderer`, `Sprite`, full `SpriteAtlas`); `sn-assets::sprites`
(`Sprites`, `sprite_pixels`, `cut`); `sn-inspect ui --sprites
[--icons]`, `apps/sn-inspect/src/png.rs` (our own minimal PNG writer),
`code --fields` listing methods; `docs/formats/ui.md` § Sprite atlases.
No new dependencies; `.gitignore` unchanged (icons go to `out/ui/`).

**Dead ends / corrections:** a first oracle script searched every
bundle for the external file and did not find it (the names are lower
case, and the search was slow). It now takes the bundle our tool names
(`duplicateassetssorted_assets_bundle93_…`). My real-data test assumed
the sprite was named `Titanium`; the game stores `titanium`. That is why
the game's lookup ignores case.

**Verified:**
- `sn-inspect ui --sprites --icons`: `0 checks differ`. Sprite 1,190,
  SpriteAtlas 8, RectTransform 2,835, Canvas 138, CanvasGroup 97,
  CanvasRenderer 2,306 parsed to their last byte, 0 not. 8 atlases by
  the label, 470 of 470 `Items` sprites resolved, 0 misses,
  `defaultSprite` found. 515 icons written to `out/ui/icons/` (not
  committed).
- Oracle: 20 of 20 `Items` sprites equal UnityPy's as `f32`; both
  `Items` pages' RGBA CRC-32 equal UnityPy's (`b4bde8b7`, `d521bff4`).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  -- --ignored sprites_by ui_trees script_layouts`: 3 passed.
- `cargo test --workspace`: 501 passed, 0 failed, 42 ignored. Clippy and
  fmt: pass.
- Not compared with the game on screen. Committed with the user's OK.

## 2026-10-12 — Fix: crash after picking up an item (M9d4b bug)

**What/why:** the user's game crashed with Bevy's "Entity despawned …
If you were attempting to apply a command to this entity" panic,
seconds after picking things up and swimming on. Cause: `hide_keys`
(M9d4b) despawned a picked-up item's entities, but left them in their
batch's `shown` lists. When the batch later left LOD 0, the shadow
switch inserted `NotShadowCaster` on them; when the batch unloaded, it
despawned them again. A command on a despawned entity panics. Fix:
`hide_keys` takes the despawned entities out of every batch's `shown`
lists (`forget_despawned`). Not from M9u: M9u0/M9u1 do not touch the
client.

**Verified:**
- Reproduced on the old code (the fix stashed), release client:
  `sn-client --start -116.24 -8.89 -50.65 --resolution 960 540
  --pick-up 20 --hold-forward 22 --shot 110 m9u-crash-before` picked up
  an acid mushroom at 20 s, then panicked about 28 s later with the
  user's error (`Entity despawned: The entity with ID 18829v0 is
  invalid`), exit 101.
- The same command with the fix (`m9u-crash-after`): ran the full 110
  s, swam about 350 m past the item's batch, screenshot saved, exit 0,
  no panic or `Entity despawned` in the log.
- New unit tests: `picked_up_entities_leave_their_batch` (the pick-up,
  then the shadow switch and the unload on a real Bevy `World`) and
  `a_command_on_a_despawned_entity_panics` (should_panic: the failure
  the fix removes).
- `cargo test --workspace`: 497 passed, 0 failed, 41 ignored. Clippy
  and fmt: pass. Debug and release clients rebuilt.
- Committed with the user's OK.

## 2026-10-12 — M9u1: script layouts from the game's assemblies

**What/why:** M9u needs dozens of MonoBehaviour layouts (Image, TMP,
layout groups, every `uGUI_*`). We read them from the field metadata of
the game's own DLLs, by Unity's serialization rules, rather than
writing each by hand (`docs/DESIGN.md` "M9u plan" and "M9u as built",
`docs/formats/ui.md` § Script fields). New:
- `sn-dotnet::meta` (type flags, `extends` with generic bases, nested
  types, TypeRef scopes, assembly names and refs, custom attributes).
  `SigType` now carries tokens (`NamedType`, `TypeToken`).
- `sn-dotnet`'s test encoder behind an `encode` feature. No new
  external dependency: `sn-assets` uses it as a dev-dependency feature
  for its synthetic tests.
- `sn-unity::script` (`Kind`, `StructLayout`, `read_script`,
  `trace_script`).
- `sn-assets::script_layout` (`ScriptAssemblies`, `Layouts`).
- `sn-inspect ui --scripts [--all]` and `ui --layout <CLASS> [<NAME>]`.
- `.gitignore` unchanged.

**Dead ends / corrections on the way (each found by an object not
ending at its last byte):** `GUIStyle` was skipped as a non-serializable
class. It is stored natively, so it was added to the built-in table and
checked on `UWE.LargeWorldStreamerGUI` (348 bytes) and `Player` (1,204
bytes, 41 fields). Strings in arrays were not padded
(`Story.CompoundGoalData`, error at byte 1,835); they now pad to the
object's 4-byte boundary. After those two fixes, nothing was left to
fix. My first synthetic test for padded strings in arrays assumed the
padding is relative to the string. It is absolute; the test was fixed.

**Found:** every TMP font asset is dynamic (SDFAA, padding 9). Only
`Aller_Rg SDF` stores glyphs, and just 2. M9u3's check is revised in
its row (face metrics against each asset's stored `m_FaceInfo`, and the
2 stored glyphs).

**Verified:**
- `sn-inspect ui --scripts`: 2,197 of 2,197 (the UI and startup files,
  410 classes). `ui --scripts --all`: 5 standalone files and 5,467
  bundles, **45,137 of 45,137 MonoBehaviours, 1,205 script classes, 0
  misses** (7.8 s warm; 91.5 s cold). Output in
  `out/m9u/scripts-all.txt` (gitignored).
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  script_layouts -- --ignored`: 1 passed, 9 objects compared.
- Unit tests: `sn-dotnet` meta 1, `sn-unity` script 2 (every kind,
  string arrays, truncated, too long, huge counts), `sn-assets`
  script_layout 4 (Unity's rules, skipped types, depth limit, generic
  bases, missing types, reading by a built layout).
- `cargo test --workspace`: 495 passed, 0 failed, 41 ignored. `cargo
  clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p
  sn-dotnet --features encode -- -D warnings`, `cargo fmt --all --
  --check`: pass.
- Committed with the user's OK (pushed together with the crash fix).

## 2026-10-12 — M9u0: the game's UI, census

**What/why:** the first step of the M9u plan. We need to know where the
UI is and what it holds before we read or draw it. New:
- `sn-dotnet::sig`: field type signatures (`SigType`, `field_type`,
  `base_type`), the base for M9u1's script layouts.
- `sn-inspect code --fields <TYPE>`.
- `sn-unity::ui`: `Canvas` (every field, ends at its last byte),
  `uGUI_CanvasScaler`'s five serialized fields, `SpriteAtlasHead`.
- `sn-assets::ui`: `UiTree` (every node with path, active state, layer,
  components and script names), `ugui_root` (the catalog key
  `uGUI.prefab`), `pda_screen_refs` (the main scene's `PDA` fields).
- `sn-inspect ui` and `docs/formats/ui.md`.

No new dependencies. `.gitignore` is unchanged (`.rs`/`.md` already
whitelisted).

**Found:** the plan's counts were bundle totals. `uGUI` itself is 361
nodes and 12 canvases, plus 10 small prefabs the scripts instantiate
(26 nodes). 11 canvases are world space; scalers are 1 m in front of
the UI camera (DepthClear 1000 m) at 1920×1080. The PDA screen is
`uGUI_PDAScreen` (402 nodes, scaler mode Inversed). All 470 `Items`
sprites resolve. Every TMP font asset is under 1.1 KB, so its glyph
tables are probably empty (hypothesis), a risk for M9u3's check
(DESIGN "M9u as built").

**Verified:**
- `sn-inspect ui` (real data): `0 checks differ (0.73 s)`, 23 checks
  (output in `out/m9u/ui-census.txt`, gitignored).
- `cargo test -q -p sn-dotnet sig`: 3 passed (primitives, classes,
  arrays, generics, custom modifiers; bad and too-deep signatures are
  errors). `sn-unity` `ui`: 4 passed (canvas, truncated and too-long
  canvases, scaler, atlas head, huge count). `sn-assets` `ui`: 2 passed.
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets --test real_data
  ui_trees -- --ignored`: 1 passed (361 / 402 nodes, hosts, paths).
- `cargo test --workspace`: 488 passed, 0 failed, 40 ignored.
  `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt
  --all -- --check`: pass.
- Committed with the user's OK.

## 2026-10-12 — M9u plan: the game's UI

**What/why:** M9d is done, so M9u's plan is written before any code
(`docs/DESIGN.md` "M9u plan", steps M9u0–M9u9; the M9u row points to
it). Docs only. Approved by the user the same day.

**Found while planning (read only, names and counts):** the HUD is
`uGUI.prefab` (13 canvases, 12 of them in world space, 504 behaviours);
the PDA's screens are in `main.unity`'s `sharedAssets`; 8 sprite atlases
(`Items` packs 470 sprites and its page is in another bundle); 9 fonts,
only `Aller_Rg`'s TMP atlas is stored and the rest are filled while the
game runs; `uGUI_CanvasScaler` places canvases at the UI camera +
forward × `distance`. Tools: `sn-inspect unity --types`, `code --il`, `code --enum`, and
a throwaway probe in `out/probe` (gitignored). Proposed dependency for
M9u3: `ttf-parser` (TrueType/CFF outlines, pure Rust); not added yet.

**Verified:** `git status --short` lists only `docs/DESIGN.md` and
`MODLOG.md`. No code changed, nothing built or tested.

## 2026-10-12 — M7v1 crash correction: retain CPU meshes for late reflections

**What/why:** the user reported a panic after about 73 s from Tree Cove:
`ExtractedToRenderWorld` at Bevy's `Mesh::indices_mut`. This was introduced
by my uncommitted winding fix. Non-skinned object meshes used `RENDER_WORLD`
only, so a first reflected placement appearing after GPU extraction cloned
a mesh whose indices were already gone. The initial short, settled GPU
runs and synthetic meshes using default CPU retention missed this ordering.

The production object mesh builder now retains `MAIN_WORLD | RENDER_WORLD`
for original meshes, allowing later lazy variants. Skinned meshes already
retained CPU data for their bounds. The reflection helper uses fallible
index access and returns unsupported for unavailable data instead of
panicking. Added a regression using the actual builder and the GPU
extraction lifetime decision before a delayed reflected instance, plus
an already-extracted-data rejection test. No dependencies, whitelist,
collision, architecture or game-install changes. Work remains uncommitted.

**Verified:**
- `cargo test -p sn-client sidedness`: 5 passed. The new delayed test
  checks that the variant reverses indices and the original stays intact
  after extraction and 32 updates before the reflected instance appears.
  The unavailable-data test calls `Mesh::take_gpu_data` and verifies
  rejection without a panic.
- `cargo test --workspace`: 479 passed, 39 ignored, 0 failed.
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo build --release -p sn-client`,
  `git diff --check`: pass. Default release executable updated.
- Long real-data flythrough from the user's Tree Cove start, 960×540,
  view 400 m, speed 5 m/s, 120 warm-up frames: 167.0 s traversal,
  170 s session, 15,051 measured frames; destination settled 0.02 s after
  arrival, clean exit 0. No panic, `ExtractedToRenderWorld`, GPU validation
  error or unsupported mesh in the log. Initial area's cache 244 variants
  grows to 251; final 408 reflected instances, 0 unsupported. 336 terrain
  batches streamed during travel across all LODs (8/34/73/221).
- Mean frame time 11.09 ms, p95 17.51 ms, worst 145.17 ms; peak sampled
  process memory 1.73 GiB, peak 761 terrain batches cached. CPU retention
  costs memory by design; this route is not an isolated memory comparison
  and no speedup is claimed. Session 170 hash checks, 0 differences.
  Logs, test outputs and ignored capture: `out/m7v1-crash/`;
  numerical assertions in `verification.log`.

**Reproduce:** set `SUBNAUTICA_DIR` to the player's install, build release:

```powershell
target/release/sn-client.exe --flythrough -450 -250 300 --speed 5 --view 400 --resolution 960 540 --warmup 120 --start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6 --slot-seed 1 --lifepod-seed 1 --time 9.6 --time-scale 0 --gpu-timings
```

Expect arrival after about 167 s, `settled: true`, `0 unsupported` in
every face-orientation audit, no panic and exit 0. The user's original
interactive command can also be retried with the rebuilt executable:
`target/release/sn-client.exe --game-dir <INSTALL> --free-cam --start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6 --view 400`.

**Corrections/limits:** initial compilation of the new regression needed
a mutable binding for Bevy's mesh asset guard; corrected, then all checks
rerun and passed. The earlier M7v1 short GPU runs did not establish long
streaming stability; this reported failure and its correction supersede
that assumption. User interactive replay, visual wall acceptance, isolated
memory overhead and Linux/macOS are **not tested**. No commits or pushes.

## 2026-10-12 — M7v1: reflected alien walls and complete face-cull states

**What/why:** user approved M7g4c: committed/pushed `ec5d16d`, remote
hash verified. Then the user reported walls visible only from one side
throughout large alien structures, clarified as visual disappearance.
This new M7v1 work is uncommitted. Collision is not changed.

Confirmed two renderer gaps: material culling lost Front and shader
fixed/default states; negative world determinants reversed face winding
without compensation. Installed-data audit finds mirrored Precursor wall
pieces, notably the prison aquarium. The object loader now keeps Off/Front/
Back from the shader pass and its property/default. New `sidedness.rs`
caches reversed-index mesh variants for reflected object instances after
transform propagation; positive transforms restore the original handle.
All mesh attributes remain intact. The custom brine buffer corrects its
facing test separately. No blanket two-sided override, architecture change,
dependencies or whitelist changes. Game install read-only; probes/captures/
third-party reference source remain in ignored `out/m7v1/`.

**Verified:**
- `cargo test --workspace`: 477 passed, 39 ignored, 0 failed. New tests:
  shader fixed/property/default Cull precedence and all three modes;
  triangle/normal agreement for all eight scale-sign combinations;
  U16/U32 reversible winding and preserved original geometry; a headless
  ECS sign-change sequence restores the original handle and reuses one
  cached variant without mutating the original.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo build --release -p sn-client`,
  `git diff --check`: pass. Default release executable updated successfully.
- Read-only UnityPy census: 1,770 reflected local transform records,
  1,636 with Precursor-named ancestry (records across bundles, not distinct
  placed objects). Name/shader-ID material census: Off 302, Front 4,
  Back 2,855, missing saved property 351. Precursor-named subset: Off 34,
  Back 648, missing 3. Marmoset default Cull 2. Thus Front loss is a real
  general bug, while reflected transforms are the main supported hypothesis
  for the user's alien-wall symptom. Bevy 0.19.1 source inspected from its
  official crate archive as reference; no code copied.
- Six sequential 1600×900 GPU runs, view 400 m, 120 settled warm-up and
  300 measured frames each. Reports/CSV/captures:
  `out/baselines/run-m7v1-{gun,aquarium,cove}-{before,after}/`.
  No GPU validation errors, loader warnings or unsettled frames. Geometry,
  loaded object/material/texture counts, terrain, sky and light counts equal
  in each pair. Shader state diagnostics (object mesh instances, including
  loaded hidden parts): Gun 1,160 Off / 0 Front / 19,566 Back, 4,387
  reflected instances, 1,551 cached variants; aquarium 482/0/6,971,
  1,518 reflected, 715 variants; Cove 202/0/5,436, 390 reflected,
  244 variants. Unsupported meshes 0 in all three. Cove exercises brine.
- Mean before → after: Gun 18.975 → 18.203 ms (p99 33.544 → 30.716),
  aquarium 14.390 → 12.967 ms (p99 28.630 → 22.743), Cove
  8.603 → 8.858 ms (p99 13.780 → 11.930). Single pairs do not establish
  a speedup or meeting the 16.7 ms target. Cached mesh copies cost memory:
  peak sampled process GiB Gun 1.854 → 2.001, aquarium 1.366 → 1.389,
  Cove 1.213 → 1.202 (process measurements include unrelated loading costs).
- Control binary matches the previous M7g4c final executable, source
  baseline `ec5d16d`; both binary hashes and report checks are in
  `out/m7v1/verification.log`. User's previously running client was already
  closed before building and benchmarking; no user process was stopped.

**Reproduce:** set `SUBNAUTICA_DIR` to the player's install, build release,
then use a fresh baseline name:

```powershell
target/release/sn-client.exe --benchmark 300 --baseline m7v1-check --baseline-revision local-M7v1 --warmup 120 --resolution 1600 900 --start 356 -68 1075 --look 360 -71 1083 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expect `objects: face orientation` with reflected instances/reversed mesh
variants and `0 unsupported`, zero loader warnings, 300 samples/report/
capture and clean exit. With fresh names, repeat for the aquarium at
`--start 325 -1600 -455 --look 345 -1600 -455`, or Cove at
`--start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6`.
Human check: replace benchmark flags with `--free-cam` and inspect both
sides of the reported walls. Shader-requested one-sided surfaces remain
one-sided; this does not invent missing exterior geometry.

**Dead ends/limits:** initial suspicion included invisible collision blockers;
user clarified rendering only. Cull state loss alone did not explain the
broad Precursor symptom (no Front in the Precursor-named material subset),
so the transform audit identified the reflected walls. No random shader
variants tried. **Not tested:** user visual acceptance, matched original-game
images, keyboard movement/collision regression, Linux/macOS. Additional
missing exterior geometry, material variants or activation rules remain
hypotheses for any walls still missing after this fix.

## 2026-10-12 — M7g4c: control-room UV deformation and rigid effect ownership

**What/why:** user approved committing/pushing M7g4b; commit `92bf67f`
was pushed to `origin/main` and the remote hash verified. Continued with
the next scoped M7g4 slice; this new work remains uncommitted.
The Gun column force field and two terminal material instances now select
their compiled UV deformation sets, with optional refraction and no soft
fade. Existing WGSL already implements their confirmed formulas. Exact
keyword/state guards remain; missing active refraction maps are rejected
because the current white fallback does not implement the declared bump
default. No dependency, architecture or whitelist changes.

The GPU count check exposed rigid effects losing animated-node ownership.
The loader now retains their rig reference, and spawning uses the rig node
as parent with identity local transform, as the ordinary object path does.
Five attachments are logged in the control-room case. Format docs, roadmap
and README updated. All extraction/captures remain ignored; install read-only.

**Verified:**
- `cargo test --workspace`: 473 passed, 39 ignored, 0 failed, including
  synthetic tests for deformation independently of soft fading, optional
  refraction, scalar defaults/overrides, missing second main map, missing
  active refraction map, unsupported sets and incorrect culling.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `git diff --check`: pass.
- `cargo build --release -p sn-client --target-dir out/m7g4c/target`: pass.
  Default output was locked by the user's running client; that process was
  left running. Copied the existing release dependency cache inside ignored
  `out/` and built to the separate target directory.
- Installed D3D11 programs and shader defaults checked with UnityPy; Rust
  `sn-inspect prefab ... --props` checks both control-room prefabs. The
  selected programs have no scene-depth soft-fade sampler or vertex near
  fade. Research/verification logs: `out/m7g4c/`.
- Final pairs, after then before, 1600×900, view 400 m, 120 settled warm-up
  frames and 300 measured frames: no GPU errors, loader warnings or
  unsettled frames. Reports/CSV/captures:
  `out/baselines/run-m7g4c-final2-{column,door}-{before,after}/`.
  Three control-room materials replace fallback; effects 73 → 76 parts.
  Control room retains 102 batches/6,815 meshes/857 materials/1,064 textures;
  tracked top-level entities 20,676 → 20,672 (four formerly independent
  effects now owned by rig nodes, plus one newly ported rigid mesh already
  owned by its rig on the ordinary path). No geometry was removed. Global
  opaque sky cache loses three converted material instances (567 → 564).
  Door: unchanged 8,202 entities/1,835 meshes/336 materials/645 textures.
  Terrain, sky and light counts equal in both pairs.
- Final mean frame times before → after: control room 17.223 → 16.841 ms,
  p99 20.697 → 20.375 ms; door 8.049 → 8.072 ms, p99 11.346 → 12.689 ms.
  Recent effects GPU: control room 0.0587 → 0.0596 ms; door
  0.0876 → 0.0877 ms. The user's other client shared the GPU throughout;
  these are contended runtime checks, not isolated performance evidence.
  Neither a speedup nor meeting the 16.7 ms target is established.
- Archived control binary hash matches the previous M7g4b final executable;
  source baseline is `92bf67f`. Both binary hashes and numeric checks are
  in `out/m7g4c/verification-final.log`.

**Reproduce:** set `SUBNAUTICA_DIR` to the player's install, run the build
command above and use a fresh baseline name:

```powershell
out/m7g4c/target/release/sn-client.exe --benchmark 300 --baseline m7g4c-check --baseline-revision local-M7g4c --warmup 120 --resolution 1600 900 --start 356 -68 1075 --look 360 -71 1083 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expect the column force field and both terminal materials as effect
materials with `deform: true, soft_edges: false`, five rigid attachment
lines, zero warnings, 300 samples/report/capture and clean exit. For the
door regression use another fresh name and
`--start -622 -558 1482 --look -608 -561 1483`.

**Dead ends/corrections:** default release build failed with Windows access
denied because the running client locked its executable; isolated build
resolved this. Initial control-room pair was 12.100 → 17.835 ms; reverse-order
final checks did not reproduce that gap. Initial entity count rose by one.
The first attachment correction addressed spawning only; a rerun showed
the loader still explicitly cleared effect rig references. Correcting both
paths yielded the five logged attachments and the explained ownership
counts above. Earlier `run-m7g4c-*` and `run-m7g4c-final-*` reports precede
that complete fix and are not final acceptance. A repeat launcher initially
had a trailing array comma; it did not run. The report checker was adjusted
to distinguish tracked handles from geometry and to match `entities` without
also modifying `scene_entities`.

**Not tested:** original-game matched images, terminal scripted colour
activation, keyboard play, isolated GPU timings, Linux/macOS. Terminal
stored colours are zero; shader selection alone does not activate them.
The column's back-culled inner layer and elevator/tech-cone/other M7g4
sets remain rejected. Camera-selected fog and refraction Y sign remain
limitations. No commits or pushes of this new M7g4c work.

## 2026-10-12 — M7g4b: Lost River brine saved-keyword selection

**What/why:** after M7g5 and M7g4a, inspected the presumed lit lake
variant. Fresh installed shader metadata (636 vertex + 636 fragment
entries) contains neither `FX_LIGHT_2SIDED` nor `FX_LIGHT_NORMALMAP`.
Ghost Tree/Bones Field materials save these keywords but have light mode 0;
their active set selects Tree Cove's vertex/fragment pair, without normal
sampling or surface lighting. The previous "lit brine" description was an
incorrect hypothesis. Added only that exact saved set to the existing effect
selector; actual lighting modes, unknown sets and wrong culling remain
rejected. No WGSL, dependencies, architecture or whitelist changes.
Format findings, roadmap and README updated; game install read-only.

**Verified:**
- `cargo test --workspace`: 472 passed, 39 ignored, 0 failed. Added synthetic
  coverage for the saved set's equality with the unlit program's inputs,
  inactive lighting properties, reordered keywords, partial sets, real
  lighting selectors and incorrect culling.
- `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo build --release -p sn-client`,
  `git diff --check`: pass.
- Fresh shader dump matches the earlier fragment disassembly by SHA-256.
  Rust `sn-inspect prefab ... --props` checks Ghost Tree lake and Bones
  Field cave pool properties. Ignored evidence: `out/m7g4-brine/`.
- Six sequential 1600×900 runs, view 400 m, 120 settled warm-up frames,
  300 measured frames each: no GPU errors, loader warnings or unsettled
  frames. Reports, CSV samples and captures:
  `out/baselines/run-m7g4b-{ghost,cove,precursor}-{before,after}/`.
  Ghost Tree: both lake material families change from fallback to
  `brine: true`; 2,113 entities/150 batches/1,816 meshes/309 materials/665
  textures unchanged. Tree Cove: 6,335 entities/1,301 meshes/266 materials/
  548 textures unchanged. Precursor: 8,202 entities/1,835 meshes/336
  materials/645 textures unchanged. Terrain, sky and light counts equal.
  The opaque global sky-material cache loses two fallback materials in
  Ghost Tree/Cove; Precursor's cache is unchanged (no lake parts spawned).
- Mean frame times before → after: Ghost Tree 5.939 → 6.152 ms; Tree Cove
  6.340 → 6.256 ms; Precursor 6.666 → 6.631 ms. P99 respectively
  8.978 → 13.459, 10.500 → 12.340, 11.028 → 10.132 ms. Ghost Tree adds
  a 0.026 ms effects GPU pass; Cove 0.057 → 0.056 ms; Precursor
  0.085 → 0.085 ms. Single pairs do not establish a speedup or isolate
  the cost of the change. Animation remains live.
- Control is the executable archived before editing, not a fresh isolated
  source rebuild. Binary hashes and report checks are recorded in
  `out/m7g4-brine/m7g4b-verification.log`; initial source HEAD was `adfc885`.

**Reproduce:** set `SUBNAUTICA_DIR` to the player's install, build release,
use a fresh baseline name (existing names are rejected):

```powershell
target/release/sn-client.exe --benchmark 300 --baseline m7g4b-check --baseline-revision local-M7g4b --warmup 120 --resolution 1600 900 --start -954 -643 989 --look -954 -653 1009 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expect both Lost River lake materials logged as effect materials with
`brine: true`, zero warnings, 300 samples, report/capture and clean exit.
For regressions use fresh names with
`--start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6` (Cove), or
`--start -622 -558 1482 --look -608 -561 1483` (Precursor).

**Dead ends/corrections:** no normal-map lighting was added because the
installed program does not contain it. The first benchmark completed, but
the PowerShell launcher read a null exit code after `WaitForExit`; changed
to `Start-Process -Wait` and retained its completed report. The report
checker initially expected identical sky-material cache counts; corrected
the expectation to account for fallback materials leaving the opaque cache
only where their parts are spawned. No implementation failures or random
shader variations.

**Not tested:** matched original-game imagery, runtime keyword changes in
the original game, keyboard play, Linux/macOS. Camera-based fog-volume
selection remains a limitation. No commits or pushes.

## 2026-10-12 — M7g4a: unlit Tree Cove brine

**What/why:** the exact Tree Cove UBER variant now uses the effect pass,
with world-space triplanar scrolling/deformation, vertex near fade, back-face
rejection, ordinary alpha and saturated colour before WBOIT. The shader still
applies fog without `FX_ADDFOG`, as confirmed from its compiled program.
The five previous variants keep their existing behaviour/state checks.
Lit brine and other keyword combinations remain explicit fallbacks.
Inventory/death-drop work at `95c1883` is preserved. No dependencies or
whitelist changes; no game data added to source/docs.

**Verified:**
- `cargo test --workspace`: 471 passed, 39 ignored, zero failures. The new
  synthetic material test covers projection defaults/overrides, all three
  scroll components, clip parameters, culling and rejection of lit variants.
  After strengthening the Z-scroll assertions, rerun with
  `cargo test -p sn-client brine_preserves_projection_vectors_and_requires_its_decoded_state`
  (expect one pass). `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo build --release -p sn-client` pass.
- Fresh release control built from `95c1883`; before/after executables and
  logs are ignored under `out/m7g4-brine/`. At Tree Cove, 6,335 entities,
  150 object batches, 1,301 meshes, 266 materials and 548 textures remain
  equal; zero loader warnings and unsettled measurement frames. The brine
  material changes from the logged fallback to `brine: true`; lit lake and
  lava variants remain rejected. One fewer global fallback material instance
  is expected. Both final GPU paths compile without validation errors.
- Final sequential Tree Cove pair (`run-m7g4a-cove-{before,after}-2`):
  mean 5.281 → 6.503 ms; p99 6.091 → 10.854 ms. New effects GPU pass
  0.057 ms. This pair is slower overall; it does **not** establish a speedup
  or isolate the CPU cost. Initial Cove control overlapped compilation and
  was unsuitable for timing comparison, so it was repeated without builds.
- Precursor pair (`run-m7g4a-precursor-{before,after}`): 8 UBER effect
  materials selected, identical 8,202 entities/140 batches/645 textures,
  zero warnings or unsettled frames. Mean 5.950 → 6.192 ms; recent effects
  GPU 0.084 → 0.085 ms. Captures are 1600×900. Animation remains live;
  these are runtime regressions, not pixel-exact reference comparisons.

**Reproduce:** set `SUBNAUTICA_DIR` to the player's install, build release,
then use a fresh baseline name:

```powershell
target/release/sn-client.exe --benchmark 300 --baseline m7g4a-check --baseline-revision local-M7g4a --warmup 120 --resolution 1600 900 --start -931.2 -931.6 326.6 --look -931.2 -941.6 346.6 --fov 60 --view 400 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
```

Expect `x_LostRiver_TreeCoveLakes` logged as an effect material, `brine: true`,
a report/capture in `out/baselines/run-m7g4a-check`, and clean exit. Repeat
with another fresh name and `--start -622 -558 1482 --look -608 -561 1483`
for the Precursor regression.

**Corrections/limits:** initial extra projection vectors made `EffectLook`
trigger clippy's large-enum warning. Reused mutually exclusive ST/projection
slots and packed Z speeds instead; clippy passes without a lint suppression
or per-instance heap allocation. An exploratory far-west lake camera also
ran; the documented acceptance camera uses a central Tree Cove placement.
Original-game visual parity, interactive inventory play and Linux/macOS:
**not tested**. Camera-based fog-volume selection remains a known limitation.
Next: lit Lost River brine's normal maps and two-sided lighting.

## 2026-10-12 — M9d6c: two players, and the M9d6 summary

**What:**
- `objects.rs`: a server-made object's "drawn" line gives the world
  event at which it was asked for and the drawn count; its removal is
  logged with the event and the count (to compare two clients' logs).
- `session.rs`: a last session report on exit (the periodic one is every
  60 s, longer than these runs), with the server's hash checks.
- README: "Play together" lists items as synced.

**Why:** M9d6c of the M9d6 plan; with a, b and c M9d6 is done.

**Verified:**
- Host `sn-client --view 400 --resolution 960 540 --host 27431 --name
  host --use-item 3,26 --panel 6=take-all,7=close,24=drop --use-hatch 8
  --kill 30 --shot 50 m9d6c-host`, 8 s later joiner `--join
  127.0.0.1:27431 --name joiner --shot 44 m9d6c-joiner`
  (`out/m9d/m9d6c_{host,joiner}.log`). On both, the same lines: object 4
  drawn, asked at world event 18, 1 drawn; removed at event 19, 0 drawn
  (the host's hand picked it up from 1.01 m); objects 0, 1, 2, 4, 5
  drawn, asked at event 23, 5 drawn (the host's death drops). Host
  report: 100 hash checks, 0 differ; 53 and 49 hash reports.
- `sn-server --port 27778` + `sn-inspect coop 127.0.0.1:27778 --count 4
  --presets lan,bad --seconds 20` (`out/m9d/m9d6c_{server,coop}.log`):
  lan 18,252/18,252 exact; bad 18,025/18,060 (99.806 %, target 99 %,
  max error 0.032 m); 252 hash checks, 0 differ.
- Solo `--hatch-name top --use-hatch 3,15,27 --hold-forward 31 --kill 40
  --shot 45 m9d6c-solo`, compared with `out/m10g/cmp.py` to M10g's
  `n1.txt`: 453/453 states equal, step shift 0 on every line, 15/15
  events equal.
- `cargo test --workspace` (with M9d6a and b): 470 passed, 0 failed, 39
  ignored; clippy `-D warnings` and fmt pass.

**Found:** the first two-client run had no hash count in the logs (runs
under 60 s print no report), hence the report on exit; its joiner logged
"lost the connection (TimedOut)" because the host quit first (expected;
the rerun let the joiner quit first).

**Not tested:** two machines; two players at the locker at once; another
player's drops dropped while that player's state overtakes the `Lose`
request over UDP (the server takes "in the pod" from the latest state:
in solo the request comes first; not seen in these runs).

## 2026-10-12 — M9d6b: `sn-inspect pickup`, the items headless

**What:**
- `sn-inspect pickup [--seed N]` (`pickup.rs`, new): `walk`'s run with
  the vitals and the item targets, an in-process server and client
  (`sn-session`, a direct in-memory link on a simulated clock; the run's
  steps go to the server as the client's states, one tick each, before
  each request). Steps: the hand on the locker and every supply out
  (left clicks by `item_action`), out of the pod, the nearest pickup, the
  nearest outcrop hit until it breaks and its resource, a drop
  (`drop_at`) picked up again at once, a death (`death_drops` from the
  camera at `MoveToRespawn`, sent before the respawn's state). Each step
  logs the grid with cells, the slots, the locker, the server-made
  objects and the gone keys; 14 checks set the exit code.
- `collision.rs`: `Streamed` keeps gone keys out of loads, `target`,
  `targets`, `targets_of`, `follow_items` (bodies follow the shared
  items, as the client's worker). `BatchBodies::targets`. `walk.rs`:
  `look_at`, `eye`, `look` visible; `death_camera` (the camera before
  the respawn's move). `coop::to_state` shared.
- No new dependency or file type.

**Why:** M9d6b of the M9d6 plan.

**Verified:**
- `sn-inspect pickup --seed 1|2|3` (`out/m9d/m9d6b_seed{1,2,3}.log`):
  14 checks, 0 failed, each; 0 requests refused; client and server world
  hashes equal; 0 penetrations, 0 surfaces passed through. Seed 1: locker
  from 1.95 m; acid mushroom at 17.4 m; limestone outcrop 3 hits → copper
  ore; the drop 1.30 m from the camera, picked up from 0.94 m; the death
  dropped 7 of 8, kept copper ore; 1,317 steps in 4.3 s. Seeds 2 and 3:
  resources titanium and copper ore; 2,904 and 2,857 steps.
- `cargo test --workspace`, clippy, fmt: see the M9d6c entry.

**Dead ends / found:**
1. Seeds 2 and 3: the nearest outcrop (26.8 m, 7.1 m) was not reached in
   30 s: the script swims straight at it and stalled against the terrain
   (0.4–1.9 m/s at the end). It takes the next nearest; a path finder is
   not worth it for a proof.
2. A death drop 2.4 m from the camera on seed 1, past the ray's 1.5 m:
   `FindDropPosition` casts its sphere 5 m, beyond the ray's point, and
   moves the item to the first contact (§ Inventory, ported as read).

## 2026-10-12 — M9d6a: death drops and shared hand code

**What:**
- DESIGN: the M9d6 plan; M9d6 split into a (this), b (`sn-inspect
  pickup`), c (two players). Found while planning b: no client sent a
  death's drops (`Request::Lose`), though the protocol and the server
  have had them since M9d3.
- `sn-session::drops` (new): `Draws` (SplitMix64, `insideUnitCircle`),
  `drop_at` (`InternalDropItem`: `drop_ray`, `find_drop_position`,
  `Random.rotation`), `death_drops` (`LoseItems`' points as millimetre
  offsets from the camera); 2 tests.
- `sn-assets::hand` (new): the hand's `aim` moved from `sn-client`'s
  `items.rs` with its test, so `sn-inspect` can use it.
- `sn-client`: the panel's and `--drop`'s drop use `drop_at`; at the
  death's `MoveToRespawn`, before the teleport, `lose_items` sends `Lose`
  (as `ResetPlayerOnDeath` calls `LoseItems` before
  `MovePlayerToRespawnPoint`).
- `sn-dotnet`: `nested_types`; `sn-inspect code --nested TYPE` and
  `Outer/Inner` for `--il` (to read the death coroutine).
- `gameplay.md` § Death drops; README. No new dependency or file type.

**Why:** M9d6a of the M9d6 plan.

**Verified:**
- `cargo test --workspace`: 470 passed, 0 failed, 39 ignored. Clippy with
  `-D warnings` and fmt pass.
- `sn-client --view 400 --resolution 960 540 --use-item 3 --panel
  6=take-all,7=close --use-hatch 8 --kill 20 --shot 29 m9d6a-death`
  (`out/m9d/m9d6a_death.log`): died on the pod's top at (-126.98, 5.67,
  -49.75); at `MoveToRespawn` "LoseItems: 5 items dropped", offsets
  1.09–1.45 m; request 6 → world event 17 `Lost` with the 5 items; the
  grid then holds Nutrient block #3 only; 5 server-made objects drawn.
- Killed in the pod (`--kill 9`, `out/m9d/m9d6a_death_pod.log`):
  "LoseItems: nothing to drop (in pod true)", no request.

**Not tested:** two players seeing another's death drops (M9d6c).

## 2026-10-12 — M9d5b: the inventory panel

**What:**
- `sn-sim::inventory::actions` (new): `item_actions` / `item_action` /
  `opposite` / `place_of`, ports of `Inventory.GetAllItemActions`,
  `GetItemAction` (keyboard and mouse) and `GetOppositeContainer`; 2 new
  tests. `drop.rs`: `random_rotation` (`Random.rotation`); 1 new test.
  `PickupInfo.randomize_rotation` (`randomizeRotationWhenDropped`), so the
  rules fingerprint's `items` part changed.
- `sn-client` `inventory.rs` (new): our panel (Bevy UI): the locker's
  `EscapePod.StorageOpen` sequence (`storage_open` on the pod's animator
  here and on the drawn rigs, the panel 0.25 s later, ignored while the
  sequence runs; closing clears it); `PDA.Open`'s refusals and its
  distance rule (stand-ins: the locker collider's centre, the player's
  position); Tab toggles, Escape closes, death closes; a left click does
  the item's action as a `Move`, a right click drops (`drop_ray`, then
  `find_drop_position` from the camera, the client's own draws);
  `--panel <SECONDS=STEP,…>` (open, close, take-all, equip, drop); 2 new
  tests.
- `player.rs`: while the panel is open the look, the hand and the cursor
  capture are off; `camera()`, `world()`, `locker_centre()`, `controls()`,
  `set_pod_bool()`; `eye()` removed (its only user was `--drop`'s
  stand-in). `items.rs`: `--drop` uses the real drop point; the locker
  logged after each change. `session.rs`: each request's log line has its
  number, which its world event or refusal carries.
- `sn-dotnet`: `find_method_with` (by parameter count),
  `find_nested_type`, `enum_values_of`. `sn-inspect code --il TYPE
  METHOD [PARAMS]`, `METHOD#N` (the Nth overload), `code --enum TYPE`
  (`Outer/Inner`): needed to read the click rules, the PDA and the
  locker from the DLL.
- DESIGN: the M9d5b plan; `gameplay.md` § The inventory panel and the
  locker; README's items bullet. No new dependency or file type.

**Why:** M9d5b of the M9d plan: the inventory panel and the locker.

**Verified:**
- `cargo test --workspace`: 469 passed, 0 failed, 39 ignored. Clippy with
  `-D warnings` and fmt pass.
- In the pod, `sn-client --view 400 --resolution 960 540 --use-item 3
  --panel 6=take-all,7=drop,9.5=close --shot 9 m9d5b-locker`
  (`out/m9d/m9d5b_locker.log`): the hand on the locker at 1.99 m,
  `storage_open` on (the controller has it), the panel 0.25 s later
  (limit 2.75 m); 6 left clicks, each "switch", requests 0–5, world
  events 2–7 with req 0–5; the locker empty, the grid Flare, Flare,
  Filtered water ×2, Nutrient block ×2 at (0,0)–(5,0). The right click in
  the pod: actions [switch] → none, "no drop in the lifepod".
  `out/m9d5b-locker.png` shows the panel.
- `--use-item 3 --panel 6=take-all,7=close,24=drop --use-hatch 8 --shot 26
  m9d5b-drop` (`out/m9d/m9d5b_drop.log`): closed, `storage_open` off; out
  through `top_out_trigger_first` (8.72 s); the right click: actions
  [drop] → drop; camera (-127.135, 5.728, -49.755), forward (1, 0, 0),
  draws 0.833 and (-0.337, -0.652), ray 1.37 m with no hit, lands there;
  request 6 accepted (world event 15, req 6); server object 4 drawn (2
  entities). `out/m9d5b-drop.png`: the flare ahead, "Pick up Flare".
- `--use-item 3 --kill 6 --panel 20=open,21=close`
  (`out/m9d/m9d5b_death.log`): death closes the panel and the locker;
  after the respawn Tab-style open and close work.
- `sn-server --port 27778` + `sn-inspect coop 127.0.0.1:27778 --count 4
  --presets lan --seconds 20`: 18,252 of 18,252 ticks exact, 124 hash
  checks, 0 differ (`out/m9d/m9d5b_server.log`, `m9d5b_coop.log`).

**Dead ends:**
1. `sn-inspect code --il` took the first method of a name, so
   `ExecuteItemAction`'s second overload (both have 2 parameters) and the
   nested `GameInput.Device` / `Button` enums could not be read; hence
   the `#N` and `Outer/Inner` forms.
2. The distance rule: `--use-item 3 --hold-forward 6`
   (`out/m9d/m9d5b_walk.log`) walked towards the locker (the look is
   frozen while the panel is open), 1.72 m from it at 10 s, under the
   2.75 m limit, so it did not close. **Not tested** in a real run.

**Not tested:** keyboard and mouse play (the clicks on the panel's
boxes, Tab, Escape) by the agent; the distance rule; equip and unequip
with real items (the supplies have no equipment; unit tests only).

## 2026-10-11 — M7g5: SIG coral and capped rock surfaces

**What/why:** replace the flat or incomplete world-object fallback for
`UWE/SIG` and `UWE/SIG Triplanar with Capping`. Load explicit texture slots,
shader defaults and the SIG keyword; implement specular/gloss/emission for
UV SIG, and world-space cap/side colour, normals, tint and specular for
capping. Preserve the compiled cap-only specular edge case and ignore its
unused SIG properties. Reuse our existing terrain projection/border helpers
without changing terrain sampling. Prevent global sky relighting from
turning SIG into Marmoset. Log SIG counts and missing albedo slots; update
README, DESIGN and confirmed format findings. No dependency or whitelist
change; no architecture change. Inline grass-material caches remain bounded
by 256 types, with a documented large-enum lint allowance after the added
object uniforms increased their size.

**Verified:** `cargo test --workspace`: 464 passed, 39 ignored; four new
synthetic tests cover selection, keywords, defaults, texture slots and sky
isolation. `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, release build and `git diff --check` pass.
Six real GPU runs (three alternating controls/changes) completed with no
shader errors, 300 measured frames and zero unsettled frames each. All
after runs: 12 plain/5 capped materials, 7 SIG-enabled, no missing albedo
slots, 23 additional textures; other scene/sky/fog counts match. Means
18.912–19.878 ms before, 18.819–20.422 ms after; opaque GPU pass averages
1.610–1.684 ms before, 1.689–1.824 ms after. No speedup or 60 fps claim.
Exact commands, counts and tail times are in DESIGN's M7g5 implementation
plan. Original-game matched rock/coral comparisons: **not tested**.
Normal play smoke (`--view 400 --resolution 960 540 --time-scale 120
--lifepod-seed 1 --slot-seed 1 --kill 3 --shot 10 m7g5-player`) also exits
cleanly after logging death, restoration and returned controls and saving
the capture. Keyboard play: **not tested**.

**Dead ends/corrections:** the first compile caught the grass fallback's
additional required texture handles; supplied them. Clippy flagged the
enlarged cached-material enum; its bounded inline storage is now documented.
The initial smoke run had elevated costs across unrelated GPU passes, so
fresh alternating controls were built from an archive of current HEAD
`86ee42a`, rather than relying on old M8h measurements. Caller revision
labels mistakenly used the older M8h hash; corrected metadata retains both
labels, and a provenance file records the archive commit and executable
hashes. Reading the disassembly confirmed SIG's missing-map default is
black, and capping does not use its declared SIG maps; neither is treated
as Marmoset's white-map/default-glow behavior.

## 2026-10-11 — M9d5a: the hand on items

**What:**
- `sn-sim`: `OutcropInfo.break_text` (the rules read it; part of the
  rules fingerprint now); `BodyFrame.time_grabbed`, and the body's `grab`
  value is `grab_active` (0.4 s after a grab) instead of always false.
  1 new test.
- `sn-assets`: `ItemText` (tech type names from the DLL, a language file;
  `get`, `tech`, `format` as `Language.Get` / `GetFormat`) and
  `item_text`; 1 new test.
- `sn-client` `items.rs`: `aim` (what the hand shows over a pickup, an
  outcrop or the locker, and the request a click sends), the own inventory
  and unlocks logged after each change, the `Text` resource; 1 new test.
- `player.rs`: the hand's ray every frame (also for the HUD); a click on an
  item sends its request and sets `timeGrabbed`; the locker's bodies are in
  the hand's world (`LOCKER_BODY`) and its click is counted
  (`locker_opened`); `--use-item <SECONDS,…>` walks to the nearest target
  and uses the hand when its ray reaches it.
- `hud.rs`: the hand's text and subscript, centred. `main.rs`: reads the
  item text (English) at start; the flag.
- Real-data test `hand_targets_follow_the_items` also prints an outcrop
  near the pod (used for the client's start point).
- No new dependency or file type.

**Why:** M9d5 of the M9d plan, split into a (the hand, this) and b (the
inventory panel; DESIGN has its plan).

**Verified:**
- `cargo test --workspace`: 460 passed, 0 failed, 39 ignored. Clippy with
  `-D warnings` and fmt pass.
- `sn-client --start -197.3 -35.2 -10.2 --use-item 3,4,5 --shot 7`
  (`out/m9d/m9d5a_client.log`): three times "Pick up Acid mushroom", the
  request, and the inventory line: (0,0), then (1,0), then (2,0), 1×1 each.
- `--start -283.5 -63.3 -114.4 --use-item 3,4,5,7,8`
  (`out/m9d/m9d5a_outcrop.log`): "Break limestone outcrop", three hits; the
  outcrop's key gone and one server object (copper ore) drawn; the hand on
  it shows "Pick up Copper ore"; picked into (0,0). `grab` on for 0.39 to
  0.40 s after each use.
- In the pod, `--use-item 3 --shot 15` (`out/m9d/m9d5a_locker.log`): walked
  round the ladder, the hand reaches the locker at 1.98 m, "Open storage
  container", opened. Screenshot `out/m9d5a-locker.png` shows the text over
  the locker.
- `sn-server` + `sn-inspect coop --count 4 --presets lan --seconds 20`:
  18,252 of 18,252 ticks exact, 128 hash checks, 0 differ
  (`out/m9d/m9d5a_*.log`).

**Dead ends and fixes:**
1. `--use-item` first aimed at the locker node's position and used the
   hand at once: the ray went up into the top hatch's trigger and started
   its cinematic. Aiming at the locker's collider gave the same 3.41 m:
   the locker is across the pod, out of reach. The flag now walks to the
   target and uses the hand only when its ray reaches an item or the
   locker.
2. Walking straight there was blocked by the ladder (0.2 m in 10 s); the
   `--use-hatch` rule (blocked 0.5 s: step aside 0.5 s) got round it.
3. Checked in the code: `isValidHandTarget`'s 2 s after creation does not
   apply to pickups, so no delay was added.

**Not tested:** keyboard and mouse play; the "Inventory full" text on
screen (unit test only).

## 2026-10-11 — M9d4b: the drawing follows the items

**What:**
- `sn-client` `items.rs` (new): `ItemsView` follows the session's shared
  item state after each world event: gone keys and server-made objects go
  to the object streamer (drawing) and to the collision worker (bodies).
  Debug flags `--pick-up <SECONDS>` (the nearest pickups within 3 m, at
  most 3) and `--drop <SECONDS>` (the first held item, 1 m ahead: our
  stand-in, the game's drop ray is M9d5).
- `objects.rs`: each drawn item instance keeps its `EntityKey`;
  `ObjectStreamer::hide_keys` despawns a gone key's entities and keeps it
  from loading again; `set_spawned` loads and draws the server's objects
  once (on the library worker) and removes those that go. `ItemDrawStats`
  counts it, logged with each change and when a server object is drawn.
- `player.rs`: the collision worker gets the gone keys and server objects
  (`BatchBodies::update_with`, `remove_key`, `sync_spawned`) and logs
  bodies removed and added.
- `session.rs`: `player`, `with_items`, `request`. `main.rs`: the flags,
  the install's item rules passed to the session and the streamer.
- Real-data test `hand_targets_follow_the_items` prints the pickup it
  chose (used to find the client's start point).
- No new dependency or file type.

**Why:** M9d4b of the M9d plan.

**Verified:**
- `cargo test --workspace`: 457 passed, 0 failed, 39 ignored. Clippy with
  `-D warnings` and fmt pass.
- `sn-client --start -197.3 -35.2 -10.2 --pick-up 3 --drop 6 --shot 10
  m9d4b-pick-drop` (`out/m9d/m9d4b_client.log`): 3 pickups requested; after
  the world events 3 keys hidden with 9 entities, and 3 bodies removed;
  after the drop 1 server object, 1 body added, then "server-made object 6
  drawn (3 entities)", `spawned_drawn` 1. Screenshot
  `out/m9d4b-pick-drop.png`: the dropped item in front of the camera.
- `sn-server --port 27777` with `sn-inspect coop 127.0.0.1:27777 --count 4
  --presets lan --seconds 20`: 12 of 12 pairs 100 % exact (18,264 ticks),
  128 hash checks with 0 differing (`out/m9d/m9d4b_*.log`).
- **Fix on the way:** the first run's stats were printed right after the
  drop event, before the prefab had loaded, so they showed 0 drawn. A log
  line when the object is actually drawn showed it was drawn once.
- **Not tested:** watching an item vanish in motion; keyboard pick-up (the
  hand comes in M9d5).

## 2026-10-11 — M9d4a: hand targets in the collision world

**What:**
- `sn-assets::collision_world`: with item rules
  (`CollisionLoader::set_item_rules`), every pickup and outcrop placed in
  a cell, an object batch or a spawn slot is a `Target` with its own
  bodies (`TARGET_BODY` ids) tagged with its `EntityKey` (placed id; slot
  as placeholder, slot, copy) and class.
- `BatchBodies` gains `target(body)`, `target_counts()`, `remove_key(key)`,
  `sync_spawned(...)` (bodies for the server's objects) and
  `update_with(..., gone)` (gone keys left out at load).
- `objects()` returns `BatchObjects { statics, targets }`. `Lifepod.locker`
  holds the locker's bodies (`LOCKER_BODY`), found through the scene's one
  `GenericHandTarget`.
- `sn-world`'s `SlotSpawn` has its `copy` index.
- `sn-sim`'s `World` has `get(id)` and a broad-phase grid for small bodies.
- No new dependency or file type.

**Why:** M9d4 of the plan, split into a (headless bodies, this) and b
(the client's drawing); DESIGN has b's plan.

**Verified:**
- `cargo test --workspace`: 457 passed, 0 failed, 39 ignored. Clippy with
  `-D warnings` and fmt pass.
- Real-data tests: 39 passed. The new `hand_targets_follow_the_items`
  prints its numbers (see DESIGN's M9d4a row).
- A new unit test compares the grid with a body-by-body brute force on
  3,000 random casts and sweeps, with removals: equal.
- `sn-inspect coop` with 4 bots on lan against `sn-server`: 18,252 of
  18,252 ticks exact, 128 hash checks with 0 differing, so the movement
  traces are unchanged (logs in `out/m9d/m9d4a_*.log`).

**Dead ends and fixes:**
1. The locker was first looked for under the `StorageContainer`'s node.
   It has no collider there (0 found). The scene dump showed a
   `StorageHandTarget` node, and the code showed its `GenericHandTarget`
   events call `EscapePod.StorageOpen`.
2. One body per target put about 1,800 bodies around the pod, and every
   query checked each one's bounds: a sweep went from 0.59 µs to 7.35 µs.
   The grid brought it to 0.89 µs.

## 2026-10-11 — M9d3d: item rules from the install

**What:**
- `sn_assets::item_rules(game, assets, dll)` builds `ItemRules` from
  M9d1's readers: pickups and outcrops by class id, outcrop resources
  resolved to class ids, TechData's sizes and equipment types with the
  code's defaults, slot table and player slots, the grid, analysis and
  default tech, the luck, the locker and supplies, and the game's tech
  type → prefab map (last wins over `prefabs.db` in order).
- `sn_install::read_prefab_database_ordered`, because the map depends on
  file order.
- `ItemRules`, `PickupInfo`, `OutcropInfo` and `OutcropDrop` moved to
  `sn-sim::inventory` (pure data), so `sn-assets` builds them without
  depending on the session; `sn-session::items` re-exports them.
  `OutcropInfo` gained `vertical_offset`.
- `Rules.items` and an `items` fingerprint part. The server, the client,
  the bots and `coop` use the real rules. `coop`/`bots` accept the new
  `Loaded` notice.
- No new dependency or file type.

**Why:** M9d3d of the plan.

**Verified:**
- `cargo test --workspace`: 456 passed, 0 failed, 38 ignored. Clippy with
  `-D warnings` and fmt pass.
- `SUBNAUTICA_DIR=… cargo test --release --workspace -- --ignored`: 38
  passed, 0 failed, including the new `item_rules_from_install` (counts in
  DESIGN's M9d3d row; two reads equal).
- `sn-server` with `sn-inspect coop 127.0.0.1:27777 --count 4 --presets
  lan,bad --seconds 20`: both ok. lan was 18,264 of 18,264 ticks exact;
  bad was 99.844 % exact, max 0.027 m. The server made 361 hash checks
  with 0 differing. Logs are in `out/m9d/m9d3d_*.log`.

**Dead ends and fixes:**
1. The first `item_rules` took 9.8 s cold and 4.7 s warm (3.6 s of it
   reading 1,543 prefabs, 0.9 s scanning the batches). Reading both in
   parallel threads (`std::thread::scope`, no dependency) brought it to
   2.1 s warm. The bundle cache's lock limits the prefab part (1.6 s).
2. The first coop run reported "problems": the tool listed the new
   `Loaded` notice as unexpected. The sessions themselves were all exact.

## 2026-10-11 — M9d3b + M9d3c: items as shared state, server and client

**What:**
- `sn-session::items`: `ItemRules` (pure, from the install in M9d3d) and
  `ItemsState`. This is every player's grid, slots, blueprints and luck,
  the lifepod locker, gone objects, server-made objects and outcrop hits.
  Each `Item` world event replays through `sn-sim::inventory`; a refusal
  changes nothing. It has a canonical encoding for the join snapshot and
  a hash part with running sums for the large maps.
- `WorldState` holds the items; new worlds fill the locker.
- The server takes `Request`s: it fills in the draws, the death list and
  spawned classes, checks the pod from the player's newest state, applies
  the event or sends `Refused`, and sends the items in `WorldChunk`s after
  `Welcome`.
- The client assembles the chunks, holds events and hash reports until
  then, and has `request()` and the `Refused`/`Loaded` notices.
- `sn-sim`: `Container::to_parts`/`from_parts`, `KnownTech::from_sets`,
  `SlotTable: Default`.
- Configs gained `rules`; the apps pass empty rules until M9d3d.
- New dependency: `sn-session` → `sn-sim`, so every machine replays the
  same item rules (pure, ours). Tests share synthetic rules in
  `tests/common/mod.rs`.

**Why:** M9d3b and M9d3c of the plan. They land together because b alone
broke two M10 tests: the server keeps a left player's items, and a late
joiner cannot know them without c's snapshot.

**Verified:**
- `cargo test --workspace`: 456 passed, 0 failed, 37 ignored. `cargo
  clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all
  --check` pass.
- `cargo test -p sn-session --release -- --nocapture` prints the numbers
  in DESIGN's M9d3b/c rows. Highlights: 10,000 ticks with 4 players, 0
  hash mismatches in 1,026 reports; 5,000 random events, with the server,
  a client and a halfway joiner equal; 1,000,000 bad requests, 0 panics;
  200,000 corrupted snapshots, 0 panics.
- Real game data: **not tested** (M9d3d).

**Dead ends and fixes:**
1. **b's first version** changed the world hash, and the M10 sync oracle
   and two server tests then reported mismatches. A client built from
   `Welcome` had no items for players already present, and a joiner could
   not know a left player's. First fix: `from_welcome` makes starting
   items for the listed players. Final fix: the snapshot.
2. **Hashing** the whole items encoding after every event made the
   1,000,000-bad-request test take 28 s. Accepted fuzzed pick-ups grow the
   gone set, so each event cost more. The large maps now keep a sum of
   per-entry hashes; the test takes 0.65 s. The change is the same for a
   long real game.
3. **The client's chunk handler** sized its buffer from the message's
   `total` before checking it: a bad count could have allocated gigabytes.
   Caught in review before any test; the check now comes first.
4. **Test mistakes, not code bugs:** the pod test sent two states in one
   tick (the server keeps only a newer tick's state), and one count
   threshold was a guess (events > 3,000; the run had 1,688).

## 2026-10-11 — M9d3a: protocol 4, items

**What:** M9d3 is planned in DESIGN ("M9d3 plan", four steps; decision 10
records its four choices). This is step a: `sn-protocol` 4 with
`EntityKey` (placed id, slot filler by placeholder/slot/copy, spawned
number), `ItemId`, the requests `PickUp`, `Hit`, `Drop`, `Move` and `Lose`
in `ClientMessage::Request {req, …}`, the world event `Item {player, req,
event}` (`PickedUp`, `Hit` with the server's luck draws, `Dropped`,
`Moved`, `Lost`), and `Refused {req, reason}` and `WorldChunk` from the
server. `codec` is public now, so the snapshot can reuse it. In
`sn-session`, the new messages are only logged as ignored; handling them
is M9d3b–c. No new dependency or file type.

**Why:** M9d3a of the plan.

**Verified:** `cargo test -p sn-protocol --release -- --nocapture`: all
pass. Every new message and variant round-trips and re-encodes to the
same bytes; malformed fields name their offsets (chunk index, a draw
above 1, unknown key/store/request/refusal, too many drops); the 1,000,000
random, truncated and bit-flipped inputs (the item samples included)
cause no panic. The largest message of all the samples is 1,033 B, under
`sn-net`'s 1,161 B. `cargo test --workspace`: 439 passed, 0 failed, 37
ignored; `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` pass. Note: the protocol version is now 4, so a
client of this build and one of the previous build refuse each other
(by design).

**Dead end:** the first encoding gave each death drop a full point and an
`f32` rotation (28 B). A full inventory's drops (54) would have been about
1.6 kB, over `sn-net`'s message limit. Each drop is now a millimetre
offset from one origin with an i16 rotation (14 B). This is exact for the
rules, which put every drop within 1.5 m of the camera.

## 2026-10-11 — M9d2: inventory rules, pure

**What:** `sn-sim::inventory`. Ports `ItemsContainer` (groups, `GroupComparer`,
`TrySort` / `GetRoomFor` / `NextFreeCell`, `HasRoomFor` with its ghost and
`ignore`, `RemoveItem(TechType)`'s battery rule), `Equipment` (`GetFreeSlot`,
`GetCompatibleSlotDefault`, `IsCompatible`), `Inventory.Pickup`, `AddOrSwap`
(grid, slot, locker, with the game's undo), `KnownTech.Analyze` / `Add`,
`FairRandomizer`, `PlayerEntropy`, `BreakableResource` (hits, draws,
default), `CanDropItemHere`, `Inventory.RayCast` and `FindDropPosition` on
`sn-sim::collide`, `LoseItems` with the one-item quirk, the escape pod
supplies and the `grab` bool's 0.4 s. Random draws are parameters, as in
`aurora`. No new dependency, crate or file type.

**Why:** M9d2 of the M9d plan.

**Plan corrections (read in the code):** the "1 s after a drop" pick-up
rule never applies, because `Pickupable.Drop` sets `timeDropped = 0` rather
than the time. It is not ported, and the M9d3 and M9d6 checks are changed
to match. The resource's torque is `Random.Range(3, 6)` with ints (3–5),
about the world x axis. `PlayerEntropy.CheckChance` is false for a tech
type without a randomizer, even at chance 1. Facts are in `gameplay.md`
§ Inventory; the plan text is in DESIGN.

**Verified:** `cargo test --workspace`: 438 passed, 0 failed, 37 ignored
(33 new in `sn-sim::inventory`). `cargo clippy --workspace --all-targets --
-D warnings` and `cargo fmt --all --check` pass. The tests are the M9d2
"Done when" list: hand-worked packings, a full grid refusing, removal
re-packing, tank to its slot and food never, swap between grid and locker
(and undone when it cannot fit), a seeded `FairRandomizer` at 0.1, 0.25,
0.5 and 0.9 within 0.001 of the chance over 100,000 tries, the outcrop
default, no drop in the pod, the drop point before a synthetic wall, and
death keeping one item. Also 20,000 random drags between grid, locker and
slots: no item lost or copied, and no grid over-full. Against the real game's
behaviour: **not tested** (no real-data step in M9d2).

**Dead ends:** two of my first test expectations were wrong; the code was
right. (1) "42 of 48 cells taken leaves no room for a 2×2" is false: every
check re-packs bigger items first, so 46 cells fit. The test now uses 45.
(2) Exact drop distances failed by 5 mm because our casts stop between
`SKIN / 2` and `SKIN` before a surface. The test allows `SKIN`, and the
difference from the game (up to 1 cm) is recorded in the code and DESIGN.

## 2026-10-11 — M8h: shared-clock sky and lighting

**What/why:** evaluate retained sky rules from the session clock each frame,
update the marked sun/shadow transform and continuous planet days, and order
fog, sky/stars, water reflections and shafts after evaluation. Existing light
parameters follow fog; material emission and ambient inputs use that state.
Keep clouds, twinkle, waves and caustics on their existing elapsed clocks.
Correct exact midnight from sky timeline 24 to 0, confirmed by read-only
inspection of the game's manager. Add opt-in `--log-sky` fixed-tick samples.
No dependency, architecture or whitelist change. Format notes and README
updated; full acceptance gaps remain explicit in DESIGN.

**Verified:** `cargo test --workspace`: 405 passed, 37 ignored;
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all -- --check` pass; release build passes. Six sky tests cover
synthetic evaluation/clock/sun behavior. Three frozen performance runs match
previous sky/fog/scene values; means 17.640–18.136 ms, p99 19.461–22.168 ms.
No speedup claimed. Noon/sunset/midnight and a 240× midnight crossing have
300-frame reports and captures in ignored `out/`; clock values agree with
existing evaluation, except the intentional exact-midnight correction.
Host/join: 82 identical fixed-tick samples, including eight midnight wraps.
Exact commands, expected outputs and GPU numbers are in DESIGN M8h.
Original-game visual matching and isolated sky CPU cost: **not tested**.

**Verification dead ends:** an initial PowerShell background launcher exposed
an unset exit code after `WaitForExit`, stopping a repetition loop early;
using `Start-Process -Wait -PassThru` completed the missing runs. The peer
launcher combined `--free-cam` with player-only `--shot`, so timed shutdown
did not run; after successful comparisons only those two owned processes
were stopped manually. This is not evidence of normal peer shutdown.
Initial midnight capture exposed the pre-existing 24→0 boundary error;
corrected it and reran midnight/crossing captures as `-final`.

## 2026-10-11 — M8g0a: named visual performance baselines

**What:** `sn-client --baseline NAME --baseline-revision LABEL` with a solo
`--benchmark` saves configuration/scene snapshots, `frames.csv` and a capture
under ignored `out/baselines/run-NAME/`. Existing names are rejected. Added
physical `--resolution WIDTH HEIGHT` and consecutive settled `--warmup FRAMES`
(120 by default for named baselines, 0 for existing measurements). Measurements
run after frame updates and use `Time<Real>` so virtual-time clamping cannot
hide stalls. Reports include game build/rules fingerprint, explicit revision
label, effective world/camera settings, terrain/grass/object/material/light/
LOD/sky counts, fog coefficients, game/visual/water times, frame statistics,
sampled memory peak and recent GPU diagnostic averages with their scope.
The report uses an explicit field list, not raw arguments or environment.
No new dependency or whitelist change. DESIGN contains the plan/results and
README the tested command. Five tests cover flags, labels, warm-up and timing
statistics. Ordinary play keeps its default DPI behaviour.

**Why:** the user approved starting the visual plan. M8g0a makes the existing
benchmark preserve reproducible configurations and evidence for subsequent
changes. It records live visual times; freezing/resetting animations, waves
and foam, other comparison locations and original-game references remain open
under M8g0. No renderer formula or gameplay rule was changed.

**How verified:**
- `cargo test --workspace`: 403 passed, 0 failed, 37 opt-in tests ignored.
- `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all -- --check`: clean. `cargo build --release -p sn-client` passed.
- With `SUBNAUTICA_DIR` set, ran this command three times, replacing `N` with
  1, 2 and 3 (use fresh names to repeat):

  ```powershell
  .\target\release\sn-client.exe --benchmark 300 --baseline m8g0a-shallows-N --baseline-revision cae48d6-m8g0a --warmup 120 --resolution 1600 900 --start 0 -10 0 --look 60 -40 60 --fov 60 --view 1200 --lifepod-seed 1 --slot-seed 1 --time 9.6 --time-scale 0 --aurora intact --water-quality high --color-grading off --gpu-timings
  ```

  Exit 0 each, 300 samples after 120 warm-up frames, 0 unsettled measurement
  frames. PNG headers: 1600 × 900. CSV sample sums match the measured interval
  within 20 µs. A local numerical comparison (`python out/verify_m8g0a.py`)
  checks 58 static fields across the three runs and within their start/end
  snapshots: camera/settings, shown terrain/grass, objects/materials, light
  counts, LODs, sky and fog all equal. 1,393 terrain batches, 7,759,704 terrain
  triangles, 34,469 tufts, 39,342 cell/batch objects, 1,968 materials, 508 point
  lights, 51 spots. Reports and captures stay under `out/`.
- Means 17.94 / 17.95 / 18.02 ms; medians 17.69 / 17.83 / 17.86 ms; p99
  20.31 / 19.27 / 20.52 ms; worst 21.22 / 22.51 / 23.56 ms. Peak sampled
  memory 1.893 / 1.894 / 1.851 GiB. RTX 3080; 16.7 ms target **not met**.
  This establishes a baseline, not a before/after performance claim.
- Existing-name check:
  `.\target\release\sn-client.exe --benchmark 1 --baseline m8g0a-shallows-1 --baseline-revision overwrite-check`
  returned exit 1 before loading the game; SHA-256 hashes of its report, CSV
  and PNG were identical before/after.
- Ordinary player smoke run:
  `.\target\release\sn-client.exe --view 400 --kill 3 --shot 10 m8g0a-player-smoke`
  returned exit 0, logged death/respawn/restoration/controls returning, and
  saved the capture. Keyboard play: **not tested**. The other M8g0 locations,
  original-game visual match and frozen visual-time capture: **not tested**.
- **Corrections during verification:** clippy caught a median helper using
  `is_multiple_of`, newer than the declared Rust 1.85 minimum; replaced with
  modulo. The first cross-run comparison incorrectly included parsed-batch
  cache occupancy among static scene fields: it varied 1,150 / 1,200 / 1,166.
  Reading `BatchCache::insert` confirmed worker-order-dependent LRU eviction
  of 120 entries at a time. The comparison now checks shown geometry exactly
  and records cache occupancy as a bounded memory metric (limit 1,200).
- `git diff --check`; `git status --short`: only our source/docs, no generated
  data. No commits or pushes.

## 2026-10-11 — Visual fidelity backlog and missing plans (docs only)

**What:** DESIGN § 4.5 adds M8g0 reproducible comparison/performance baselines,
M8h shared-clock sky and lighting, and M4c terrain geometry fidelity, each with
a "Done when" checklist. It collects scattered visual gaps under their existing
milestones, identifies historical descriptions superseded by later features,
and records the proposed implementation order. The gameplay deferral and
floating-pod-after-M11 decision remain. No code, dependencies or ignore rules
changed; README's tested feature claims are unchanged.

**Why:** the user asked to write down the missing parts of the visual review,
so terrain equality, reproducible comparisons and follow-up work cannot be
lost among first-pass milestones and historical "as built" notes.

**How verified:** read against DESIGN §§ 4.1–4.4, format notes and the current
client paths for sky initialization, water settings, grading, shader fallbacks,
local lights, animation and shadow-only drawing. `git diff --check` and a review
of the documentation diff passed; `git status --short` lists only DESIGN and
this MODLOG. Runtime visuals and Rust tests: **not tested** (documentation only).
No commits or pushes.

## 2026-10-11 — M9d1: item data, headless

**What:**
- `sn-unity::items` (new): layouts of `Pickupable` (5 bools), `TechTag`,
  `PrefabIdentifier` (class id only), `PlayerEntropy`, `FairRandomizer`,
  `LootSpawner`, `StorageContainer`, `SpawnEscapePodSupplies`,
  `PrefabListSpawner`; 6 tests (synthetic objects, every prefix an
  error, a trailing byte an error, byte flips never panic). A few
  `gameplay.rs` helpers became `pub(crate)`.
- `sn-dotnet::items` (new): `constructor_args`,
  `static_string_int_dictionary`, `string_array`: exact IL patterns for
  `Inventory.Awake`'s container size, `Equipment.slotMapping` and
  `Inventory.UnlockDefaultEquipmentSlots`; 4 tests on synthetic
  assemblies (including a wrong pattern of each kind and damaged bytes).
- `sn-assets::items` (new): `item_code`, `TechLookup` + `item_prefab`
  (the game's `CraftData.GetTechType` rule), `addressable_prefab` (an
  `AssetReference` GUID to its prefab), `player_items` (luck, the
  `LootSpawner` found through the essentials scene's spawner, the pod's
  locker through `SpawnEscapePodSupplies`), `language`; 2 tests. Real-data
  test `item_data`.
- `sn-inspect pickups` (new).
- Docs: `gameplay.md` § Pick-ups and outcrops (rewritten), § Inventory
  (new), § Text; `dotnet.md` § Item rules; DESIGN M9d1 row.
- No new dependency.

**Why:** M9d1 of the M9d plan.

**How verified:**
- `cargo test --workspace`: 398 passed, 0 failed; clippy
  (`--all-targets -D warnings`) and fmt clean.
- `SUBNAUTICA_DIR=… cargo test --release -p sn-assets -p sn-dotnet -p
  sn-unity -- --ignored`: all passed (sn-assets 32 including `item_data`:
  189 slot prefabs, 108 with a `Pickupable`, 0 without a tech type, 3
  outcrops whose 8 resources resolve to their listed tech type).
- `sn-inspect pickups` (`out/m9d/pickups.log`, 13 s): 1,543 prefabs, 0
  read errors; `Pickupable` on 156 prefabs (50 placed, 23,832
  placements, 106 slot-only: P0's census numbers again); tech type from
  `TechTag` 72, class id 84; 1 `None`; 101 tech types, all named in
  English; sizes 1×1 86, 2×2 15; 4 outcrops, 9 of 9 resources match;
  inventory 6 × 8; 38 slots, the player's 7 all in the table; 8
  randomizers; pod locker 4 × 8, 6 starting items; 126 analysis entries.
- **Dead end:** `PrefabIdentifier` was first read as two strings (class
  id, id), from `UniqueIdentifier`'s fields; every one of the 1,543
  prefabs failed 4 bytes short. The asset holds the class id only.
- **Found:** one placed pick-up (a prop inside
  `DeepGrandReefAbandonedBase`) has tech type `None` by the game's rule;
  that the game also gives `None` is a hypothesis (not checked in the
  game).
- `git status --short`: only our source and docs.

## 2026-10-11 — M9d plan: pick up and inventory (docs only)

**What:** `docs/DESIGN.md` § 4.3: the M9d plan (steps M9d1–M9d6, who
decides what, object identity) with equipment slots, unlocks on
pick-up, dropping, the lifepod's locker and the `grab` animation pulled
in, and every known gap from the game listed with the milestone it goes
to; a new row **M9u**, the game's UI, after M9d and before M9e; "After
Phase E" item 1 lists the gaps M9d leaves there. No code.

**Why:** M10's basis is in; M9d is next and too big for one step. The
user asked for the plan to be as close to 1:1 as possible and for every
remaining difference to be written down for later.

**How verified:** the game's rules quoted in the plan were read in the
decompiled code (`Pickupable`, `Inventory`, `ItemsContainer`,
`BreakableResource`, `PlayerEntropy`, `FairRandomizer`,
`CraftData.GetTechType`, `SpriteManager`); they become *confirmed* in
`gameplay.md` only when M9d1/M9d2 check them (also read: `Equipment`,
`KnownTech`, `QuickSlots`, `ArmsController`, `SpawnEscapePodSupplies`,
`LootSpawner`). Approved by the user 2026-10-11.

## 2026-10-11 — M10h3: other players' hatch cinematics and the pod's doors

**What:**
- `sn-protocol` 3: player events `CinematicPrepare { on }` (5) and
  `CinematicPlay { on }` (6); round-trip test list extended; the world
  state passes them through. `docs/formats/protocol.md`.
- `player.rs`: the cinematic's `Prepare` / `Play` signals are sent at
  their step, after `HatchUsed` and before `CinematicEnded` (receivers
  then always know the hatch). The pod (colliders, hatch triggers,
  cinematics) loads whenever the world has one; `Start::in_pod` says
  whether the player starts in it (`--start`: outside, pod still there).
  `PlayerSim::hatch_names`.
- `remote.rs`: each remote player's open hatch from the due events;
  signals go to the drawn pod's animators (`BodyDrive::pod_values`) and
  the remote body's animator (the player's bool). Runs before
  `body::drive`.
- No new dependency.

**Why:** M10h3 of the M10 plan.

**How verified** (`out/m10h/f-*.log`):
- `cargo test --workspace`: 386 passed; clippy and fmt clean.
- Host floating outside (`--start -127.5 0.3 -57 --look-down -25`),
  joiner `--use-hatch 3 --hold-forward 14 --kill 25`, both `--log-ticks
  1`: the host saw the joiner's 10 events with the owner's ticks (hatch,
  prepare on, play on, prepare off, play off twice: the first-use clip's
  two end events, cinematic end, death, trigger, respawn) and applied the
  5 signals ("hatch 6 play true at tick 676", …); drawn ticks 2,002/2,003
  and 2,117/2,117 exact.
- Same clock (promise 5): host and joiner both put the Aurora's
  countdown at server tick 224,979.2 and the swap at 226,329.2.
- Solo: the M10g hatch run's trace equal to M10g's (453/453, 15 events).
- README: "Play together" (hosting, joining, the UDP port to forward,
  what is synced and what isn't, trust, what was tested): M10i's text,
  written ahead of the user's test.
- **Not tested:** two players using one hatch at once (the own simulated
  pod animator does not get the others' signals); the climb by eye (the
  screenshot at 17 s came before the joiner's hatch use).

**Found:** the first run had the host started outside the pod, so it had
no pod cinematics and dropped the signals ("no known hatch"); hence
loading the pod whenever the world has one.

## 2026-10-11 — M10h2: other players' bodies

**What:**
- `sn-client` `remote.rs` (new): for each other player in the world
  (joined and `Ready`), the player's own body prefab; its rig base at
  the drawn sample (the state's rotation in a cinematic, `camRoot`'s
  height: minus the skin and the `verticalOffset` bob, turned by the
  look's yaw: as `Body::view_model`); its animator gets the state's
  floats and bools and the owner's animator triggers (world events);
  the head (the one shadows-only part) is drawn; hidden while there is
  no sample; removed on leave. A status line every 10 s.
- `objects.rs`: `RemoteBodyRig`, `want_remote_body` /
  `remove_remote_body` / `remote_body_ids`; the spawn reuses the own
  body's prefab and `spawn_instance`.
- `session.rs`: `remotes()` says who is ready; `due` keeps the frame's
  world events. `player.rs`: `pose_to_bevy` public, `body_params()`.
- No new dependency.

**Why:** M10h2 of the M10 plan.

**How verified:**
- `cargo test --workspace`: 386 passed; clippy and fmt clean.
- Host and joiner on this machine (`out/m10h/b-*.log`): each spawned
  the other's body at the join; 1 shadows-only part (the head) drawn;
  placed vs drawn rig base 0.000000 m over 566–940 checks per 10 s;
  the host removed the joiner's body when it left.
- Screenshot (`out/m10h2-join-view.png`; joiner `--start -127.5 0.3 -57
  --look-down -25`, host `--use-hatch 3`): the host's body stands on the
  pod's roof after the hatch exit, head drawn, facing +x as its look.
- **Not tested:** the swim and walk animations in motion by eye; more
  than two players.

**Found:** the host logged a removed body as placed again in the same
frame (the despawn lands at the frame's end); removed bodies are now
skipped. `--look` does not turn the player (only the free camera); the
screenshot used a start south of the pod instead.

## 2026-10-11 — M10h1: the link (`--host`, `--join`), states and events

**What:**
- `sn-client` `session.rs`: the session's server, client and UDP socket
  run on a network thread (every 1 ms) under a mutex; the main thread
  takes steps, pushes states and events, samples remote players. Modes:
  solo (as M10g), `--host [PORT]` (the in-process server also on UDP;
  the local client is a direct link), `--join ADDRESS[:PORT]` (waits up
  to 10 s for the `Welcome` and the first clock sample before loading:
  the host's settings decide the world; a reject prints its reason).
  `--name` (default: USERNAME/USER), `--net-sim lan|internet|bad` (the
  client's own link, through `sn_net::ImpairedTransport`), `--log-ticks
  N` (testing). Logs: own state every N ticks, each remote player's
  drawn state at the same ticks, own events, world events when the drawn
  tick reaches them; a status line every 60 s (messages, delay, hash
  checks, steps skipped, peers' bytes and round trip).
- `player.rs`: after every step the own state (as `sn-inspect coop`'s
  bots: position, velocity, look, rotation in a cinematic, flags, life,
  the body's last animator values) and the events (hatch used, cinematic
  ended, died, respawned; animator triggers at the next step's tick).
- `sn-session` `Client::steps`: more than 25 steps due → all but the
  newest 10 skipped and counted (`ClientStats::steps_skipped`); unit
  test extended. `docs/formats/protocol.md` § Client rules.
- Dependency: `sn-client` uses `sn-net` (UDP for host and join; ours).

**Why:** M10h1 of the M10 plan (the sub-steps written into DESIGN first).

**How verified** (logs in `out/m10h/`, compared by `out/m10h/cmp.py`):
- `cargo test --workspace`: 386 passed; clippy and fmt clean.
- Two clients on this machine: `sn-client --view 400 --log-ticks 1
  --host 27431 --name host --use-hatch 3 --hold-forward 12 --kill 30
  --shot 60 …`, then 8 s later `--join 127.0.0.1:27431 [--net-sim P]
  --name joiner --use-hatch 4 --hold-forward 14 --kill 25 --shot 50 …`.
  Each drawn tick of the other compared with the other's own log:
  - no impairment: 2,504/2,504 and 2,616/2,616 exact;
  - `internet` on the joiner: 2,502/2,503 and 2,622/2,622 (max error
    0.07 m);
  - `bad`: 2,498/2,503 (99.8 %, max 0.35 m) and 2,611/2,611;
  - every miss is among the joiner's last ticks (3,034 and 3,064–3,068;
    its last own tick 3,034 and 3,068): it quit before those states were
    sent, and the host extrapolated.
  - Events: the host saw the joiner's 5 (hatch, cinematic end, death,
    trigger, respawn) and the joiner the host's 4 after its join, kinds
    and ticks equal; the host's first hatch use came before the join, in
    the `Welcome`.
  - Host: 115–116 world-state hash checks per minute, 0 differ; 0 own
    steps skipped; delay 3.2 ticks (none), 13.3 (bad). Under `bad` the
    server ignored 2 states that overtook `Ready` (unordered vs reliable
    channel; harmless, noted).
- Solo after the change: the M10g hatch run's trace is identical to
  M10g's (453/453 states, same ticks, 15 events).
- **Not tested:** two machines; keyboard play together; a password (not
  offered by the client yet).

**Dead end / found:** the first two-client run (full view distance) had
both clients at 2–4 fps once both were loaded; with at most 10 steps per
frame the host's physics fell up to 15.7 s behind the clock, so the
joiner drew it "held" at 32 of 48 logged ticks and the delay grew to its
50-tick cap. Hence the backlog skip, and `--view 400` for two clients on
one GPU.

## 2026-10-11 — M10g: solo play through the in-process server

**What:**
- `sn-client` `session.rs` (new): `sn-session`'s server and client in
  the client, joined over a direct in-memory link (every message
  encoded and decoded, as on the network; no sockets, no impairment).
  The session's time 0 is the first frame. It logs the join, `Ready`,
  any mismatch, and a status line every 60 s (messages, events, hash
  reports and checks, clock jumps).
- `main.rs`: `start_session` reads the rules (`sn_assets::rules`: build,
  fingerprint, tick, the Aurora's numbers), makes the new world's
  settings from the flags (Lifepod 5's point from the seed or `--lifepod`,
  the Aurora from `--aurora` / `--aurora-countdown` / the seed, the slot
  seed, the clock), joins, and takes them back from the `Welcome`;
  `lifepod_start` is gone (its work is in there, assets indexed once).
- `player.rs`: one physics step per server tick (`Client::steps`; at most
  10 per frame, the rest later instead of dropped); `Ready` goes to the
  server when the collision around the player is in.
- `aurora.rs`: the game clock is `clock_start` + the server's time ×
  `time_scale` (was: the sum of Bevy's frame times, which clamps long
  frames); the Aurora's countdown counts from the world's start; event
  lines carry the server tick.
- `sn-sim` `Exploder::new_game_at` (new) with a unit test.
- Dependencies: `sn-client` now uses `sn-protocol` (the settings) and
  `sn-session` (the in-process server and client), both ours.

**Why:** M10g of the M10 plan: from here solo and multiplayer run the
same code, so M10h only opens the link to the network.

**How verified** (logs in `out/m10g/`):
- `cargo test --workspace`: 386 passed (1 new). `cargo clippy
  --workspace --all-targets -- -D warnings`: clean. `cargo fmt --check`:
  clean.
- Baseline first, on the unchanged code: `sn-client --hatch-name top
  --use-hatch 3,15,27 --hold-forward 31 --kill 40 --shot 45 NAME`, run
  twice. **The two baseline runs were not equal:** the scripts are
  checked once per frame, so one run placed a scripted moment a step
  later (85 of 452 trace lines shifted one step, one stationary line
  differs at a window's edge), and the Aurora's countdown moved with load
  time (4983.0 s vs 4982.7 s). So "equal positions at every step" could
  not be the check; compared instead (`out/m10g/cmp.py`):
  - after the change, two runs: identical line for line (453 trace
    lines, 15 event lines);
  - baseline vs after: the 453 traced states equal in sequence (position,
    velocity, motor, flags, look, input), the 15 event lines (hatch uses,
    push-outs, death, respawn, with positions) equal, scripted moments
    within one step of the baseline's (151 lines one step earlier, as
    between the baseline runs).
- `--kill 3 --shot 15`, before and after: the 11 player lines equal;
  `Died` and `MoveToRespawn` at the same places.
- The Aurora: countdown at 4979.6 s game time (server tick 224,979.2),
  swap at 5006.6 s (226,329.2), equal in every run after the change
  (the countdown now counts from the world's start, not from when the
  ship's scene first updated: 3.4 s earlier than before on this
  machine, which is the load time). Lighting: the pod's state lines are
  equal (they follow the player, not the clock).
- `--benchmark 300` (no player): runs; session start 0.57 s.
- Session lines: joined as player 0, welcomed with the settings, ready
  at server tick 183.9 (3.9 s after start).
- `--shot 70` (75 s): the 60 s status line: tick 3,001, 294 messages to
  the server and 234 back, 2 world events (joined, ready), 61 world-hash
  reports, 61 checked, 0 differ, 0 clock jumps; no warnings or errors.
- **Not tested:** playing with keyboard and mouse after the change.

**Found:** the session's join notices were logged before Bevy's log
existed and were lost; now logged in the first frame.

## 2026-10-11 — M10f: bots with the player's rules, over impaired UDP

**What:**
- `sn-inspect coop <ADDR> [--count 2,4] [--presets lan,internet,bad]
  [--seconds S]` (new, `coop.rs`). Each bot's script runs first with the
  player's own rules and collision (`walk`'s `start`, `leave_pod`,
  `go_to`, `board`; vitals on; a kill through `Vitals::take_damage`),
  recording every step: position, velocity, rotation, look, flags,
  life, and the animator values M9g's rules computed. Events come from
  the record (hatch start/end, death, respawn, animator triggers). Then
  the bots join `sn-server` over UDP, each link impaired by the preset,
  and replay the record in the server's ticks. Every drawn tick is
  compared with the owner's state; every bot's received player events
  are compared with what the owner sent (kinds and ticks); slot-fill
  hashes go to the server.
- Why record and replay rather than step live: bots do not affect each
  other (players pass through each other; collision is static), so the
  record is exactly what the bot would do live; the scripted runner
  (`walk.rs`) loops internally and would need a rewrite to step from a
  network loop. The real client (M10h) steps live.
- `walk.rs`: an optional per-step record (`StepRecord`); `body_run.rs`:
  the last animator values.
- `sn_net::ImpairedTransport` (new): a real transport with the presets'
  delay, jitter, loss and duplication on the wall clock, both ways.
- `sn_assets::CollisionLoader::slot_fill_hashes` (new): per batch, a hash
  of what its slots were filled with (`protocol.md` § Slot-fill hash).
- `sn-session` `Client::slot_fill`, `Client::steps_taken`; the server no
  longer counts moves while a player is not alive as implausible (the
  respawn move showed as 2,700 m/s); `sn-server` logs work per pass as
  median, p99, p99.9, max and passes over 1 ms, and slot-fill checks.

**Why:** M10f of the approved M10 plan.

**How verified** (`out/m10f-server.log`, `out/m10f-coop.log`,
`out/m10f-coop8.log`; `sn-server --port 27424 --run-for 1500 --report
30`, then `sn-inspect coop 127.0.0.1:27424 --count 2,4 --presets
lan,internet,bad --seconds 60` and `--count 8 --presets lan --seconds
120`):
- Recording: about 2.3–3.9 s per bot. First exit through the hatch at
  step 91, its cinematic 330 steps (6.60 s); boarding 110 steps (2.20 s),
  as M9g5's expected values; kills: died at step 822, respawned at
  1,072 (5.0 s). In the 8-bot recording bot2 did not board within its
  time (`boarded: false`) and floated on; noted, not investigated.
- Drawn ticks exact: lan 6,394/6,394 (2 bots), 38,316/38,316 (4),
  347,032/347,032 (8); internet 6,381/6,381 (2), 38,272/38,283 = 99.971 %
  (4), max error 0.058 m; bad 6,350/6,362 = 99.811 % (2), 38,299/38,358
  = 99.846 % (4), max error 0.237 m. All targets met.
- Events: in every run each bot's list of each other bot's events
  equals what that bot sent, ticks included (no "EVENTS DIFFER").
- Server: world-state hash checks 0 differences in every 30 s window;
  slot-fill checks 12 / 24 / 56 per join wave, 0 differences; upload
  8,043–8,295 B/s per bot (target < 10 KB/s).
- Server work per pass (wall time), 8 players, three 30 s windows:
  median 31–32 µs, p99 286–307 µs, p99.9 445–471 µs, max 875, 1,116 and
  810 µs (1 pass over 1 ms of about 59,000). With 2–4 players some
  windows had stalls up to 25.6 ms (114 passes over 1 ms in one), which
  do not follow the load. The worst-case target (< 1 ms) is met to
  p99.9, not strictly; CPU time was not measured (needs OS calls or a
  dependency: DESIGN § 9, item 9, open).
- "2 ignored messages" on the bad preset: states (unordered) arrived
  before a resent `Ready` (reliable) and were dropped, as the server's
  rule says. Harmless; noted.
- `cargo test --workspace`: 385 passed (the server's states test now
  also checks a respawn jump is not counted). `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- Not tested: live stepping in the client (M10g/h), a real network
  (M10i).

## 2026-10-11 — M10e: `sn-server`, the headless server

**What:**
- `sn-assets::host` (new): `build_number`, `rules` (the rules
  fingerprint: FNV-1a 64 over the player, vitals and look parameters'
  `Debug` text, the raw `Balance/TechData` bytes and the Aurora's code
  numbers; plus the tick length and the plausible-speed limit),
  `lifepod_point`, `aurora_countdown`. Shared so server and client
  compute the same values. `sn-assets` now depends on `sn-protocol` (for
  the hash).
- `apps/sn-server` (new): `Host` (the session server behind an `sn-net`
  endpoint, any transport; log lines; reports with per-player bandwidth,
  round trip and counters) and `main.rs` (flags as the client's names
  and defaults: `--lifepod-seed`, `--slot-seed`, `--time`,
  `--time-scale`, `--aurora`, `--aurora-countdown`; `--port` default
  27420, `--bind`, `--max-players`, `--ask-password` read from the
  console and never stored, `--report`, `--run-for`). Reads only the
  host's install; sends only ids, numbers, seeds and hashes.
- `sn-inspect bots <ADDR> [--count N] [--seconds S]` (new): N bot
  players over UDP on scripted paths (no collision; M10f adds the
  player's rules), each drawn tick compared with its owner's state.
- Docs: `protocol.md` § Rules fingerprint; README (try it, layout).

**Why:** M10e of the approved M10 plan.

**How verified:**
- `sn-server --run-for 25` on the install: ready 0.7 s after start;
  build "10", fingerprint `d269297a0830e68b` (parts logged), tick 20,000
  µs, implausible above 100 m/s; seed 1: Lifepod 5 at (−127.73, 0,
  −49.75) after 72 draws, Aurora countdown 4,499.6 game s; idle loop
  6 µs of work per pass.
- `cargo test -p sn-server`: over UDP on 127.0.0.1 a build "9" client
  gets `Reject(Build "10")` and the server logs "rejected …: … same game
  build"; a build "10" client is welcomed as player 0; 200 junk datagrams
  counted as bad, nothing breaks.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data
  multiplayer -- --ignored`: the fingerprint and parts are equal over
  two reads, equal to the release server's, tick 20,000, seed 1 → 72
  draws, countdown within 2.3–4 days.
- 10 minutes, `sn-server --run-for 630 --report 60` and `sn-inspect
  bots 127.0.0.1:27422 --count 8 --seconds 600` on this machine
  (`out/m10e-server.log`, `out/m10e-bots.log`): 0 panics, problems
  none; 1,679,888 ticks drawn over 56 pairs, all exact, max error 0 m;
  0 hash mismatches in 4,700+ checks; 84,000 states relayed a minute
  (8 × 25 × 7 × 60); up 8,156–8,162 B/s per bot, down about 47 KB/s;
  round trip 1.6–2.7 ms.
- Server work per loop pass (wall time around `Host::step`): mean 37–54
  µs per minute; worst pass per minute 1.0, 1.2, 1.8, 0.6, 1.1, 3.0,
  0.9, 1.2, 1.4 ms, and 6.0 ms in the first minute while `cargo test`
  compiled. **The < 1 ms target holds for the mean, not for the worst
  case.** Suspected cause: OS scheduling (wall time, the 8 bots on the
  same machine); not proven. M10f is to measure the server's CPU time.
- The scripted paths' 30 m jump shows in the server's log as an
  implausible move (1,505 m/s), as it should (not flagged cinematic).
- `cargo test --workspace`: 385 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- Not tested: the game client joining (M10g/h), another machine (M10i).

## 2026-10-11 — M10d: the client's half, and the sync oracle

**What:** `sn-session::client::Client` (pure, on the caller's clock):
`Hello` on creation, `Welcome` → world state, `Ready` once; clock sync
(pings 50 ms ×20 then 250 ms; the shortest round trip of the last 10 s
gives the offset; slewed ≤ 1 %, jump above 250 ms); `steps(now)` gives
the own physics steps in the server's ticks; own states sent every 2
ticks; a tick buffer per remote player with `sample(player, tick)`:
exact, linear between consecutive ticks, Hermite across gaps, a held
earlier state across a teleport, 250 ms of extrapolation then hold;
events applied on arrival and released in order at the drawn tick;
`WorldHash` every 50 ticks. `sn-net` gains a third channel,
**unordered** (frame kind 3). The sync oracle
(`crates/sn-net/tests/sync_oracle.rs`): a server and two clients over
the simulated network, scripted paths with sharp turns, stops, a dive, a
zigzag and a 30 m teleport every 12 s; every whole tick a client draws
is compared with the other client's own record of that tick.
`docs/formats/protocol.md` § Client rules; channel table and bandwidth
updated.

**Why:** M10d of the approved M10 plan.

**Dead ends and changes on the way (each run's numbers, both directions):**
1. As planned (states on the latest-only channel, 4 states per message,
   delay from each message's newest tick, 2 s window, cap 25 ticks): lan
   100 %; internet 99.885 / 99.943 %; bad 86.4 / 86.1 % exact. On bad
   about 10 % of ticks never arrived: the latest-only channel drops a
   message overtaken by a newer one, and with ±50 ms jitter on two legs
   that is common, yet the older message still carried ticks nobody else
   had.
2. States on a new unordered channel (both ways), delay from each newly
   arrived tick's lateness: bad 96.4 / 95.7 %; the rest mostly
   "drawn before it arrived".
3. Window 10 s and cap 50 ticks: no change (96.5 / 95.8 %), so neither
   was the limit.
4. A 1 s warm-up in which the delay takes the measured value at once
   (it started at 5 ticks and may only grow 10 % per unit of time; bad
   needs about 22): bad 99.04 / 98.84 %, internet 99.885 % in both
   directions. Printing the misses showed two at start-up and two loss
   bursts: with each tick in two messages, losing both happens about
   0.1 % of the time at 1 % loss per leg, right on the 99.9 % line.
5. 6 states per message (each tick in three): all targets met (below).
   Cost: upload 8.0–8.1 KB/s instead of about 5.4 (M10f's target raised
   from 8 to 10 KB/s with this reason).

**How verified:**
- `cargo test -p sn-net --test sync_oracle -- --nocapture
  --test-threads=1`: 3 passed. 70 s per run; ticks drawn about 3,430–3,500
  per direction:
  - lan: 3,498 / 3,498 exact both ways, error 0, clock error 0 µs, delay
    2.1 ticks;
  - internet: 99.914 % both ways (3 drawn before arrival each), max
    error 0.0000 m, clock error after 10 s −464 / −164 µs, delay 8.5–8.7
    ticks;
  - bad, seeds 3, 4, 5: 99.913 / 99.796, 99.854 / 99.883, 99.680 /
    99.912 % exact, max error 0.0000 m (misses on straight segments,
    where bridging and extrapolation are exact), clock error after 10 s
    within ±10.5 ms and at most 16 ms afterwards (under a tick), delay
    20.7–25.5 ticks;
  - world hash: 139–140 checks per run, 0 mismatches; 6 hatch events per
    direction released 0.3–13 ticks after their tick (late ones:
    reliable resends on bad);
  - upload 7,969–8,144 B/s per client.
- The oracle failed at each stage above before the fix that passed it
  (its own negative control).
- `cargo test -p sn-session`: 4 new client unit tests (clock sample
  choice and slew, the sample kinds, events held in order, own states
  every second step).
- `sn-net` scenarios also check the unordered channel: nothing
  duplicated or invented, all 1,500 arrive on lan, 95 % on bad (5 %
  loss).
- `cargo test --workspace`: 383 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- Not tested: real movement and collision (M10f), a real network
  (M10i), more than two clients (M10f: 4).

## 2026-10-11 — M10c: `sn-session`, the server's logic

**What:** new crate `crates/sn-session` (layer 2, pure; depends on
`sn-protocol`): `WorldState` (the shared state: settings, event count,
players with name, ready and cinematic; built from `Welcome` and the
ordered world events; FNV-1a hash) and `Server`, a state machine fed
connections, bytes and times, returning send/close actions and notices:
join checks with reasons, ids kept per name, `Welcome` then
`PlayerJoined`, `Ready`, states relayed per player on the latest channel
(dropped before Ready or > 5 s ahead; implausible speeds logged, not
refused), player events turned into world events in one order, leave on
`Bye`/close/timeout, `Hello` timeout 5 s, rejected connections closed
after 1 s, `Pong` with tick and fraction, `WorldHash` and `SlotFillHash`
checks. Protocol 2: `WorldHash { events, hash }` instead of a tick (the
state is a function of the event log; a player event carries its
owner's tick, so "the state at tick T" is not well defined).
`docs/formats/protocol.md` § World state and § Server rules.

**Nitrox, read (architecture only, rule 6; nothing copied):**
`PlayerMovementProcessor` (server) stores the client's position and
relays it unchecked; `PlayerMovementBroadcaster`/`LocalPlayer` send it
every frame. `EscapePodManager`: up to 50 players per pod, a new pod only
when full. Join: `MultiplayerSessionReservationRequest` → reservation
(rejected with flags: name taken, full, password, bad name, banned) →
`PlayerJoiningMultiplayerSession` → `JoiningManager` queue syncs one
joiner at a time and broadcasts the join when the joiner reports done.
Taken: both plan hypotheses confirmed (DESIGN updated), readable reasons,
others see a player after it is ready, identity by name. Not taken: the
one-at-a-time queue (our `Welcome` is the state after event N and later
events follow in order).

**Why:** M10c of the approved M10 plan.

**How verified:**
- `cargo test -p sn-session -- --nocapture`: 10 passed (3 world, 7
  server), messages through the codec both ways as on the wire. Rejects
  printed with their messages for protocol, build, fingerprint, password,
  full; names "", " Ryley", "ryley" (taken), "a\tb" refused; closed after
  the 1 s linger, not before. 8 joins get ids 0–7, the 9th is refused
  full; a goodbye and a timeout give `Left`/`TimedOut`; the timed-out
  name rejoins as id 5, a new name gets 8; a silent connection is closed
  at 5 s. States: none before Ready, not echoed, > 5 s ahead dropped, a
  50 m/s step counted implausible, the newest in a late `Welcome`. Pong
  at 3.25 ticks: tick 3, fraction 16,384.
- 10,000 random ticks: 70 joins, 28 rejects, 65 leaves, up to 8 players,
  734 events; every client's log equals the server's from its join on;
  every present client's hash equals the server's at every tick (65,525
  comparisons); 560 hash reports checked, 0 mismatches; a wrong report
  is counted.
- The random run fails when the server skips one client for some events
  (mutated, then restored).
- `cargo test --workspace`: 376 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- Not tested: a server behind a real transport (M10e), clients that
  drive their own clocks and buffers (M10d).

## 2026-10-11 — M10b: `sn-net`, the multiplayer transport

**What:** new crate `crates/sn-net` (layer 3, no dependencies): our own
thin layer on UDP (§ 9, decision 5). `Endpoint` (one socket, many peers:
handshake with a random connection id, 5 s timeout, refusal when full,
goodbye, a restarted client replacing its old connection) with two
channels per peer: reliable-ordered and latest-only per key. Acks of the
newest packet + 32 before it with the ack's delay, RFC 6298 round-trip
estimate, resend timeout with backoff, keep-alive every 250 ms, acks
within 10 ms. `Transport` with `UdpTransport` (non-blocking `std::net`;
ignores Windows' `ConnectionReset` after an ICMP unreachable) and
`sim::SimNetwork` (in memory, simulated time, seeded latency, jitter,
loss, duplication; the presets lan / internet / bad). Everything runs on
the caller's clock in microseconds. Packet format in
`docs/formats/protocol.md` § Packets.

**Why:** M10b of the approved M10 plan.

**Dead ends and fixes on the way:**
- The UDP test first resent 275 of 500 messages with no loss: a burst of
  ~70 packets, but an ack covers only the newest 33, so the older ones
  could never be acknowledged. Now at most 32 packets with reliable
  frames are in flight: UDP resends 0, lan 0 (was 80), internet 25 (was
  89, the preset loses 1 %).
- The garbage test hung: its noise socket never read its inbox, and the
  harness kept stepping to that socket's (past) arrival time. The
  simulator's `next_arrival` now only reports arrivals after the current
  time.
- An ack window shift by more than 63 would have overflowed (debug
  panic) on a sequence jump; guarded before the first run.

**How verified:**
- `cargo test -p sn-net -- --nocapture --test-threads=1`: 19 passed (10
  unit, 9 scenarios). Each preset ran 60 s of traffic (about 20 reliable
  messages a second each way, a burst of 300 at 10 s, two latest keys
  every 40 ms) and 10 s to settle: every reliable message arrived exactly
  once in order (checked by number and content), no latest message went
  back. Round trip against the configured one: lan 2.00/2.00 ms; internet
  78.6 and 77.5 / 80 ms; bad 290.4–317.6 / 300 ms over 4 seeds (all
  within the plan's 10 %). Full counters per preset in
  `docs/formats/protocol.md` § Packets.
- Cut link (100 % loss after 5 s): both ends time out 5.000050 s and
  5.000329 s after their last packet. Connecting to nobody: timed out at
  exactly 5 s after 50 requests. Full server: the second client refused;
  a restart reported as `Replaced`, a goodbye as `Closed`.
- 400,000 random datagrams (half starting like ours) at both ends of a
  live connection: 199,991 bad and 9 stray at the client, 199,991 bad at
  the server, 0 panics, the connection unaffected. Packet decoder:
  300,000 random/truncated/flipped inputs, 0 panics.
- UDP on 127.0.0.1: 500 reliable messages in 8 ms, 0 resent, round trip
  1.65 ms, latest messages delivered in order.
- The scenarios fail when the latest channel accepts an older packet
  (mutated, then restored).
- `cargo test --workspace`: 366 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- Not tested: a real network between two machines (M10i); NAT; rates far
  above the plan's (no congestion control beyond the 32-packet cap).

## 2026-10-11 — M10a: `sn-protocol`, the multiplayer messages

**What:** new crate `crates/sn-protocol` (layer 2, pure, no
dependencies): `Tick`, `PlayerId`, `PROTOCOL_VERSION` 1; client messages
(`Hello`, `Ready`, `Ping`, `States`, `Event`, `SlotFillHash`, `WorldHash`,
`Bye`) and server messages (`Reject`, `Welcome`, `Pong`, `States`, world
`Event`); a strict, canonical codec (minimal varints, bools 0/1, finite
floats, length limits, no trailing bytes; errors carry byte offsets);
quantization helpers (`quant`); the animator tables (`anim`: 7 floats,
19 bools, 3 triggers, the names `sn_sim::body` sets; a value's index is
its wire slot); FNV-1a 64 (`Fnv64`) for the fingerprint and hashes.
`docs/formats/protocol.md` describes every field. `Hello`'s first three
bytes and `Reject { Protocol }` are fixed forever so any version can
answer any other. `MAX_PLAYERS` is 8 (§ 9, decision 8). No dependency
added.

**Why:** M10a of the approved M10 plan.

**How verified:**
- `cargo test -p sn-protocol -- --nocapture`: 15 passed. Printed: one
  state 46 B (54 B with a rotation), 4 states 188 B, 8 states 356 B, the
  largest `Welcome` (8 players, longest names, every option) 862 B, all
  under 1,200 B; player upload at 25 packets/s × 4 states: 5,400 B/s
  with IPv4/UDP headers (the plan's 3–4 KB/s forgot that each state
  goes twice; DESIGN corrected). Robustness: 333,334 random, 333,333
  truncated, 333,333 bit-flipped inputs, 0 panics; the 71,216 (as
  client) and 70,073 (as server) that still decoded all re-encoded to
  the same bytes.
- Position quantization over 200,000 seeded values within ±4,096 m:
  error ≤ 2⁻¹³ m (0.12 mm), below the plan's 0.25 mm; ≤ 2⁻¹⁴ m at 2 km.
- The robustness test fails when the decoder accepts non-minimal varints
  (checked by mutating it, then restored).
- `cargo test --workspace`: 347 passed (332 + 15). `cargo clippy
  --workspace --all-targets -- -D warnings`: clean.
- Not tested: nothing sends or receives these messages yet (M10b on).
  Whether every animator name `sn_sim::body` produces is in the tables is
  checked by hand here (the 7 floats, 19 bools and `player_death` in
  `Body::update`); M10h logs any name `AnimState::set_*` refuses.

## 2026-10-11 — Plan: M10, co-op multiplayer (docs only)

**What:** `docs/DESIGN.md` § Phase D "M10 plan": an analysis of
peer-to-peer (lockstep, rollback, split ownership) against client–server
for the user, with the recommendation client–server with "host a game"
inside the client; who decides which state; ticks as the shared time
base; what a player sends; the crates (`sn-protocol`, `sn-net`, new
`sn-session`, `sn-server`); steps M10a–M10i, each with its "Done when";
how sync is proven (rules fingerprint, world-state hash, per-tick error
against the owner's own log, slot-fill hash, event ticks). Also § 3.2
(`sn-session`), § 3.3 (transport, replication, MP model), the M10 rows
in § 4.3 and Phase D, and § 9 decisions 4–8.

**Why:** the user asked for an M10 plan small enough to implement step
by step, and whether P2P would sync better. The user approved the plan
with the recommended choices the same day (§ 9, items 4–8 now decided).

**How verified:** docs only, no code. Facts the plan relies on were read
in the code: the client's physics loop steps `PlayerParams::fixed_dt` in
`sn-client` `player.rs`, collision loads on a worker thread around the
local player only, the slot fill draws per slot (`SlotRng::new(seed,
placeholder_id, slot)`). Not checked yet (marked *hypothesis* in the
plan): how Nitrox handles movement authority and the shared escape pod.

## 2026-10-10 — Fix: controls after the bottom hatch, stuck after the top hatch

**What:** two bugs the user found playing M9g5e (not yet committed).
- `sn-sim::cinematic`: `Cinematic::end_event` returns `None` when no
  cinematic runs (the game's early return). A first-use clip fires its
  end event on two layers in one step; the second call put the player
  back on the animated node, so the node's yaw (270° at the bottom
  exit) stayed on the player's transform while the look had taken it
  back: the view turned twice and W no longer moved where the camera
  looked. Callers in the client and `walk.rs` skip `None`.
- `sn-sim::player`: the walking motor pushes out of overlaps as the
  swimming one does (`CharacterController` overlap recovery,
  **hypothesis**). The top hatch's normal exit ends 0.30 m inside the
  pod's roof (its end point is VR-only), and the player could not move.
- `sn-inspect walk`/`dive`: `check_hatches` also checks that no yaw is
  left on the player after each hatch.
- `sn-client` debug flags: `--use-hatch` takes a list of times and walks
  to the hatch (side-stepping, as `walk`), `--hatch-name TEXT`,
  `--hold-forward SECONDS`; with them a per-step trace for 3 s after each
  cinematic.
- Docs: `gameplay.md`, DESIGN (M9g5e as built), README.

**How found:** reproduced in the client with the new flags. Bottom:
`--hatch-name bot_out --use-hatch 3 --hold-forward 13`: the trace showed
rotation euler (0, 270, 0) on the player with look yaw 270 after the
exit (fixed: (0, 0, 0), W moves along the look). Top: `--hatch-name top
--use-hatch 3,15,27 --hold-forward 31`: after the third use
(`top_out_trigger`, 2.30 s) the gap was −0.300 m and the player stayed
put at 3.5 m/s velocity (fixed: `PushedOut(0.454)` to y 5.67, then it
walks off the roof).

**How verified:**
- `cargo test --workspace`: 332 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean. The new walking test fails with
  the walking push-out removed (checked) and passes with it.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data --
  --ignored`: 30 passed.
- `sn-inspect walk`, `dive`, `swim`, seeds 1–5: all OK; yaw left after
  every hatch 0.000°; exactly one push-out per walk/dive run (the known
  one after the bottom first-use exit); `swim` 0 penetrations.
- **Not tested:** the hatches with keyboard and mouse after the fix
  (the user's check).

**Dead ends:** the debug driver first used the wrong hatch (the hand's
ray from the spawn hits the top hatch's trigger before the bottom one),
then stood still against the ladder (it had no side-step), then could
not use `top_in` from the roof (the ray hits the hull above the trigger;
it now uses it directly after 2 s, logged). M9g5d's scripts never
checked the player's rotation after a hatch or used the top hatches,
which is why both bugs passed them.

## 2026-10-10 — M9g5e: the hatch cinematics and the death camera in the client

**What:** last step of M9g5, and so of M9g (`docs/DESIGN.md` § 4.3, "M9g5
plan" and "as built").
- `sn-assets`: `PlayerAnimation` / `Assets::player_animation` (the
  player's compiled controller, defaults, `camAnchor` and head camera
  nodes, look limits), now shared by `sn-inspect`'s `BodyRun` and the
  client.
- `sn-client` `player.rs`: the hatches play their cinematics (pod and
  player animators run here, `Hatches::begin` / `finish`, end events,
  late update, `apply`, `ease_tilt`); the camera follows the cinematic's
  `camRoot` and, after a death, the head camera bone; the view model
  turns with the player's rotation; `--use-hatch SECONDS`, `--kill
  SECONDS` (debug). A death-camera log line each second. The unit test
  of the camera conversion now checks `pose_to_bevy`.
- `body.rs`: the cinematic's bools reach the drawn player rig and every
  drawn rig of the pod's controller.
- `main.rs`: the two flags. Docs: DESIGN, README.
- No new dependency.

**Why:** M9g5e's "Done when": cinematic duration and end pose logged;
screenshots mid-hatch (first and third person) and during death.

**How verified:**
- `cargo test --workspace`: 323 passed. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data --
  --ignored`: 30 passed. `sn-inspect walk`/`dive` seeds 1–5: RUN OK
  (after the shared loader).
- `sn-client --use-hatch 3 --shot 14 m9g5e-hatch-end`: the top
  first-use hatch, 8.72 s, ends on the roof out of the pod, first-use
  swap logged. `--use-hatch 3 --shot 6` (first and `--third-person`):
  screenshots mid-hatch. `--kill 3 --shot 10`: death camera 0.10–0.12 m
  above the player's transform, respawn, restore and controls back.
- **Not tested:** the hatches with keyboard and mouse; any of it next to
  the game on screen.

**Found:** I forgot to pass the pod's controller name to `BodyDrive` at
first, so no drawn pod rig would have opened; caught before running. The
double "first use" log line matches the game (two end events in one
frame, both handled; read in `PlayerCinematicController`).

## 2026-10-10 — M9g5d: the hatch cinematics in the scripted runs

**What:** fourth step of M9g5 (`docs/DESIGN.md` § 4.3, "M9g5 plan", the
expected values and "as built").
- `sn-inspect walk` and `dive` use the hatches through `sn-sim`'s
  cinematics (the pod's animator, `cin_target` posed every step, the
  player's "Cinematics" layer watched) instead of M9b's teleport;
  `check_hatches` checks each use's duration, end place and oxygen;
  `dive` checks its three uses (first-use exit, exit, entry) and the
  "Cinematics" states per phase. State checks match phase names exactly
  (`BodyRun::states_exact`).
- `sn-sim`: the swimming motor pushes out of overlaps (`World::push_out`,
  `Event::PushedOut`; **hypothesis**, `gameplay.md`). Unit test
  `swimming_pushes_out_of_an_overlap`.
- Docs: DESIGN (expected values corrected for the 50 Hz step, written
  before the check; "as built"), `gameplay.md` (measured durations, the
  push-out hypothesis).
- No new dependency.

**Why:** M9g5d's "Done when": each hatch use takes its clip length +
the moves, end poses next to M9b's, 0 penetrations, the expected states,
seeds 1–5.

**How verified:**
- `cargo test --workspace`: passes. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data --
  --ignored`: 30 passed.
- `sn-inspect walk --seed N` and `dive --seed N`, N = 1–5: all RUN OK;
  hatches 6.62 / 0.96 / 2.22 s on every seed; oxygen unchanged in each
  cinematic; 0 penetrations, 0 surfaces passed through.

**Dead ends / found:** the first run's durations were 0.02–0.04 s longer
than the plan's: the plan forgot that a 0.25 s move takes 13 steps at
50 Hz and that the pod's animator starts the clip one step after its
parameter is set; the expected values were corrected in DESIGN before
the checks were tightened. The first-use exit left the swimming capsule
0.27 m inside the pod on every seed (why the animation's end overlaps
the hull: not investigated); our swim step never separated overlaps, so
the push-out was added. `dive` first passed without checking its hatches at
all (it only logged them); the checks were added.

## 2026-10-10 — M9g5c: the cinematic rules

**What:** third step of M9g5 (`docs/DESIGN.md` § 4.3, "M9g5 plan").
- `sn-sim::math`: `Q` (Unity's quaternion), `Pose`, `lerp_angle`,
  `V3::lerp` (4 unit tests).
- `sn-sim::cinematic` (new): the `PlayerCinematicController` states, the
  hatch run (`Hatches::begin`, `finish`, `apply`), the look folded into
  the player's rotation and back, `Player.UpdateRotation`'s easing (5
  unit tests).
- `sn-sim::player`: `Player::rotation`, `cinematic`,
  `force_controller_size`; `HatchTrigger::cinematic`.
- `sn-sim::body`: `head_camera`, `respawned`, `view_model`, `camera` (2
  unit tests). `sn-sim::vitals`: `Situation::cinematic` (unit test
  `a_cinematic_neither_breathes_nor_refills`).
- `sn-assets`: `CinematicTrigger::cinematic_params`. The client and
  `walk` fill the new field; their behaviour is unchanged until M9g5d/e.
- Decompiled `UWE.Utils` from `Assembly-CSharp-firstpass.dll` with
  ilspycmd into the system temp folder (read only) to read
  `LerpEuler` (per-axis `Mathf.LerpAngle`).
- Docs: DESIGN row and "as built".
- No new dependency.

**Why:** M9g5 plays the hatches as the game does; these are its rules.

**How verified:**
- `cargo test --workspace`: passes (sn-sim 77 tests). `cargo clippy
  --workspace --all-targets -- -D warnings`: clean. `cargo fmt --all
  --check`: clean.
- `sn-inspect walk --seed 1` and `--seed 2`: RUN OK (unchanged path).

**Dead ends:** clippy rejected a method named `mul` on `Q`; it is now
the `*` operator.

## 2026-10-10 — M9g5b: animation events and posed nodes

**What:** second step of M9g5 (`docs/DESIGN.md` § 4.3, "M9g5 plan").
- `sn-anim`: `Animator::events` (`FiredEvent`) filled by each update
  from the playing states' clips; `event_crossed` (the rule,
  **hypothesis**, `animation.md`).
- `sn-assets`: `PosedNode`, `PosedLink`, `Prefab::posed_node` (a node's
  placement from an animator's pose).
- Tests: 4 new `sn-anim` tests (`events_are_crossed_once_per_pass`,
  `a_last_frame_event_fires_once_at_the_end`,
  `a_looping_event_fires_every_loop`,
  `events_of_the_next_state_fire_during_a_transition`), `sn-assets` unit
  test `a_posed_chain_takes_the_animated_values`, real-data test
  `lifepod_hatch_cinematic_events`.
- Docs: `animation.md` § Animation events, DESIGN row and "as built".
- No new dependency.

**Why:** the hatch cinematics end on a clip event and move the player
along an animated node of the pod.

**How verified:**
- `cargo test --workspace`: passes (sn-anim 24 animator tests).
  `cargo clippy --workspace --all-targets -- -D warnings`: clean.
  `cargo fmt --all --check`: clean.
- Real data: `lifepod_hatch_cinematic_events` passes: the stored pose
  gives `cin_target`'s stored place (within 1e-4 m); each hatch's end
  event fires one frame after its clip length; `cin_target` moves
  1.87–7.13 m (e.g. `top_out` from y 1.92 inside to y 5.22 on top).

**Dead ends:** the first test run expected exactly one end event; the
first-use hatches give two in the same frame (two layers play clips
with it). Kept as Unity does it; the test now asks for one frame.

## 2026-10-10 — M9g5a: the hatch cinematics' data

**What:** first step of M9g5 (`docs/DESIGN.md` § 4.3, "M9g5 plan",
written before this step).
- `sn-unity`: `CinematicEndForward`, `CameraToPlayerManager` (unit test
  `cinematic_end_forward_and_head_camera`), `PlayerFields::cam_anchor`
  (test `player_fields` checks it).
- `sn-assets`: `CinematicTrigger` gains `cinematic`, `cinematic_key`,
  `animated_node`, `animator_node`; `Scene::cinematic_forwards`,
  `Scene::locate_transform` public; `PlayerBody::cam_anchor_node`,
  `head_camera_node`; types `CinematicForward`, `ObjectKey`.
- `sn-inspect scene NAME --cinematics` (new), `player --body` prints the
  camera anchor and the death camera bone.
- Docs: DESIGN plan, row and "as built"; `gameplay.md` (cinematic and
  death camera facts).
- No new dependency.

**Why:** M9g5 plays the hatches as the game does; this is the data it
needs.

**How verified:**
- `cargo test --workspace`: passes. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean. `cargo fmt --all --check`: clean.
- Real data (`--ignored`): `player_movement_data` (8 triggers all on
  `cin_target` of `Life_Pod_damaged_03`, 0.25 s in and out, one
  forwarder listing all 8, enter at start and exit at end, the end
  events at each hatch clip's last frame: `bot_in` 1.667, `bot_out`
  0.667, `escapepod_first_botout` 6.333, `escapepod_first_topout`
  8.167, `side`/`side2` 2.667, `top_in` 1.167, `top_out` 2.0 s) and
  `player_body` (`Cam`, `cam_deathpos`) pass, with the other lifepod
  and player tests.

**Dead ends:** the first test version required every clip's end event
at its last frame; the intro's `escapepod_full` has it mid-clip. The
check now covers the hatch clips only.

## 2026-10-10 — M9g4: the player's body in the client

**What:** fourth step of M9g (`docs/DESIGN.md` § 4.3, "M9g plan" and "as
built").
- `objects.rs`: the worker loads `Assets::player_body` and sends it as
  one prefab (`Update::Body`); the client spawns it once and tags its
  rig's base (`PlayerBodyRig`, hidden until the player exists). The
  main camera's near plane is sent (`Update::Camera`) and applied.
  "Shadows only" renderers go on `SHADOW_ONLY_LAYER` (1) with a
  `ShadowsOnly` marker.
- `body.rs` (new): `BodyDrive` (what the player system hands over) and
  the `drive` system: the rig's base at the view model, the animator's
  parameters set, a status line every 10 s (parts drawn, shadows-only
  parts off the camera's layer and in a shadow pass, base layer state,
  rotations moved, skin 0's joint matrix spread, node bounds, CPU time).
- `player.rs`: `sn_sim::body` per frame and per physics step (jump, land
  and death events); the camera and the hand's ray at the game's eye;
  `--third-person` camera; starting pitch; `play_seconds`. New unit test
  `pose_rotation_matches_the_rules` (the camera rig's Unity rotation and
  the view model's yaw-only turn against `CameraPose::rotate`).
- `main.rs`: `--third-person`, `--look-down DEG`, `--shot SECONDS NAME`
  (save `out/NAME.png` after that much play, then exit); the sun sees
  layers 0 and 1; `body::drive` between `player::update` and `animate`.
- Docs: DESIGN row and "as built", `gameplay.md` (shadow modes, the
  body's rig numbers), README.
- No new dependency.

**Why:** M9g's "Done when" for this step: body nodes, bones and models
counted, head only in the shadow pass (counted), CPU time logged,
screenshots in first and third person.

**How verified:**
- `cargo test --workspace`: passes. `cargo clippy --workspace
  --all-targets -- -D warnings`: clean. `cargo fmt --all`: applied.
- `sn-client --look-down 85 --shot 12 m9g4-pod-down` (in the pod): body
  134 nodes, 3 renderers drawn (diveSuit body, hands, head), 4
  sub-meshes all skinned on the rig, 1 shadows only; rig 113 nodes, 213
  slots, skins of 43 / 36 / 5 bones, culling mode 1; near plane 0.03 m
  (was 0.1); base layer `Walking`; head off the camera's layer and in a
  light's shadow pass; animated nodes down to 1.53 m below the view
  model (capsule bottom 1.50 m); rules and drive 11–13 µs per frame.
  Screenshot: legs, boots and hands on the pod floor.
- `--start -115 -6 -40 --look-down 45 --shot 12 m9g4-swim-down`: base
  layer `Swim`, 63 of 71 rotations moved; screenshot shows both hands
  treading water in front of the camera.
- `--start -115 -6 -40 --third-person --look-down 20 --shot 12` and
  `--shot 13.3`: the head drawn; the two shots' poses differ (arms and a
  leg moved), so the GPU skinning follows the animator.
- `--benchmark 120` (fly camera, no body): 0 warnings, mean 26.75 ms
  (M7f4g's clean run 25.0 ms; this machine's runs vary by several ms;
  not A/B'd against the committed build).
- **Not tested:** the body's motion against the game side by side (for
  the user); playing it with keyboard and mouse; looking down 60° in
  the pod shows no body at all, not compared with the game.

**Dead ends:**
- The first third-person shot from the pod showed only the pod's wall
  (the camera is 2.5 m behind the eye and has no collision); shots are
  taken in open water instead.
- The first open-water shot looked like a bind pose (arms straight
  out). Joint matrices, the animator's state and moved rotations were
  logged: the animator was in `Swim` and skin 0's joint matrices
  differed by 1.5 m, so the mesh was not drawn rigidly. A second shot
  1.3 s later showed a different pose: arms out is the game's swim idle.
  An A/B with `--no-animation` does not work for this: it builds no
  rigs, so no body is spawned.

## 2026-10-10 — M9g3: the body in the scripted runs

**What:** third step of M9g (`docs/DESIGN.md` § 4.3, "M9g plan", the
expected states and "as built").
- `sn-inspect`: `body_run.rs` (new): the body's rules set the player's
  animator every physics step in `walk` and `dive`; the base and Death
  layers' states are collected per phase and checked against the
  expected states written in DESIGN before the run; NaNs, quaternion
  lengths, unknown parameters and cost logged. The hand's ray starts at
  the game's eye (M9g2). `anim --states` prints the selectors.
- `sn-anim`: an exit node with no transition that holds goes on to the
  layer's exit (**hypothesis**; unit test
  `unmatched_sub_machine_exit_goes_to_the_layer_exit`).
- `sn-sim`: `Player::walk_grounded` (`GroundMotor.IsGrounded()`; unit
  test `walk_grounded_is_the_walking_motors_own_flag`).
- Docs: `animation.md` (selectors), `gameplay.md` (the walking motor's
  flag), DESIGN expected states and "as built".
- No new dependency.

**Why:** M9g's "Done when" for this step: each phase reaches its states,
no NaN, unit quaternions, cost logged.

**How verified:**
- `cargo test --workspace`: passes (sn-anim 20 animator tests, sn-sim 65).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data -- --ignored
  player_`: 4 passed (including the player's and lifepod's animators run
  60 s, with the new exit rule).
- `sn-inspect walk --seed N` and `dive --seed N`, N = 1–5: all RUN OK;
  every state check ok; 0 NaNs; worst |q| − 1 1.2e-7; no unknown
  parameter; body 54–60 µs per step (mean).

**Dead ends / found:** the first runs failed twice, each time on
something our code did differently from the game: the player stayed in
`Swim` after boarding (the sub-machine exit rule above), and after the
respawn the body played the falling animation (our `grounded` is not
the walking motor's flag). The dive's time to empty moved by one breath
(42.26 → 45.18 s on every seed) because the hatch is now used 0.08 s
later from the game's eye; still within the one-breath check. Checked by
running the committed build's `dive --seed 1` against this one.

## 2026-10-10 — M9g2: the body's rules (animator values, camera rig)

**What:** second step of M9g (`docs/DESIGN.md` § 4.3, "M9g plan" and "as
built").
- `sn-sim::body` (new): `Body` (`fixed_step`, `update`, `jumped`,
  `landed`, `died`), `BodyParams`, `BodyFrame`, `CameraPose` (`rotate`,
  `inverse_rotate`, `eye`, `camera_pitch`), `AnimValue`; helpers
  `move_towards` (`UWE.Utils.Slerp` for floats), `delta_angle`,
  `vector_slerp`.
- `sn-unity`: `AutoParent` reader. `sn-assets`: `PlayerBody` gains
  `camera_offset_node`, `camera`, `main_camera_parent`,
  `main_camera_in_object` and `body_params`; `Assets::main_camera_node`.
- `sn-inspect player --body` prints the camera chain and the eye.
- Docs: `gameplay.md` § The player's body (the rules, the eye; M9c's
  camera place corrected), DESIGN row and "as built".
- No new dependency.

**Why:** M9g3 (the scripts) and M9g4 (the client) drive the player's
animator and camera with these rules.

**How verified:**
- `cargo test --workspace`: passes (new: 12 `body` tests: helpers,
  smoothing in the view frame, walking ignores pitch, the yaw rate and
  its wrap, the falling delay and jump, dive scans, the surface band,
  bob ranges −0.04 at rest and −0.34 at speed, the landing bob, strafe
  tilt settling at ∓9.92°, the eye's pivots, the death trigger; in
  `sn-assets` the parameter set equals the lists; `auto_parent`).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data -- --ignored
  player_`: 4 passed (`player_body` now also checks the body numbers and
  that the main camera hangs on `cameraOffsetTransform`).
- `sn-inspect player --body`: eye at rest 0, 0.0628, −0.15 m; errors 0.

**Found:** M9g1's dead end resolved: `AutoParent` on `PlayerCameras`
moves the main camera under `pdaCamPivot` at start. M9c's camera place
(the player's transform) was 6.3 cm low and 15 cm forward of the
game's; not changed in the client yet (M9g4). Not ported: Unity's
damped `SetFloat` (the game's damp time is 0).

## 2026-10-10 — M9g1: the player's body, headless

**What:** first step of M9g (`docs/DESIGN.md` § 4.3, "M9g plan" and "as
built").
- `sn-unity`: `PlayerFields` reads every field (new: `equipment_models`
  with `EquipmentType`/`EquipmentModel`, `head`, `cam_root`,
  `arms_controller`, `player_animator`, `left_hand_bone`; replaces
  `equipment_slots`) and fails on leftover bytes; new `ArmsController`;
  `MainCameraControl::view_model`; `PPtr::component_game_object`.
- `sn-assets` (`player_body.rs`, new): `Assets::player_body`,
  `PlayerBody::equip` and `equipment_changes` (`Player.EquipmentChanged`),
  `RULE_PARAMETERS` / `FIXED_PARAMETERS`, `Assets::shader_name`.
- `sn-inspect player --body`.
- Docs: `docs/formats/gameplay.md` § The player's body; DESIGN row and
  "as built"; README.
- No new dependency.

**Why:** M9g needs the player's model, which suit shows, the head, the
camera's nodes and the animator's parameter rules before the rules
(M9g2) and the client (M9g4).

**How verified:**
- `cargo test --workspace`: passes (new: `player_fields` extended,
  `arms_controller`, `main_camera_control` checks `view_model`, three
  `player_body` tests of the equipment rule).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `SUBNAUTICA_DIR=... cargo test -p sn-assets --test real_data -- --ignored
  player_`: 4 passed (new `player_body`: 134 nodes, slots 4 / 2 / 3, 3
  drawn renderers all MarmosetUBER, head "shadows only", view model and
  camera at the origin, look-up pivot at (0, 0.063, -0.15), arms 10 / 15 /
  0 / 0, 201 parameters with all 29 of ours present, equipping a body
  model swaps it with the default).
- `sn-inspect player --body`: errors 0, 1.2 s.

**Found:** the scene's `ArmsController` smoothing (10 / 15) differs from
the code's initial values (4 / 8). `cameraUPTransform` is not at the
camera's origin (M9c assumed the eye at `camRoot`; M9g2 measures it).
Dead end: searching the decompiled `SNCameraRoot`, `MainCameraControl`,
`Player` and `CameraToPlayerManager` did not show how
`PlayerCameras/MainCamera` follows the player.

## 2026-10-10 — Plan: M9g, the player's body; M7f4h moved after M11

**What:** `docs/DESIGN.md` only. New M9g row in Phase E and "M9g plan"
(§ 4.3, after M9c's): the player's model from the `main` scene, the
equipment rule picking the suit, the animator driven by the game's
parameter rules, the view model, the head drawn only in shadows, then the
hatch cinematics and death animation (M9g1–M9g5). M7f4h (the floating
lifepod) moved after M11, as the first rigid body with a simulation owner.
M10 now plans remote players as animated bodies. "After Phase E" item 3
points to M9g. The status line at the top was stale (said M1–M8a) and is
updated.

**Why:** the user wants the player's model and animations before
multiplayer. The animation work (M7f4a–d) and the player's movement state
(M9b, M9c) are there; what is missing was found today: the client never
spawns the `main` scene's `Player`; every suit is active in the stored
scene (`Player.equipmentModels` picks one at run time, not read yet); the
head is "shadows only" in first person (M7f4e draws that mode as normal);
nothing sets the animator's parameters (`ArmsController`); Unity's damped
`SetFloat` is missing in `sn-anim`. The arms' IK (FinalIK) is at weight 0
with empty hands, so it waits for the tools (**hypothesis** about the
plugin). M7f4h is not needed for the body and would be redone for co-op.

**How checked:** `sn-inspect scene main --tree 6` (the player's hierarchy,
output in `out/`); the decompiled `ArmsController`, `Player`,
`MainCameraControl`, `EscapePod` read (in `out/`, not committed); Bevy
0.19.1's `check_dir_light_mesh_visibility` read for shadow casters by
render layer. No code changed; nothing run besides the inspector.

## 2026-10-10 — M7f4g: Lifepod 5's own light

**What:** seventh step of M7f4 (`docs/DESIGN.md` § 4.4, plan and "as
built").
- `sn-unity`: readers for `LightingController` (with `MultiStatesSky`,
  `MultiStatesLight`), `MarmoLifepodSky.anchorSky` and
  `EscapePodCinematicControl` (its lights animator, hatch light, sky
  curve).
- `sn-assets` (`lifepod_light.rs`, new): `Scene::lifepod_lighting` (the
  anchor sky with its world rotation, the controller, its lights with
  world placements) and `Scene::stop_pod_intro` (what `StopAll` switches
  off).
- `sn-sim::lighting` (new): the game's `LightingController` as a pure
  state machine (`Timer`, `SnapToState`, `LerpToState`, `Update`).
- `sn-client`: sky keys `Fixed | Global | Pod` for the per-sky materials;
  `Global` and `Pod` materials (and the grass) relit in place when the
  player's `in_pod` changes or the controller moves the pod sky;
  `lifepod_light.rs` (new) runs the controller every frame, follows
  `in_pod` (fly camera: in at the start, out past 15 m) and drives the
  three red lamps. The pod's spawned modules take the pod's sky. Flag
  `--lifepod-state operational | danger | damaged` (default damaged).
  `PlayerSim::in_pod`.
- Docs: `docs/formats/gameplay.md` § The lifepod's light; DESIGN plan,
  "as built", not-1:1 list, `_UwePowerLoss` added to M8c7b; README.
- No new dependency.

**Why:** M7f4 plan (the pod's interior was lit by the outside sky; its
lights and lighting states were not read).

**Findings:**
- The escapepod scene has no `SkyApplier`: the pod is lit by the global
  sky, which `MarmoLifepodSky` makes the pod's own while the player is
  inside (and then everything without an applier takes it).
- Stored: state Operational, fade 1 s; sky `SkyEscapePod` master /
  diffuse / specular 10 / 2 / 1.5 (= Operational), per state 10 / 0.8 /
  2.5, 2 / 0.5 / 0.8, 1.5 / 3 / 1; two red spots 0 / 1.25 / 0, a red point
  0 / 0.22 / 0; emissive 0 / 1 / 1.
- A new game ends the intro, played or skipped, in state 2 (Damaged).
- Quirk in the game's code: `LerpToState(s, 5)` from another script
  becomes a `fadeDuration` (1 s) fade on the next frame (`Update` restarts
  it because `prevState` differs).

**Dead ends:**
- The first benchmark showed 42.9 ms against M7f4f's 25.0 ms. It was the
  machine: other programs used 27–40 % CPU, and the committed M7f4f build
  run alternately measured 46.7 and 45.0 ms against this build's 43.0
  and 29.6 ms. Built in a scratch worktree under `out/` (removed after).
- The first log only printed while a fade was running, so the settled
  values were never logged; now logged once settled.

**Verified (2026-10-10):**
- `cargo test --workspace` passes (new: 7 `sn_sim::lighting` tests,
  `lighting_controller_and_lifepod_sky` in `sn-unity/tests/objects.rs`);
  `cargo clippy --workspace --all-targets` and `cargo fmt --all --check`
  clean.
- Real data (`--ignored`): 28 tests in `sn-assets` pass, new
  `lifepod_sky_and_lighting_controller`.
- `sn-client --benchmark 300` (default): global sky `SkyEscapePod` at the
  start (161 materials relit in 0.05 ms), sky settled at 2.5 / 0.8 / 1,
  lamps off. `--lifepod-state danger`: 0.8 / 0.5 / 3, lamps on at 1.25,
  1.25, 0.22; `operational`: 10 / 2 / 1.5, lamps off. Screenshots
  `out/m7f4g-{damaged,danger,operational}.png` differ as expected (red
  lamps in Danger, dim in Damaged, bright in Operational).
- `--flythrough -127.7 2 -10` out of the pod: "player out of the pod:
  global sky SkySafeShallows (842 materials relit in 0.36 ms)".
- Entering through a hatch with the player: not tested in the client
  (same code path as the start; the hatch's `in_pod` is unit-tested in
  `sn-sim`).
- Not compared with the game on screen.

## 2026-10-10 — M7f4f: levels of detail by distance, the game's field of view

**What:** sixth step of M7f4 (`docs/DESIGN.md` § 4.4, "as built").
- `sn-unity`: `LODGroup` read fully (reference point, size, fade mode,
  cross-fading, each level's height and fade width); `quality.rs` (new):
  `QualitySettings` (each level's name, shadow settings, `lodBias`,
  `maximumLODLevel`).
- `sn-dotnet`: `static_initializer_f32` (a static field's `ldc.r4 v;
  stsfld` in `.cctor`, exactly once).
- `sn-assets`: `quality_settings`, `field_of_view_code`
  (`MiscSettings.fieldOfView`); `Prefab::lod_groups` (enabled groups),
  `PrefabNode::lod_group` (its group and the levels listing it),
  `lod_conflicts`; `Prefab::drawn` now yields every level's nodes;
  `PrefabLodGroup::world`.
- `sn-world::lod` (new, pure): `relative_height`, `level`.
- `sn-client`: every level spawned, the level shown chosen per instance
  per frame (hidden parts hidden; the shadow pass sees the same choice);
  scene instances under one root entity each; the camera's vertical field
  of view 60° (was Bevy's 45°); LOD counts, switches, triangles and the
  switch's cost logged. Flags `--no-lod`, `--fov <deg>`.
- `sn-inspect lods` (new command).
- Docs: `docs/formats/unity.md` § LOD groups and quality settings.
- No new dependency.

**Why:** M7f4 plan (everything was drawn at LOD 0, and the camera's field
of view was not the game's).

**Findings:**
- 3,692 enabled groups in the placed prefabs (76,854 placements), none
  cross-fading. Quality levels: Low `lodBias` 0.66, Medium 1, High 10;
  `maximumLODLevel` 0 in all (checked against the UnityPy oracle).
- `MiscSettings.fieldOfView` = 60 (vertical, degrees).
- At bias 10 most groups stay at LOD 0 to about 50 m; some are culled
  from about 200 m.

**Dead ends:**
- None that cost time.

**Not 1:1 (listed in DESIGN "as built"):** the rule is Unity's documented
one, not compared with the game; High quality and 60° assumed (the
player's saved options not read); hidden levels spawned up front (a cost
the game does not have).

**Verified (2026-10-10):**
- `cargo test --workspace` passes (new: 2 `sn_world::lod` tests, 2
  `quality` reader tests, `lod_group` in `sn-unity/tests/objects.rs`);
  `cargo clippy --workspace --all-targets` and `cargo fmt --all` clean.
- Real data (`--ignored`): 27 tests in `sn-assets` pass, new
  `lod_settings_field_of_view_and_groups`.
- `sn-client --benchmark 300` from the lifepod: 8,208 groups, by level
  5,861 / 2,039 / 66 / 176 / 2, 64 culled; mean 25.0 ms against 24.8 and
  24.5 ms with `--no-lod` (noise about ±1 ms). Looking at the Aurora:
  from 500 m 16.9 ms (`--no-lod` 19.7), from 1,500 m 15.1 ms (16.6).
- The Aurora's swap still happens with the scene under its root entity
  (`aurora: models swapped at game time 532.39 s`).
- Not compared with the game on screen.

## 2026-10-10 — M7f4e: the Aurora on the game clock, its exterior cull, shadow flags

**What:** fifth step of M7f4 (`docs/DESIGN.md` § 4.4, "as built").
- `sn-dotnet`: `random_range_f32` (the one `Random.Range(a, b)` in a
  method, both float constants) and `multiplier_f32` (the one
  `ldc.r4 v; mul`), plus the `mul` opcode.
- `sn-unity`: `ShipExteriorCullManager` and `ShipExteriorCull` readers.
- `sn-assets`: `exploder_code` (the countdown rule and delays from the
  DLL); `PrefabNode.cast_shadows`; `Prefab.exterior_culls` (each script's
  box colliders); `Scene::exploder`, `ship_exterior_cull_manager`,
  `aurora_groups` (the Aurora's parts by the states they show in);
  `Prefab` and `PrefabNode` are `Clone`.
- `sn-sim::aurora` (new): the exploder (countdown, the `else if` event
  chain with "just went above", `IsExploded`, the cull's
  initialised-and-deserialised condition), the manager's frame rule, the
  strict oriented box test.
- `sn-client`: `GameClock` (from `--time`, `--time-scale`), `aurora.rs`
  (the exploder on the clock, swap and cull applied to the scene's parts,
  events logged); rooms with `ShipExteriorCull` spawn an
  `ExteriorCullVolume` with their world boxes; renderers with
  `m_CastShadows` off get `NotShadowCaster` and keep it when their
  batch's shadow switch changes. Flags: `--aurora-countdown <s>`,
  `--time-scale <x>`; `--aurora intact | exploded` now holds the state.
- `sn-inspect aurora`; `prefab::placed_transforms`.
- Docs: `docs/formats/gameplay.md` § The Aurora's explosion, § Shadow
  casting modes.
- No new dependency.

**Why:** M7f4 plan (the Aurora was a flag, its cull and the renderers'
shadow flags were not read).

**Findings:**
- Code: `Random.Range(2.3, 4) × 1200` s after the start; sound +24, explosion
  +25, swap +27 s. Our seed 1 draws 4,500 s.
- The exterior cull never acts in a new game (it needs the exploder both
  initialised and loaded from a save). The "exterior" is the whole wreck.
- 2 of 3,336 world prefabs have `ShipExteriorCull` (cargo room 4 boxes,
  Prawn bay 3), each placed once; the manager checks every 10 frames.
- Drawn renderers with shadows off: 5,380 in placed prefabs (29,145
  placements), 195 in the Aurora scene; 1 shadows-only; 0 two-sided.

**Dead ends:**
- None that cost time. A first census over placed prefabs only was
  widened to every world prefab (placeholders could bring in unplaced
  ones); the result was the same two rooms.

**Verified (2026-10-10):**
- `cargo test --workspace` passes (new: 7 `sn_sim::aurora` tests,
  `reads_random_ranges_and_multipliers` in `sn-dotnet`,
  `ship_exterior_cull` reader test in `sn-unity`); `cargo clippy
  --workspace --all-targets` and `cargo fmt --all --check` clean.
- Real data (`--ignored`): `aurora_clock_and_exterior_cull_volumes`
  (code numbers, manager every 10 frames, 330/337 nodes drawn by groups
  equal to the whole scene swapped, 18 exterior nodes, 7 boxes) and
  `startup_scenes_and_aurora` pass.
- `sn-client --benchmark 300` from the lifepod: countdown at 4,982.7 s
  (clock 483.1 s when the scene loaded + 4,499.6 s), mean 21.00 ms (M7f4d:
  21.06 ms); 0 warnings.
- Swap pair (`--free-cam --start 500 60 350 --look 1000 0 0 --benchmark
  120`): `--aurora intact` shows the intact ship; `--aurora-countdown 0
  --time-scale 10` logs the countdown at 498.39 s, the explosion at
  523.61 s, the swap at 525.36 s, and the screenshot shows the wreck
  (`out/m7f4e-intact.png`, `out/m7f4e-swap.png`). The sound event was
  skipped there: one frame jumped past both 522.3 and 523.3 s, which the
  game's chain does too.
- Camera in the cargo room (`--start 944 18.5 17.3 --aurora exploded`):
  "camera entered a ship exterior cull volume (7 registered)", exterior
  stays shown (the game's rule).
- **Not checked:** the swap moment against the game; whether a loaded
  save culls the exterior (needs saves, M9f); shadows of "off" renderers
  compared with the game on screen.

## 2026-10-10 — M7f4d: blend shapes

**What:** fourth step of M7f4 (`docs/DESIGN.md` § 4.4, "as built").
- `sn-unity/src/blend_shape.rs` (new): `Mesh.m_Shapes` read (offsets,
  frames, channels, full weights; every range checked against the
  mesh's vertex count) instead of skipped; Unity's frame-weight rule
  (`channel_frame_factors`); weights applied to decoded geometry;
  `PlayerSettings.legacyClampBlendShapeWeights` read from the object's
  last four bytes.
- `sn-assets`: stored weights applied before skinning for meshes drawn
  in their stored pose; `blend_shape_clamp`, `blend_shape_names`,
  `mesh_info` (a mesh without its geometry). Animator defaults of blend
  shape slots now find their channel (CRC-32 of the name alone).
- `sn-client`: rigs also for animators that drive only blend shapes;
  driven renderers get one Bevy morph target per frame and
  `MeshMorphWeights` written every frame (clamped 0–100, as the game);
  bone-less skinned renderers hang on their rig node; morphed bone-less
  meshes get fixed bounds holding every shape. Logs: shapes per rig,
  blend shape parts weighted per frame.
- `sn-inspect prefab --shapes`; `anim` lists missing binding path hashes.
- Docs: `docs/formats/unity.md` § Blend shapes (new),
  `docs/formats/animation.md` (the binding attribute).
- No new dependency.

**Why:** M7f4 plan; the player's body and creatures use blend shapes.

**Findings:**
- 16,019 meshes, 83 with blend shapes: 319 channels, one frame each, full
  weight 100, all with normals and tangents; 1,080,876 offsets.
- Placed prefabs and the escape pod: 65 drawn renderers with blend shapes
  (7,270 placements), 665 channels, **every stored weight 0**; 22
  renderers (6,800 placements) have 102 channels driven by an animator.
- Unclamped animated weights run from −91.4 to 102.9 over 30 s; the
  project's clamp flag is set, so the game shows 0–100.

**Dead ends:**
- M7f4a–c matched blend shape bindings against
  `CRC-32("blendShape.<name>")`; nothing matched. The attribute is the
  CRC-32 of the channel name alone (the Bleeder's three, then every
  binding that reaches a renderer with shapes).
- The UnityPy oracle in temp had lost its `__init__.py`; reinstalled
  UnityPy 1.25.4 into a fresh temp folder.
- Python's text mode wrote CRLF into some edited files; set back to LF.

**Verified (2026-10-10):**
- `cargo test --workspace` passes (new: 7 `blend_shape` tests on
  synthetic bytes — read, truncation, ranges, frame factors, apply,
  dense frames, the clamp flag — and `morph_weights_follow_the_slots` in
  `sn-client`); `cargo clippy --workspace --all-targets` and
  `cargo fmt --all --check` clean.
- Real data (`--ignored`): `every_mesh_parses_with_its_blend_shapes`
  (counts above, equal to UnityPy's), `blend_shape_clamp_and_brain_coral`
  (flag set, as UnityPy reads it; `BrainCoral` LOD 0: 3 channels, stored
  at 0). `sn-inspect prefab --shapes` logs `BrainCoral` LOD 0's bounds
  with and without its stored weights (the same: all 0); 0 errors.
- `sn-client --benchmark 300` from the lifepod: 1,121 rigs (58 before),
  468 blend shape parts weighted per frame, animation 0.5–0.7 ms per
  frame; mean 21.06 ms vs 20.01 ms with `--no-animation`; no warnings.
- **Not checked:** the motion on screen against the game (the user);
  GPU-skinned meshes whose shapes reach past their bone bounds
  (creatures, not placed yet).

## 2026-10-10 — M7f4c: animated objects in the client

**What:** third step of M7f4 (`docs/DESIGN.md` § 4.4, "as built").
- `sn-client/src/animation.rs` (new): `RigDesc` (an animator of a
  prefab: its nodes, compiled program, slot → node, defaults, culling
  mode, skins), the `AnimatedRig` component and the `animate` system
  (each frame: `Animator::update`, then the animated Transforms written;
  the game's culling modes from the parts' visibility), Unity → Bevy
  bind pose conversion (`S·M·S`).
- `objects.rs`: the worker compiles each controller once (shared clips)
  and builds rigs for animators that move a Transform; parts below an
  animator hang on their rig node; skinned meshes whose bones are all
  in one rig are sent with bone weights (normalised, at most 256 bones)
  and drawn with Bevy's GPU skinning and `DynamicSkinnedMeshBounds`.
  Rigs of placeholder-spawned prefabs come along with their parts.
- `sn-client --no-animation` (everything in its stored pose); a log line
  per animated prefab and an `animation:` stats line every 2 s.
- `sn-inspect anim --placed`: animators over every placed prefab.
- New dependency: `sn-client` and `sn-assets` use `sn-anim` (our crate).

**Why:** M7f4 plan; the player's body will use the same rigs.

**Findings:**
- 82 placed prefabs (9,252 placements) run an animator; their drawn
  parts are 11,042 skinned and 109 rigid. Many animate only blend
  shapes (the 3,299 small deco corals, the jewelled disks), the anchor
  pods' controllers animate nothing. All placements at once would cost
  6.7 ms per frame on one thread.
- At the lifepod (seed 1), 58 rigs load (the pod's two hull LODs, eggs,
  wreck doors, Precursor terminals and teleporters in the loaded
  batches); 17 are updated per frame (the rest culled), 80–150 µs per
  frame, worst 562 µs while loading.

**Dead ends:**
- Rig node entities without `Visibility` made Bevy warn (B0004) for
  every mesh below them; they now carry `Visibility`.
- The first screenshot pair (the default start, inside the pod) showed
  no difference (1 pixel): the parts the animator moves are not in that
  view. The outside view is used instead.

**Verified (2026-10-10):**
- `cargo test --workspace` passes (including 2 new `sn-client` tests:
  the bind pose conversion against mirrored points, slot rotations);
  `cargo clippy --workspace --all-targets` and `cargo fmt --check` clean.
- `sn-client --benchmark 300` from the lifepod: mean 20.06 ms; with
  `--no-animation` 20.21 ms (no measurable cost). No warnings.
- Outside view (`--start -121 4.5 -44 --look -127.7 1.5 -49.7
  --benchmark 120`, with and without `--no-animation`): the hull's 23
  GPU-skinned meshes draw where M7f2's CPU-skinned copy did (no torn or
  displaced geometry; differences only in the moving waves). Screenshots
  `out/m7f4/outside-anim.png`, `outside-noanim.png`.
- **Not checked:** motion against the game on screen (the user); the
  animated parts inside the pod (hatch joints, storage door, panels)
  are not visible from the views taken.

## 2026-10-10 — M7f4b: the animation runtime (`sn-anim`)

**What:** second step of M7f4 (`docs/DESIGN.md` § 4.4). New crate
`crates/sn-anim` (layer 2, pure; uses `sn-unity`'s data types only):
- `sample`: a clip's curves at a time (streamed cubic, dense linear,
  constant).
- `blend`: 1D, 2D simple directional and 2D freeform directional
  weights; nested trees.
- `Program` (a controller compiled against its clips: one slot per
  animated property, layer masks, additive references) and `Animator`
  (parameters, triggers, state machines with selectors, any-state
  transitions, exit time, durations, offsets, interruptions with a
  frozen pose, override and additive layers, skeleton masks, write
  defaults, `Animator.Play`, layer weights).
- `sn-assets`: `Prefab::bind_animator` (slots → nodes, default pose from
  the hierarchy); `sn-inspect anim <target> --play <S>`.
- Rules and what is a hypothesis: `docs/formats/animation.md` § Runtime.

**Why:** the player's model and animations (Phase E, "After Phase E"
item 3) and the creatures need the game's `Animator`.

**Findings:**
- An empty state writes nothing. The player's arm layers wait in empty
  states at weight 1 while the base layer's idle moves the arms; with
  defaults written there the arms stay in the stored pose.
- The freeform trees' stored pair vectors are `(−angle, (|pᵢ| − |pⱼ|) ·
  2/(|pᵢ| + |pⱼ|))`: gradient bands in polar space.
- The player in its default states: "Walking" plays `player_view_idle`;
  69 slots move (arms, fingers; up to 100° at the right hand's IK
  target). The lifepod: 35 slots move from the stored pose into the idle
  and closed states (hatch attach joints, storage door, wall panels).

**Dead ends:**
- First version: empty states with write defaults put the defaults back
  on every slot of their layer, so 0 of the player's slots moved in 20 s
  of idle although the idle clip's curves vary (173 curves). Fixed as
  above, with a test.
- One interruption test first assumed a zero-length transition blends;
  it switches at once (the test was wrong, not the rule).

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game (`sn-anim`: 8 unit +
  19 animator tests); `cargo clippy --workspace --all-targets` clean;
  `cargo fmt --check` clean.
- Real data: `cargo test --release -p sn-assets --test real_data --
  --ignored player_and_lifepod_animators_run`: passes (213 slots / 710
  values for the player, 137 / 458 for the lifepod, 0 missing; 60 s at
  60 updates/s, no NaN, |q| = 1 within 10⁻⁴).
- `sn-inspect anim scene:main --play 20`: no NaN, worst |q| − 1 1.2e-7,
  about 50 µs per update; `anim scene:escapepod --play 20`: about 11 µs.
- Not compared with the game on screen (no client yet: M7f4c).

## 2026-10-10 — M7f4a: animation data, headless; M7f4 plan

**What:** the M7f4 plan (`docs/DESIGN.md` § 4.4: eight steps, animation
first; the items moved elsewhere and closed without code are listed
there), then its first step:
- `sn-unity::anim`: `AnimationClip`, `Avatar`, `Animator`,
  `AnimatorController` (Unity 2019.4.36f1 layouts from UnityPy's type
  database; each must reach the object's last byte); `name_hash`
  (CRC-32). Round-trip tests with an encoder in the test module.
- `sn-assets`: prefab nodes keep their `Animator` (controller, avatar)
  and a skinned renderer's blend shape weights; `animation_set` loads a
  controller with its clips (shared through a cache);
  `Prefab::binding_paths`.
- `sn-inspect anim <prefab key | scene:NAME> [--states]`.
- `docs/formats/animation.md` (new).

**Why:** the user is building the player's model and animations, which
need M7f4's `Animator` work.

**Findings** (all in `docs/formats/animation.md`):
- 600 animators, 289 controllers, 334 avatars, 2,294 clips; **no
  humanoid avatar** (all generic); no override controllers.
- The streamed block is frames of keys, each a cubic up to its curve's
  next key; continuous over 5,113,013 keys except 16 stepped ones.
- Bindings' curve counts equal each clip's curves in every clip.
- Binding paths and names are CRC-32: the player's 77,373 bindings and
  the lifepod's 3,425 all match their hierarchy.

**Dead ends:** a first prototype of the census in Python counted
quaternion lengths mid-segment of up to 1 − 1/√2; that is component-wise
interpolation, not a misread (Unity normalises), so it is not a check.

**Verified (2026-10-10):**
- `cargo test -p sn-unity` (5 new tests: clip and controller round
  trips, streamed key errors, animator layout, CRC-32).
- Real data: `cargo test --release -p sn-unity --test real_data
  every_animation_object_parses -- --ignored`: 2,294 clips, 289
  controllers, 334 avatars, 600 animators, 0 errors, 0 gaps.
- `sn-inspect anim scene:main` and `anim scene:escapepod`: bindings
  found 77,373 / 3,425, missing 0 (the lifepod's LOD 1 copy misses 4
  paths that exist only on LOD 0).

## 2026-10-10 — M9c: oxygen, health, death and respawn; the game's mouse look

**What:** fifth step of Phase E (`docs/DESIGN.md` § 4.3, "M9c plan" and
"as built"). The plan was written first.
- `sn-dotnet`: `const_f32` (a `const float`'s `Constant` row) and
  `field_initializer_f32` (only `ldarg.0; ldc.r4; stfld` in `.ctor`,
  exactly once). Synthetic tests, including a field set twice and one
  set from a non-constant.
- `sn-unity`: `MainCameraControl` (every field, checked to the last
  byte); `LiveMixin` now reads every field (`startHealthPercent`,
  `player`) to the last byte.
- `sn-assets`: `player_data` gains the camera, its node and the oxygen
  source's height; `player_code` reads `OxygenManager
  .oxygenUnitsPerSecondSurface` and `GameInputSystem
  .defaultMouseSensitivity` from the DLL; `vitals_params`, `look_params`.
- `sn-sim`: `vitals` (depth classes, breaths on the game-time clock,
  refill, suffocation `Sequence`, fall damage, death → respawn timeline)
  and `look` (`MainCameraControl`'s rule). `player::Event::Landed` now
  carries the impact velocity.
- `sn-inspect`: `dive` (the scripted run); `player` prints the new
  values; `walk`'s pieces are shared with `dive`.
- `sn-client`: vitals run with the player; the game's look replaces our
  sensitivity and ±89° limit; a HUD of our own (`hud.rs`: oxygen and
  health bars with numbers, depth, the suffocation overlay). No new
  dependencies (Bevy's default `ui` feature was already on).
- Real-data test `vitals_and_look_data`.

**Findings** (`docs/formats/gameplay.md` § Oxygen, health and death and
§ Mouse look; `docs/formats/dotnet.md`):
- Refill 30 units/s (code only), capacity 45, 1 / 1.5 / 2 units per
  second by depth class, suffocation 8 s, recovery 4 s.
- Breaths follow game time, not time under water, so the first comes 0
  to one period after going under.
- Suffocation starts only at exactly 0 oxygen (`Utils.NearlyEqual`
  against 0 is an exact compare).
- `CrushDamageUpdate` is never called for the player.
- Mouse: 0.15 × 1.5 × 0.5 = 0.1125° per count, no smoothing. Pitch
  limits are ±87° in the scene against ±80° in the code.
- The camera sits at the player's origin (`camRoot`, `skin` 0), as M9b
  assumed.

**Dead ends:**
- Three vitals tests first ran for exactly 18 s or 30 s and lost the
  last breath to the rounding of a clock summed from 0.02 s steps. The
  tests now stop off the boundary; the rule is unchanged.
- The first dive tried to surface from under the lifepod and stayed
  there, then climbed onto the pod's roof and could not board. The
  script now swims 8 m clear of the pod first and dives 1 s before
  boarding.
- The first time-to-empty check passed by mistake. It took the depth
  class at the respawn point (inside the pod: surface, period 99999 s).
  It now uses the depth where the oxygen ran out.
- The prediction check also first allowed 99999 units (the surface
  period counted as a breath). Once tightened, it failed at 3.06: steps
  taken by the shared walking code skipped the prediction. It is now
  resynced after those steps.
- Seed 5's second dive slid along the seabed back under the pod. The
  player died a second time there, and the script used a hatch while
  dead. The script now swims clear before surfacing, and no hatch can be
  used while the controls are off.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game, including
  `sn-sim`'s 14 new tests (vitals 10, look 4), `sn-dotnet`'s new test and
  `sn-unity`'s `main_camera_control`; `cargo clippy --workspace
  --all-targets` clean.
- Real data: `cargo test --release -p sn-assets --test real_data --
  --ignored vitals_and_look_data player_movement_data game_code`: 3 pass.
- `sn-inspect dive --seed N` for N = 1–5: `RUN OK`, every check ok.
  Times as in the findings, 0 penetrations, 0 surfaces passed through,
  13–26 µs mean per step.
- `sn-inspect walk`: still `RUN OK`.
- `sn-client` launched for 45 s on the real install: it logs the values
  read and no errors. The HUD's look, the mouse look in play and a death
  in the client are not tested by the agent.

## 2026-10-10 — Plan: homes for M9b's open differences

**What:** `docs/DESIGN.md` only. Added the game's mouse look
(`MainCameraControl`: sensitivity, pitch limits, smoothing) to M9c, with
its own "Done when" check. Added "The player's animations" as item 3 of
"After Phase E" (the hatch cinematics, body and arms, camera bob; next to
Creatures, which needs the same `Animator` work); the later items moved
down by one. M9b's "Not 1:1 yet" now says where each difference is
planned.
**Why:** the user asked that every difference left by M9b have a place in
the plan.
**Verified:** docs only; `git status --short` lists only our own files.

## 2026-10-10 — M9b: the player (walk, swim, the lifepod's hatches)

**What:** fourth step of Phase E (`docs/DESIGN.md` § 4.3, "M9b plan" and
"as built").
- `sn-unity`: parsers for `GroundMotor`'s own fields, `UseableDiveHatch`,
  `CinematicModeTrigger` (with Unity's serialized event lists),
  `PlayerCinematicController` and `EscapePodFirstUse`; new `physics`
  module: `PhysicsManager` (with the layer collision matrix),
  `TimeManager`, `TagManager`. Each is checked against the exact byte
  length on real data. Synthetic tests check them on every truncation.
- `sn-assets`: `player_data` gains the walking motor, the rigid body, the
  player's layer and the ocean level; `physics_settings`; scene functions
  for the pod's triggers, dive hatches and the first-use swap; new
  `collision_world`: the batch loader moved out of `sn-inspect swim`
  (terrain, objects, placeholder spawns, layer rules), the lifepod's
  colliders and triggers, and the player's parameters. Mesh colliders
  with a mirroring scale swap their winding back (PhysX `flipsNormal`).
- `sn-sim`: `player` (state, parameters, input; one fixed step: motor
  choice at the game's water levels, swimming, walking through our own
  character controller with step offset, slope limit and ground check,
  jumping, the hand target, the hatch triggers). `collide`: triangles are
  now one-sided, bodies carry groups, plus `cast` and `crossings`.
- `sn-inspect`: `walk` (the scripted run) and `prefab --winding`;
  `player` prints the new data; `swim` uses the shared loader and counts
  surfaces passed through.
- `sn-client`: the player is the default (`--free-cam` keeps the fly
  camera). It has a first-person camera at eye height and WASD / Space /
  C / mouse controls. E or a left click uses a hatch trigger. Collision
  streams on a worker thread.
- Real-data test `player_movement_data`.

**Findings** (`docs/formats/gameplay.md` § Player movement):
- Layer 19 ("Player") collides with every layer but 9 ("OnlyVehicle"),
  and every collider loaded so far is on layer 0.
- The fixed time step is 0.02 s.
- PhysX 4.1 (Unity 2019.4.36f1) treats mesh colliders as one-sided in
  contacts, sweeps and rays. I read its BSD-3 source to check; nothing
  was copied.
- The pod is entered and left through 8 cinematic triggers. Its dive
  hatch is inactive.

**Dead ends:**
- The first controller stopped at a 0.3 m step's face. It never lifted
  the capsule, although a 0.4 m step offset should clear it. With that
  fixed, it then climbed a 0.6 m step. Both were traced in the
  controller's three passes (up, across, down) and fixed; the unit test
  covers 0.3 m (climbs) and 0.6 m (doesn't).
- The real hatch is not `UseableDiveHatch`, as the plan assumed. That
  node is inactive in the pod; the cinematic triggers do the job.
- `sn-client --benchmark` turns the player off (measuring runs keep the
  fixed camera), so the client check is a normal launch stopped after
  45 s.
- Winding rays from above deep terrain batches start inside rock and see
  back faces (up to 3,150 of 3,300 in one batch). The check uses shallow
  batches only, plus the `sn-mesh` orientation test.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game, including `sn-sim`'s
  24 tests. They cover:
  - swim speed capped and drag stopping;
  - surface damping and the motor switch at the game's water levels;
  - walking on a floor, a 0.3 m step but not 0.6 m, sliding on 70°,
    jump height;
  - hand reach, and the hatch;
  - a one-sided wall that blocks from the front only.

  `sn-unity` adds tests for the new parsers, and `sn-assets` for the
  mirrored winding.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  player_movement_data player_and_pda_data lifepod_colliders` passes.
- `cargo run --release -p sn-inspect -- walk`: RUN OK. Leaves the pod at
  1.7 s through `bot_out_trigger_first` (first-use swap logged), swims
  71 m away in 10 s and boards through `bot_in_trigger` at 21.8 s. 1,138
  steps, 0 penetrations, 0 surfaces passed through, smallest gap 5 mm,
  5.9 µs mean per step. Speeds: walking max 3.50 (read 3.5); swimming
  7.22 (7.6 read, 7.22 after one step of drag).
- `sn-inspect swim --seed 1…5`: all arrive in Kelp Forest, 0
  penetrations, 0 surfaces passed through, smallest gap 5.1–5.2 mm,
  13–19 µs mean per step.
- `sn-inspect prefab --winding`: terrain 17,066 front / 1 back; mesh
  colliders 6,012 front / 152 back.
- `sn-client` (normal launch, 45 s, no input): the player is ready 4.3 s
  after start, collision for 4 batches in 2.0 s, and it stands on the pod
  floor at (−126.79, 1.83, −49.75), grounded, as in the headless walk.
- **Not tested:** playing with keyboard and mouse in the client (the
  agent can't give input); the look limits and mouse sensitivity are our
  own.

## 2026-10-10 — M9a: collision (capsule sweep, terrain and object colliders, scripted swim)

**What:** third step of Phase E (`docs/DESIGN.md` § 4.3, "M9a plan").
- New crate `sn-sim` (layer 2, pure, no dependencies): `collide`. A
  capsule is swept against triangles, spheres, capsules and oriented boxes
  by conservative advancement on exact distances, so nothing can tunnel
  through. `move_and_slide` makes up to 4 slides and keeps a 1 cm skin.
  `clearance` and `push_out` find and fix overlaps. Bodies are added and
  removed by id, with a uniform 4 m grid in each body.
- `sn-terrain::collision_triangles`: the level 0 batch mesh without
  skirts, as Unity-space triangles.
- `sn-assets::collision`: `prefab_colliders` keeps enabled, non-trigger
  colliders on active nodes and caches mesh-collider triangles.
  `PrefabCollider::world` applies Unity's scaling rules (hypothesis).
- `sn_world::Transform::transform_point` / `rotate_vector`.
- `sn-inspect swim [--seed N] [--seconds S]`: the scripted swim, headless.
  Terrain and objects (cells, slot spawns, batch objects) are loaded for
  the batches within 56 m of the player, the game's collision range.
- Real-data test `lifepod_colliders`.

**Findings** (`docs/formats/gameplay.md` § Collision): the game collides
with terrain only at clipmap level 0 (7×7×7 chunks of 16 voxels). There
the collision mesh is thinned by a native plugin we don't port. The
underwater player capsule is radius 0.3, height 0.75, centre 0.125 m
below the camera. The lifepod has 40 box colliders.

**Dead ends:**
- The first swim stayed at −20 m and touched nothing (0 contacts), so it
  tested nothing. The swim now aims below the seabed and slides along it.
- The first target was the nearest Kelp Forest map cell. It lies on the
  forest's edge (4 m cells), and 3 m short of it is Safe Shallows. The
  target now needs Kelp Forest 20 m around it, and the end point must be
  in Kelp Forest.
- Seed 1 got wedged for 590 s in a 0.5 m slot between a terrain wall and
  an overhanging object. That was correct collision: the slot is narrower
  than the capsule. The script now detours sideways after 1 s blocked.
- Two unit tests first expected the gap to stay ≤ `SKIN`. The 0.1 %
  overbounce lifts it a few mm more (by design), so the bound is now
  2 × `SKIN`.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game. That includes
  `sn-sim`'s 12 tests: floor and wall slides, an inside corner, a thin
  wall at 100 m/s from both sides, each primitive's stopping distance, a
  rotated box, a bumpy height field, and 3,000 random moves in a closed
  room with obstacles with no penetration. Also the scaling rules,
  `world_triangles` and `transform_point`.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  lifepod_colliders` passes.
- `cargo run -p sn-inspect -- swim` (seed 1): ARRIVED in kelpForest after
  51.8 s simulated, 3,693 contacts (3,424 terrain, 269 objects), 0
  penetrations, smallest gap 5.3 mm, 30.5 µs mean / 118.9 µs p99 per
  step, batch loads about 0.6 s each. Seeds 2–5: all arrive, 0
  penetrations, smallest gap 5.1–5.3 mm.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  -- -D warnings` are clean.
- **Not tested:** anything in the client (M9b uses this); the game's own
  collision for comparison; the physics layer matrix (not read); whether
  the game's triangles are one-sided.

**Follow-up (same day, user's request):** every M9a gap now has a home in
`docs/DESIGN.md`. M9b's row gains the physics layer matrix, the lifepod
module and placeholder colliders, and the one- or two-sided check (each
with a "done when" check). The terrain thinning plugin, the scaling check
and PhysX rigid-body response join the "Deferred, not dropped" list.

## 2026-10-10 — P1: our own .NET reader; tech type names, craft menus, TechData defaults

**What:** second step of Phase E (`docs/DESIGN.md` § 4.3).
- New crate `sn-dotnet` (layer 1, pure, no dependencies): PE container,
  CLI metadata (`#~` tables with the full ECMA-335 schema, `#Strings`,
  `#US`, `#Blob`), method bodies, an IL decoder for every opcode, token
  resolution (MethodDef, Field, MemberRef, TypeDef/TypeRef/TypeSpec).
  Game readers: `enum_values` / `tech_type_names`, `craft_trees` (runs the
  scheme methods on a small value stack, only the instructions they use),
  `tech_data_defaults`.
- `sn-assets::code`: `read_assembly`, `game_code` (`TechDefaults`).
- `sn-inspect code [--trees]`, `sn-inspect code --il <Type> <Method>`
  (console only).
- Test-only encoder (`sn-dotnet/src/encode.rs`) that writes small .NET
  assemblies, for unit tests on synthetic bytes.

**Findings** (all in `docs/formats/dotnet.md`): 21,383 method bodies,
570,477 instructions, 0 errors; 793 `TechType` names (793 distinct
values); 7 menus, 159 nodes (fabricator 100), 134 craft nodes, each with a
TechData entry; 17 TechData defaults (item size 1×1, craft amount 1, craft
time 0, max charge −1, …). `RocketScheme` exists but is never used.

**Dead ends / corrections:**
- First real run stopped at a MemberRef whose parent is a TypeSpec
  (generic `List<…>` constructor); TypeSpecs are now named by their
  generic type.
- `TechData.defaultProperties` matched the `default…` field rule but is
  `new List<string>(defaults.Keys)`, not a value; skipped explicitly.
- The scheme reader needed `newarr`/`dup`/`stelem.ref` (C# `params`
  arrays) and `ret` beyond the planned four patterns; still straight-line
  code, so no stop-and-ask.
- `gameplay.md` said `TechType` had 787 members; the decompiled source has
  793 too, so that was a miscount. Corrected.

**Verified (2026-10-10):**
- `cargo test --workspace` passes without the game (sn-dotnet: 9 tests,
  incl. enum, menu and defaults round trips on our own encoded assembly,
  and every truncation and 3 bit flips per byte without a panic).
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  game_code` passes.
- `cargo run -p sn-inspect -- code` prints the numbers above, exit 0.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  -- -D warnings` clean.

## 2026-10-10 — Lint and format fixes for the Anchor Pod commit

**What:** `cargo fmt` on `crates/sn-assets/src/prefab.rs` (the Anchor Pod
rotation block, line breaks only) and the Anchor Pod real-data test loops
over `expected_rot` with `enumerate` (clippy `needless_range_loop`). No
behaviour change.
**Why:** both came in with f3dc283 and made `cargo fmt --check` and
`cargo clippy -D warnings` fail.
**Verified:** `cargo fmt --all -- --check` and `cargo clippy --workspace
--all-targets -- -D warnings` clean; the Anchor Pod real-data test passes
(`cargo test -p sn-assets --test real_data -- --ignored anchor`).

## 2026-10-09 — P0: gameplay data, headless

**What:** first step of Phase E (`docs/DESIGN.md` § 4.3). New readers:
- `sn-unity::gameplay`: `EntTechData`, `PDAData` (log, databank, scanner,
  `defaultTech`, `analysisTech`, `compoundTech`), `Player` (up to
  `guiHand`, skipping its `GUIStyle`), `Oxygen`, `LiveMixin`,
  `LiveMixinData`, `PlayerMotor` / `UnderwaterMotor`, `PlayerController`,
  `BreakableResource`. `sn-unity::collider`: box, sphere, capsule and mesh
  colliders (for M9a).
- `sn-assets::gameplay`: `tech_data` (`Balance/TechData` JSON, keys the
  game reads, absent keys kept as `None` until P1 reads the code's
  defaults), `ent_tech_data`, `player_data` (the main scene's player
  components and the `PDAData` the player points to),
  `Assets::node_components`.
- `sn-inspect techdata`, `sn-inspect player`, `sn-inspect prefab
  --colliders` (also counts `Pickupable` / `BreakableResource` on placed
  prefabs and on the loot table's prefabs).

**Findings** (all in `docs/formats/gameplay.md`):
- TechData: `{"entries": [...]}`; 463 entries + 513 ingredients = the 976
  `techType` keys of the file search; 0 errors, duplicates or unknown
  keys; 28 ingredient rows (12 tech types) have no entry of their own
  (hypothesis: all-default entries trimmed by the game's editor).
- `PDAData` is not a resource: the main scene's `Player.pdaData` points to
  it. The player and its components are part of the main scene.
- Player: oxygen capacity 45, suffocation 8 s / recovery 4 s, health 100,
  swim speed 7.6 (the scene overrides the code's 6.64), walk 3.5.
- Every collider (24,248 on placed prefabs) and every `BreakableResource`
  (4 prefabs) parses to its last byte.

**Dead end:** my field-listing script took `PlayerController.forwardReference`
(a property) for a field; the exact-end check caught it (4 bytes short).

**Verified (2026-10-09):**
- `cargo test --workspace` (no `SUBNAUTICA_DIR`): all pass, including new
  unit tests on synthetic bytes (`ent_tech_data`, `pda_data`,
  `player_fields`, `oxygen_and_health`, `motors_and_controller`,
  `breakable_resource`, `reads_every_shape`,
  `wrong_class_or_size_is_an_error`, `reads_entries_and_reports_odd_ones`,
  `bad_values_are_errors`); every parser test also checks that each prefix
  is an error and byte changes don't panic.
- `SUBNAUTICA_DIR=… cargo test -p sn-assets --test real_data -- --ignored
  tech_data_and_ent_tech_data player_and_pda_data`: 2 pass.
- `sn-inspect techdata` (0.5 s), `player` (0.8 s), `prefab --colliders`
  (11 s): numbers above.
- `cargo clippy` on the changed crates: clean except a
  `needless_range_loop` in `crates/sn-assets/tests/real_data.rs:953`,
  which comes from the previous commit (Anchor Pod test), not from P0;
  left as it is.
- **Not tested:** any of these numbers in the running game; the GroundMotor
  fields beyond `PlayerMotor`; `Survival`; tech type names (P1).

## 2026-10-09 — Plan: Phase E, the road to a playable game

**What:** `docs/DESIGN.md` § 4.3 (new): the remaining look milestones are
deferred (listed there; their rows unchanged) and Phase E comes first: P0
gameplay data, P1 our own .NET metadata/IL reader (`sn-dotnet`), M9a–M9c
(collision, player, oxygen/health), M10 (multiplayer), M9d–M9f (pick-up and
inventory, crafting, save/load, server-authoritative), then a list of later
blocks toward the full game. New `docs/formats/gameplay.md`: where the game
keeps its gameplay data. The M9 row points to § 4.3.
**Why:** the user wants real gameplay and, in the end, Subnautica 1:1. Most
look milestones are first passes waiting on matched screenshots, and nothing
can be played yet. The user chose (2026-10-09) to read code-only data from
their own `Assembly-CSharp.dll` at runtime rather than store it.
**Findings (decompiled with ilspycmd into the system temp folder, nothing
kept in the repository):** recipes are **not** in code. `TechData` loads the
JSON text asset `Balance/TechData` (found in `resources.assets`: 976
`techType` keys, 40 `craftTime`). Code only: the `CraftTree` menus, the
`TechType` enum names, TechData's defaults.
**Dead end:** the first string search (`CraftData`, `CraftTree`) suggested
recipes were in code; decompiling `CraftData` showed they moved to `TechData`.
**Verified:** docs only, no code changed. **Not tested:** everything in the plan.

## 2026-10-09 — M7g4 (part): the door force fields; extra materials as extra passes

**What:** the user reported the Blood Kelp cache (camera −620 −556 1488) and
its "holographic door" as wrong. The door's force field (`x_Forcefield`)
uses two `UWE/Particles/UBER` materials: `FX_ADDFOG FX_DEFORM FX_MULMAP
FX_SCROLL FX_SOFTEDGES WBOIT` with and without `FX_REFRACTMAP`. Both are
decoded from the compiled programs (`docs/formats/materials.md`
§ `UWE/Particles/UBER` meshes) and drawn by `effects_uber.wgsl`:
- soft edges against the scene depth;
- a scrolling deform map that shifts every texture's uv by
  (map − 1) × `_DeformStrength`;
- a refraction map whose offset goes to the WBOIT target B.yz, which the
  composite already applies.

The variant table and checks are in `objects.rs` (`PARTICLES_VARIANTS`);
draws take four textures (`_MainTex`, `_MainTex2`, `_DeformMap`,
`_RefractMap`). Trap found: both materials hold a float and a vector named
`_RefractStrength`; the program reads the float.

The node has one sub-mesh and two materials. Unity draws extra materials
over the last sub-mesh again; we dropped them. They now draw, and each one
is logged. Others found this way: abandoned base corridors, Precursor
columns, a coral plant's opaque pass, the thermal reactor halo, the Gun's
elevator tube and terminal screen. Nearly all are `MarmosetUBER` or decoded
UBER.

`Update::Material` now boxes its description (clippy: large enum variant).

**Verified:**
- `cargo test --workspace`: 157 pass, including new
  `force_fields_follow_the_decoded_terms` and
  `door_force_field_variants_are_drawn_by_the_effect_pass`.
- `cargo test --workspace -- --ignored`: 19 pass. Clippy and fmt are clean.
- Client at the door (`--start -622 -558 1482 --look -608 -561 1483`):
  - no shader warnings; both portal materials are logged as effect
    materials;
  - effects GPU 0.22 ms;
  - the flat green sheet is now scrolling horizontal streaks with
    refraction and a soft second layer (`out/m7g4/door2.png`, `door3.png`).
- **Not compared with the game on screen.**
- The refraction's y sign is not checked.

**Open concerns** (user review 2026-10-09; the values are the game's, so
they stay as they are; listed in `docs/DESIGN.md` "M7g open"):
1. The spotlight cones are dimmer than in the game screenshot.
2. The Precursor pillars are nearly black; this may be the missing
   atmosphere volume ambient (not checked).
3. The other UBER variants keep the stand-in look.
4. The refraction's y sign is unverified.

## 2026-10-09 — M7h: what the prefabs' placeholders spawn (doors, key terminals, crystals)

**What:** the cache's door, key terminal and the ion crystals on the
pedestals were missing. The game spawns them at run time from
`PrefabPlaceholder`s (`PrefabPlaceholdersGroup.Start`), so they never appear
in the world's cells.
- `sn-unity` `placeholder.rs` reads both scripts. The parsers require the
  data to end at the last field, which checks the layout.
- `sn-assets` attaches the placeholders to their nodes.
- The client spawns each active placeholder's prefab (by its class id's
  world entity info) under the placeholder's parent, nested, creatures
  left out. Every skip is logged with its reason. `--no-placeholders` turns
  this off for comparisons.
- `sn-inspect`: `entities --find TEXT` searches every placement (the
  per-batch listing shows only the first cells), and `prefab KEY --props`
  prints a prefab's material properties.

**Verified:**
- Reader unit tests.
- Real-data test `prefab_placeholders`: the pedestal spawns
  `PrecursorIonCrystal`; the cache root spawns `Precursor_Gun_Terminal2Door`
  and `Precursor_PurpleKeyTerminal`.
- Client at the cache: "placeholders spawned so far: 2091 (189 distinct
  prefabs)"; the door stands in its frame (`out/m7g4/door.png`).
- Frame times are within noise: 8.66 ms with vs 8.24 ms without in one pair
  of runs, 16.92 vs 16.95 ms in a pair under load.
- Not compared with the game on screen.

## 2026-10-09 — M7g4 (part): the Precursor consoles' holograms

**What:** the hologram over the cache console was a solid textured green
funnel; in the game it is a faint funnel with a floating symbol. Its six
materials (`x_Precursor_ComputerTerminal_*`) use three `UWE/Particles/UBER`
variants: `FX_ADDFOG FX_SCROLL WBOIT`, plus `FX_MULMAP` and/or
`FX_FRESNELCLIP`. They are decoded from the compiled programs
(`docs/formats/materials.md`). The colour is 2 × `_Color` × vertex colour ×
the scrolling textures × the day/night strength (by
`_UweLocalLightScalar`). Alpha is the colour's mean brightness × its alpha,
so dark texels are transparent; an optional Fresnel clip applies. Fog and
WBOIT output work as for the glows.
- New `effects_uber.wgsl`, drawn in the same accumulation pass as the
  glows.
- The client draws a UBER material this way only if its keyword set was
  decoded and its blend, depth test and cull match what the pass does.
  Every other material is logged with the reason and keeps the stand-in
  look.

**Dead end (cones):** the cache's spotlight cones looked missing. Six
`Precursor_Cache_Spotlight_Generic_Bright` cones are placed, and debug
renders showed all of them, pointing down from the pillars. Each term of
the glow formula behaves correctly in isolation. They are faint by their
values: `_Color.a` × intensity ≈ 0.08. Measured against the user's game
screenshot, our cones add about +11 to the green channel and the game's
about +28. Our cave is also brighter and bluer than the game's (2, 24, 14),
probably the cache's atmosphere volume ambient, which is not ported. The
rest of the gap is **unexplained**.

**Verified:** client tests of the decoded formula and the variant checks.
The console shows the floating "G" symbol (`out/m7g4/cache-placeholders.png`).
Not compared side by side with the game.

## 2026-10-09 — M7g3 fix: the glows take their light's colour (`VFXVolumetricLight`)

**What:** the first M7g3 build drew the cones as flat grey solids (user
report; `out/m7g3/cone.png`). The shader decode was right (re-checked
against the compiled program term by term); its inputs were not. The
game's `VFXVolumetricLight` script (next to each glow's `Light`) overrides
the material on `Awake`: `_Color` = the light's colour with alpha × light
intensity / 8, plus its own intensity, start offset/falloff, soft edges and
near clip; each `LateUpdate` the glow's renderer is on as the light is. We
used the bare material (white, alpha 1, intensity 0.5).
- `sn-unity` `VfxVolumetricLight` (parser; field layout confirmed on a scene
  copy, 176 bytes), `sn-assets` `VolumetricGlow` on the glow's node (script,
  resolved light, whether `Awake` sets the block; the renderer's enabled
  flag follows the light), the client's `glow_material` (a material per
  distinct set of values).
- Docs: `docs/formats/materials.md` § Fake volumetric lights.

**Verified:** unit test `reads_a_volumetric_light`; real-data test
`volumetric_light_glow` (the ion crystal pedestal: green light (0.42, 1,
0.69) at intensity 3, script intensity 0.35, so `_Color.a` 0.375). Client
at the cone (`--start -1110 -680 -600 --look -1110 -680 -585`): 10
distinct glow values logged (green/teal colours, alpha 0.125–0.75,
intensity 0.175–0.5); the cone is a translucent green beam fading along its
length (`out/m7g3/cone2.png`); at the pedestal the white sphere and cones
are now a faint green haze (`out/m7g3/pedestal3.png`), effects GPU 0.28 ms.
**Not compared with the game on screen.** Not followed: later changes of a
light (day/night or scripts) while playing.

## 2026-10-09 — M7g3: the fake light glows with the game's shader and WBOIT

**What:** `UWE/Particles/WBOIT-FakeVolumetricLight` (the white spheres and
cones: ion crystal pedestals, Precursor spotlights) drawn the way the game
draws it (`docs/DESIGN.md` § 4.2, decided with the user: our own pass after
the fog). Decoded from the compiled D3D11 programs
(`docs/formats/materials.md` § Fake volumetric lights).
- `apps/sn-client/src/effects.rs` (new): the worker sends these parts'
  meshes with vertex colours; the main world spawns `EffectPart`s; the
  render world uploads the meshes once, accumulates the glows into two
  half-float WBOIT targets with the game's blend and weights, then
  composites like `Hidden/WBOIT Composite`, after the sun shafts and
  before tonemapping. `effects.wgsl` (soft edges from the scene's depth,
  Fresnel, near fade, vertex-alpha falloff, water fog at the glow's own
  distance and the fade over its fogged path); `effects_composite.wgsl`.
- `water_common.wgsl`: `water_fog_length` (the in-water fog path, same
  rules as `apply_water_fog`). `sun_shafts.rs`: a `SunShaftsPass` set to
  order after. `objects.rs`: effect materials and meshes, a log line per
  effect material with its values. No new dependencies.
- Decode corrected on re-reading: the fog fade uses the path through
  fogged water past the start distance, not the whole distance; the
  composite's alpha is `(1 − A.a)² + A.a × scene.a`.

**Verified (2026-10-09):** workspace tests 148 pass (4 new formula tests:
the glow's terms, the fog fade's 112.5–125 m, one glow composites as plain
alpha blending, overlapping glows averaged by weight; they check the
formulas in Rust, not the WGSL itself), real-data 17 pass, clippy and fmt
clean. Client at a pedestal (`--start -1121 -686 -712 --look -1125 -688
-705 --benchmark 120 --gpu-timings`): `effects: 21 parts, 3 meshes`, the
effects pass 0.21 ms of GPU time, mean frame 6.99 ms, 0 warnings, and the
glow shader is no longer in the unported list. Screenshots
`out/m7g3/pedestal-final.png` (a soft glow over the pedestal) and
`out/m7g3/cone.png` (a spotlight cone, evenly lit near its apex with hard
sides). Looked into with a debug render of the Fresnel term (≈ 0.9 across
the cone) and logged mesh data: the cone's apex normal is its axis and
its side normals are for a narrower cone, so that look follows from the
game's data under the decoded formula. **Not compared with the game.**

## 2026-10-09 — M7g2: shader names and render state; unported shaders logged

**What:** every material's shader is known by name (`docs/DESIGN.md`
§ 4.2). Nothing is hidden: shaders we haven't ported stay drawn as before
and are logged.
- `sn_unity::Shader` (class 48): the parsed form up to the name:
  properties (with defaults), sub-shaders, passes and their state (blend
  per render target, colour mask, z write/test, cull, offsets, tags); the
  programs' bindings walked over. Layout written from UnityPy's 2019.4 type
  database (`docs/formats/unity.md` § Shaders); 2 tests on synthetic bytes.
- Client: each material's shader name (one parse per shader object); a
  shader other than MarmosetUBER is logged the first time a part with it is
  drawn, and the totals per shader each time loading settles. The UBER
  decision keeps the property fingerprint: it picks exactly the UBER
  shaders (checked on all drawn materials).
- `sn-inspect prefab --materials` uses `Shader` (the string scan is gone),
  tells materials apart by name **and** shader (names repeat across bundles:
  1,975 materials, not 1,973; `Standard` has 4, not 2), prints every shader
  used with its first pass; `unity --all` parses every shader of the game.
- Real-data test `shaders_of_the_fake_volumetric_light`. No new dependencies.

**Verified (2026-10-09):** `unity --all`: 5,472 files, 363 shaders read, 0
unreadable. The 17 shaders of the drawn materials: name, sub-shader and pass
counts, first-pass blend, colour mask, z write, z test, cull identical to
UnityPy (17 of 17; `out/m7g2/oracle.py`). The fake volumetric light is
additive (`Blend One One`), no depth write, no culling. MarmosetUBER's
texture defaults (`_MainTex`, `_SpecTex`, `_Illum` white, `_BumpMap` bump)
read with UnityPy: our defaults match (was a hypothesis). Workspace tests
143 pass, real-data 17 pass, clippy and fmt clean. Client at the lifepod
(`--benchmark 300`): 9 unported shaders logged, totals e.g.
`UWE/Particles/UBER` 91, `WBOIT-FakeVolumetricLight` 35, `UWE/SIG` 33,
`Standard` 27, `SIG Triplanar with Capping` 15 drawn parts; 0 warnings;
mean 11.55 ms (before M7g2: 11.60 ms). At the ion crystal pedestal: 8
shaders logged; the screenshot (`out/m7g2-pedestal.png`) shows a wall in
front of the camera, so no picture of the pedestal yet. **Not compared with
the game.**

## 2026-10-09 — M7g1: the main camera's culling mask (occluder shells gone)

**What:** the client no longer draws renderers on layers the game's main
camera leaves out (`docs/DESIGN.md` § 4.2). The game's rule, no name
matching.
- `sn-unity::Camera` (class 20, up to the culling mask; `draws_layer`),
  `TAG_MAIN_CAMERA`; 2 tests on synthetic bytes.
- `sn-assets::Assets::main_camera` (the `main` scene's enabled camera on an
  active GameObject tagged `MainCamera`, like `Camera.main`); real-data test
  `main_camera_skips_the_occluder_layer`.
- Client: the worker reads the mask at start (logged with the layers it
  leaves out; if it fails, a warning and every layer drawn) and skips such
  nodes in `prefab_parts`, logging each one it skips.
- `sn-inspect scene` uses `Camera`; `prefab --materials` also prints the
  first placement of each material. Plan changed at the user's request:
  shaders not yet ported **stay drawn** and get logged (M7g2), not hidden.
  No new dependencies.

**Verified (2026-10-09):** workspace tests 141 pass; real-data tests 16
pass; clippy `-D warnings`, fmt clean. Client `--start -59 -1192 85 --look
-59 -1199 95 --benchmark 120` (the Lava Castle base): "main camera: culling
mask 0x65ffff17, layers not drawn [3, 5, 6, 7, 25, 27, 28, 31]"; 28 nodes
skipped in the area that loaded, all `Occluder_*_shell` on layer 27
(including the prison aquarium's in the Aurora scene); 0 warnings; 120
frames mean 10.64 ms. The screenshot (`out/m7g1-lavabase.png`) is nearly
black: no sunlight at that depth, and the base's own lights are not drawn
as the game does. **Not compared with the game** on screen.

## 2026-10-09 — Research: occluder shells, white spheres, untextured objects (plan M7g)

**What:** checked an earlier research note (by another model, not in the
repo) on the "invisible walls" in alien bases and the white spheres, and
wrote the plan (`docs/DESIGN.md` § 4.2, M7g1–M7g6). Nothing in the client
changed.
- `sn-inspect prefab --materials`: every material on a drawn node of the
  placed prefabs and the startup scenes, with uses, layers, texture slots,
  render queue, keywords and its shader's name (a heuristic string scan,
  inspection only).
- `sn-inspect scene <name>` also prints each `Camera`'s near/far and
  culling mask (Unity 2019.4 layout, hand-written).
- New `docs/formats/materials.md` (census, per-shader findings);
  `docs/formats/unity.md` § Cameras and layers. No new dependencies.

**Verified (2026-10-09):** `sn-inspect prefab --materials` (65 s): 1,369
prefabs, 0 unreadable, 1,973 materials. `DepthOnly` → `Unlit/DepthOnly`, 39
nodes in 38 prefabs, all on layer 27; `x_AtmoLight_*` →
`UWE/Particles/WBOIT-FakeVolumetricLight`, queue 3101, 68 nodes.
`sn-inspect scene main`: `MainCamera` mask `0x65ffff17` (layer 27 off),
`MainCamera (UI)` `0x20`, `ImguiCamera` `0x80000000`, which matches the
layer names (UI = 5, DebugOverlays = 31). Earlier note **confirmed:** the
occluder shells and the sphere/cone lights are drawn as white meshes by us
and not by the game. **Corrected:** the lights are blended, not opaque
(alpha 1, so they look the same); the door force field is not drawn at all
(its prefab is not placed in the world data). **Missed by it:**
`UWE/Particles/UBER` meshes (1,058 placements) and the triplanar rocks
(976 placements, drawn white). Not checked on screen.

**Housekeeping noticed, not changed:** the repo root holds gitignored
leftovers of earlier sessions (`Voxeland.cs`, `VoxelandChunk.cs` look like
decompiled game code, plus `scratch_*.py`, `shaders.txt`, `test_colors.py`).
They are not tracked, but AGENTS.md wants such output in `out/`.

## 2026-10-09 — M7f3: Lifepod 5

**What:** the `escapepod` scene is placed as a new game does
(`docs/formats/unity.md` § Scenes, Lifepod placement).
- `sn-world::StartMap` (pure: `IsStartPointValid`, `GetRandomStartPoint`
  with our seeded draw, wrap/clamp at the edge; 3 unit tests).
- `sn-unity`: `parse_random_start`, `EscapePod`, `PrefabSpawner` (both
  spawner scripts), `SpawnType` constants; synthetic tests.
- `sn-assets`: `Assets::start_map` (the texture, rows flipped to
  `GetPixel` order), `Scene::place_escape_pod` (returns the player spawn),
  `Scene::follow_targets` (`MoveAndRotateWithTransform` once),
  `Scene::spawns` + `SceneSpawn::placement` (`PrefabSpawnBase.SpawnObj`'s
  rules), `Prefab::set_local`/`world`; unit tests for placement and the
  inverse transform.
- `sn-inspect scene --lifepod [--seed N]`.
- Client: the start point and player spawn are computed before the app
  starts (`--lifepod-seed N`, default 1; `--lifepod X Z`); the worker
  places the pod with the same point and spawns its modules; **the camera
  now starts at the player spawn** unless `--start` is given (before: 0
  −10 0; benchmarks without `--start` are not comparable with earlier
  ones). No new dependencies.

**Verified (2026-10-09):**
1. `sn-inspect scene --lifepod`: start map 0.59 % valid; seed 1 → (−127.73,
   0, −49.75) after 72 draws, seed 2 → (24.92, 0, −133.47); player spawn
   (−126.79, 2.1, −49.75); 6 objects on their targets; 7 spawners fire
   (UI, medical cabinet, 3 power cells, fabricator, radio), their parents
   inside the pod. Two client runs give the same point (seed 1).
2. Real-data test `lifepod_start_and_modules`; all 12 real-data tests of
   sn-assets/sn-unity/sn-world, workspace tests, clippy, fmt: pass.
3. Client, default start: "scene escapepod: 1 top-level objects, 248
   nodes, 25 drawn, Lifepod 5 at …, spawned [6 modules]", 1,242 scene
   entities, 0 warnings, 300 frames mean 11.10 ms. Screenshots
   `out/m7f3-outside.png` (the pod on its float ring at the water line,
   "5 LIFEPOD") and `out/m7f3-inside.png` (seat, wall panel, a module).
   **Not compared** with the game. Known gaps: the camera stands at the
   spawn transform (the game's eye is higher), the interior is lit by the
   outside sky (the pod's own Marmoset sky and lights not used), no
   floating motion, no intro damage effects.

**Differences from the game:** every known gap left by M7f1–M7f3 (Aurora
LODs, explosion timing, culling, effects, skinned animation and blend
shapes, the lifepod's floating, eye height, interior sky and lights, intro
state, module scripts) is listed in the new `docs/DESIGN.md` row **M7f4**;
README "What doesn't work yet" updated to match.

**Also:** commit `16c6542` repaired characters a PowerShell round trip had
re-encoded in `main.rs` and the skinned census (in `9256e34`); edits are
now made with Python or the editor only.

## 2026-10-09 — M7f2: skinned meshes

**What:** skinned meshes are drawn in the pose their hierarchy stores.
- `sn-unity`: `SkinnedMeshRenderer` (renderer fields, mesh, bones, blend
  shape weights, root bone, bounds; `MeshRenderer` shares the renderer
  part); `Mesh::bind_poses`; `MeshGeometry::bone_weights`/`bone_indices`
  from vertex channels 12/13. Synthetic tests for both, cut-short input
  gives errors.
- `sn-assets::skin` (pure: `Σ w · bone · bindPose · v`, normals and
  tangents too, missing bones left out; 4 unit tests on synthetic bones);
  `PrefabNode::bones` (node indices), `Prefab::skinned_geometry`,
  `Prefab::visible` (with indices). Skinned renderers now give the node
  its mesh and materials, so they are drawn (before: skipped, the next
  LOD level shown instead).
- Client: skinned parts are skinned once per prefab node in the worker
  and placed in the prefab root's space; bone-less ones as plain meshes.
- `sn-inspect prefab --skinned` (checks, LOD 0 vs LOD 1 bounds, nearest
  placement); `prefab <KEY>` exports skinned nodes skinned.
  No new dependencies.

**Dead end:** first read the bind poses column by column (assuming
Unity's `m00 m10 …` field order). Sizes still matched LOD 1, as sizes
ignore translation; the bottom-row check (269 of 269 wrong) showed the
file stores them row by row. Fixed before anything used them; the
census now also compares centres.

**Verified (2026-10-09):**
1. `sn-inspect prefab --skinned`: 73 placed prefabs with active skinned
   meshes, 304 renderers drawn: 190 skinned, 113 bone-less (drawn plain),
   1 without bone indices; 0 bones outside their hierarchy; 0 bind poses
   without a 0 0 0 1 bottom row; LOD 0 vs static LOD 1 on the 3 prefabs
   that have both: sizes within 0.1 %, centres within 1 cm. 0 errors.
   Escape pod scene: 31 skinned renderers, all with bones.
2. Real-data test `skinned_lod_matches_its_static_lod`
   (`AbandonedBaseFloatingIsland1`, 1 % tolerance); all 11 real-data
   tests of sn-assets/sn-unity, workspace tests, clippy, fmt: pass.
3. Client at the lifepod start: 16,440 entities, 0 warnings, 300 frames
   mean 12.13 ms (12.27 ms in the M7e2 follow-up run, not A/B in one
   session). Close-up `out/m7f2-braincoral.png` (start 44 −18 96): the
   brain coral (LOD 0 now, bone-less) at its place. **Not compared** with
   the game; no animation (the game's `Animator`s pose these).

## 2026-10-09 — M7f1: scenes, the Aurora

**What:** plan for M7f in `docs/DESIGN.md` (three steps: scenes and the
Aurora, skinned meshes, Lifepod 5). This step: a scene reader and the
scenes the game spawns at start, drawn in the client.
- `sn-unity::scene_scripts`: `MainGameController.additionalScenes`,
  `LightmappedPrefabs.autoloadScenes`, `CrashedShipExploder` (synthetic
  tests, cut-short input gives errors).
- `sn-assets::Scene` (`Assets::scene`, `startup_scenes`, `behaviours`,
  `spawn_lightmapped_prefab`, `swap_aurora_models`); the prefab hierarchy
  reader is shared (`Assets::hierarchy`); `PrefabNode` now has its
  GameObject key and own active flag, `Prefab::set_active` as Unity's
  `SetActive`; `Assets::script_class` public.
- `sn-inspect scene <name> [--tree D | --script Class]`, `scene --startup`.
- Client: the worker loads the startup scenes after the asset index and
  sends each top-level object as an instance, shown always (not streamed);
  the escape pod scene is skipped until M7f3. Spawning one instance is now
  `ObjectStreamer::spawn_instance`, shared by batches and scenes.
  `--no-scenes`, `--aurora intact|exploded` (default intact: a new game).
  No new dependencies.

**Found** (`docs/formats/unity.md` § Scenes): `main` → `Essentials` →
`Cyclops` (template), `EscapePod` and `Aurora` (spawned at the origin).
The Aurora scene also holds four non-streaming world parts (Precursor
prison exterior and aquarium, Lost River base, Lost River large trees).

**Verified (2026-10-09):**
1. `sn-inspect scene --startup`: the chain above; aurora 6 top-level
   objects, intact 330 nodes drawn, exploded 337, 2 objects off / 2 on.
2. Real-data test `startup_scenes_and_aurora` (those numbers, class
   counts 3,189/437/291, escape pod 248 GameObjects, 35 skinned
   renderers); all real-data tests of sn-assets and sn-unity, workspace
   tests, clippy `-D warnings`, fmt: pass.
3. Client at (250 120 650) looking at the Aurora: log "scene aurora: 6
   top-level objects, 3189 nodes, 330 drawn, Aurora intact", 1,196 scene
   entities (1,274 exploded), 0 warnings. 300 frames: 4.69 / 4.75 ms with
   the scenes, 4.47 ms with `--no-scenes`, 5.96 ms exploded (one run each).
   Screenshots `out/m7f1-aurora-far.png`, `out/m7f1-aurora-exploded.png`:
   the ship sits in the water at the right place (by eye). **Not compared**
   with the game; always the most detailed LOD (the game switches to LOD
   1/2 with distance); no fire, smoke, radiation effects.

## 2026-10-09 — M7e2 follow-up: Noisey Wave's sway, UBER grass sky

**What:** after the user's review of M7e (closing two of its open
points; the rest saved as M7e3 and M4b in `docs/DESIGN.md`):
1. Noisey Wave's sway (type 243): read its vertex program again and the
   game's chunk placement (`WorldStreaming/MeshBuilder`: grass vertices
   are in the chunk's space, origin `cellId × 16` at level 0). `sn_terrain::
   build_grass` now keeps each vertex's position in its chunk
   (`chunk_local`); the client puts its part along the material's
   `_ObjectUp` in the second uv set; `grass.wgsl` adds the sway along
   `_WorldWaveDir` with the program's 2D simplex noise.
2. MarmosetUBER grass (4 types) got **no** sky before (Marmoset defaults:
   no ambient), not the global one as written in the M7e2 entry. Checked
   the game: the pooled grass piece (`TerrainPoolManager.chunkGrassPrefab`)
   has no `SkyApplier`, so the global sky is right. `objects.rs`
   `apply_sky` (pulled out of `object_material`, unchanged);
   `TerrainLook::set_global_sky` is called when the objects' worker has
   read the skies and updates the grass materials made and still to make.

**Verified (2026-10-09):**
1. Unit tests: chunk-local positions (corner a multiple of 16, bases at
   the ground); the noise within ±1, smooth and varied; the sway at most
   `_WaveAmount` × height, along the wave direction only; the shader's
   constants; Noisey Wave material values. Workspace tests, clippy, fmt:
   pass.
2. Client at the lifepod: log "4 MarmosetUBER grass materials lit with
   the global sky (found)", 0 warnings or errors, 300 frames mean
   12.27 ms (11.72 ms in the M7e2 entry's run: not compared A/B in
   the same session).
3. Type 243 found with a throwaway probe of the octrees: 4 top voxels in
   batches y 14–19, all in 9-18-12; `sn-inspect grass 9 18 12`: 8 tufts.
   Close-up run there: the shader compiles, no errors. **Not checked by
   eye**: the red grass is hidden in dense seaweed that also sways.

**Dead end:** a first noise test divided by 130 twice and failed; the
noise itself was right (checked by printing values).

## 2026-10-09 — M7e2: the game's grass shaders

**What:** decoded the deferred programs of `UWE/SIG Terrain Grass`,
`UWE/SIG` and `UWE/SIG AlphaCutout + Noisey Wave`
(`docs/formats/terrain-materials.md` § Grass shaders) and ported them:
`grass_look.rs` + `grass.wgsl` (vertex waving from the vertex colours,
world-space mask, top/bottom tints with the height gradient, cutoff, SIG
specular/gloss/glow, the game's light pass). `sn-assets::GrassLook` now
knows its shader (`GrassShader`, by properties), the SIG, mask, specular
and glow maps and the parsed material. MarmosetUBER grass (4 types) is
drawn with our object shader: `objects.rs` `material_desc` (pulled out of
the worker's material loading, unchanged) and `object_material` shared.
Not ported: Noisey Wave's sway (type 243; ≤ 8 cm, needs the game's chunk
origin), biome skies for UBER grass (global sky).

**Verified (2026-10-09):**
1. Real-data test `terrain_materials_resolve_and_decode`: shader per
   grass type as UnityPy names them (48 Terrain Grass, 6 + 1 `UWE/SIG`,
   1 Noisey Wave, 4 MarmosetUBER); textures 287 = 211 terrain + 76 grass.
   All 11 real-data tests of the touched crates, unit tests (3 new: wave
   formula and shader constants, material values), clippy, fmt: pass.
2. The objects are unchanged by the refactor: lifepod census before and
   after equal (1,531/1,567 UBER materials, 16,216 entities, 1,619
   materials, 0 warnings).
3. Benchmarks with / without grass: lifepod 11.72 / 11.48 ms; Grassy
   Plateaus 9.25 / 8.97 ms; lifepod at midnight 11.84 / 11.44 ms. Log:
   60 grass materials, 56 with the grass shader, 4 with the object shader.
4. Waving: two runs of the same view at different times differ in 9.5 %
   of the seaweed region's pixels with grass, 2.9 % without (sand: 13.1 /
   14.4 %, the caustics). Seaweed region mean RGB 87.7/85.5/83.1 (first
   pass) → 85.4/85.9/84.3 (darker bases). Screenshots
   `out/m7e2-start.png`, `out/m7e2-plateaus.png`, `out/m7e2-night.png`.
   **Not compared** with the game.

**Dead ends:** the keyword lists our disassembly script prints next to
each program can belong to a neighbour (as noted in M8c7); the `UWE/SIG`
variant with the SIG map was found by its constant table instead. A
first script edit of `objects.rs` stopped half-way (an anchor changed by
rustfmt) and was finished by hand; the census above checks the result.

## 2026-10-09 — M7e1: terrain grass

**What:** the 60 block types with grass now grow it, placed by the game's
rules (`docs/formats/terrain-materials.md` § Grass): 4 spots per face,
tilt range, density or Perlin noise, jitter, spin, Z-up turn, scale,
the per-chunk budget of 10,000 vertices/triangles with the raised
reduction, vertex colour/height. New: `sn-unity::GrassSettings` (all of
`VoxelandBlockType`'s grass fields), `sn-assets::TerrainGrass` /
`GrassLook` (mesh geometry, textures, material values) and
`TerrainMaterials::grass_types`, `sn-terrain::build_grass` (pure, 7 unit
tests), `sn-inspect grass X Y Z` and a grass census in `terrain-materials`.
The client builds one merged mesh per (batch, type) in the terrain worker
at level of detail 0 and draws it with a first-pass object material, no
shadow casting; `--no-grass`. Dependency: `sn-assets` now depends on
`sn-terrain` (ours; for `grass_types`).

**Verified (2026-10-09):**
1. `sn-inspect grass 12 18 12` twice: 7,826 tufts, 608,726 vertices,
   752,923 triangles, same hash `73e29a516d24d7e8`.
2. Real-data test `terrain_materials_resolve_and_decode` (updated): 60
   grass types; textures 261 = 211 terrain + 50 grass-only; mesh sizes of
   `coral_reef_grass_03` (18/18) and `Coral_reef_small_deco_07` (125/516)
   equal UnityPy's. All 11 real-data tests of the touched crates, unit
   tests, clippy, fmt: pass.
3. Benchmarks (two runs with grass, one without): lifepod 34,469 tufts,
   2,891,017 triangles in 8 batches; 11.62 / 11.68 ms vs 11.46 ms
   without. Grassy Plateaus (−700 −94 −300): 41,007 tufts, 1,015,413
   triangles; 9.09 / 9.04 ms vs 8.80 ms. Counts equal on both runs.
   Screenshots `out/m7e-start.png` (green coral grass on the plateau tops
   and the sand), `out/m7e-plateaus.png` vs `m7e-plateaus-nograss.png`
   (the red seaweed fields). **Not compared** with the game; the grass
   stops ~100 m out (the game also draws its level 1 with reduction 0.5).

**Found:** four grass types (52, 76, 83, 251) use coral materials with
MarmosetUBER keywords (`_Cutoff` 0), not the grass shader; noted for M7e2.

## 2026-10-09 — M7d: spawn slots filled

**What:** the cells' 90,289 `EntitySlotsPlaceholder`s are filled the way
the game fills them when a cell first loads (`docs/formats/entities.md`
§ Spawn slots). New: `sn-world::slots` (slot parse, the game's choice
rule, copies within 4 m, Z-up turn, our seeded SplitMix64 keyed by
placeholder id and slot index), `sn-unity::json` (the catalog's JSON
reader moved out of `addressables.rs`, now accepting `//` and `/* */`
comments), `sn-unity::parse_world_entity_data`, `sn-assets::loot_table` /
`entity_infos`, `sn-inspect slots [X Y Z] [--seed N]`. The client spawns
the fillers with the cells at their own cell level (creatures skipped, as
for placed objects); `--slot-seed N` (default 1), `--no-slots`. No new
dependencies.

**Finding:** the roadmap's hypothesis was wrong: slots hold **no
vegetation**. Seed 1: 141,639 objects, of which 102,777 creatures (not
drawn), 35,808 resource outcrops (limestone, quartz, …), 1,186 eggs, 1,029
fragments. The missing small plants must come from somewhere else (M7e
grass is the next candidate).

**Verified (2026-10-09):**
1. `sn-inspect slots`: 90,289 placeholders, 1,288,139 slots, 0 errors;
   190 distribution entries / 1,295 rows / 352 biomes, 3,336 entity infos,
   every pickable prefab has an info and a path; all but 7 slots inside
   their batch; counts per biome logged (e.g. Safe Shallows sand flat
   4,766 slots, 559 filled, 830 objects). Hash of all spawns
   `0d4ea805ed9120b2` on two runs with seed 1, `8edc2ef9122ad69a` with
   seed 2.
2. Real-data test `spawn_slot_tables_and_fill` (lifepod batch: 30
   placeholders, 1,719 slots, 325 objects at seed 1, 334 at seed 2; same
   seed equal) and unit tests (slot parse and corruption, choice rule,
   copies/rotation, determinism, JSON comments, WorldEntityData) pass;
   all real-data tests of sn-unity/sn-assets/sn-install pass (catalog with
   the moved JSON reader). Clippy, fmt: clean.
3. Lifepod benchmark (300 frames, two runs with slots, one without): 328
   slot objects shown (478 more entities: 16,216 vs 15,738), identical on
   both runs, 0 warnings; mean 11.10 / 11.20 ms vs 11.02 ms without.
   Screenshots `out/m7d-slots.png` / `m7d-noslots.png` (wreck debris
   appears on the sand), `out/m7d-outcrops-slots.png` /
   `m7d-outcrops-noslots.png` at (−76 −7 −100): a limestone outcrop on
   the slope, seated on the terrain. **Not compared** with the game.

**Dead ends:** UnityPy can't read `WorldEntityData` by type tree (no
script types); its layout was read from the raw bytes (68 bytes per info).

## 2026-10-09 — M8e3: the objects' lights light the world

**What:** placed objects' realtime lights spawned with them
(`objects.rs` `spawn_light`; Bevy lights only for culling and
clustering) and applied by `game_local_lights` in `game_light.wgsl` with
the game's point/spot formula (falloff curve **hypothesis**, see
`docs/formats/lighting.md` § How we render local lights); objects'
directional lights (≤ 8) through the light parameters; `--no-local-lights`
for comparisons. `sn-inspect prefab --lights` now also lists the densest
50 m columns of lights (all, and glowing coral only) with the prefab
holding most of them.

**Verified (2026-10-09):** unit tests `light_falloff_at_known_distances`
(range 10 m: 1 at 0 m, 0.5 at 2 m, 1/17 at 8 m, 0 at 10 m; continuous,
never rising) and `shader_uses_the_same_falloff` (the shader holds the
same constants). Night benchmarks, lights on / off: lifepod 458 point + 4
directional lights, opaque pass 1.96 / 2.00 ms, image mean RGB 8.4 13.6
16.7 / 8.2 13.1 16.2 (`out/m8e3-night-*.png`); Grand Reef glowing coral
(−1325 −500 −370) 1,744 point + 7 spot lights, opaque 1.11 / 0.86 ms,
mean 0.0 4.4 7.0 / 0.0 4.0 6.1 (`out/m8e3-reef-*.png`: the rock around
the coral lit). **Not compared** with matched game screenshots.

**Dead ends:** the densest light columns of the whole world are all
Precursor interiors (gun, prison, lava castle); a first "dense area"
render at (425 −55 1095) put the camera inside the mountain (black
image). The glowing-coral-only list was added for that.

## 2026-10-08 — M8e1: the game's lights read; sun intensity units fixed

**What:** `sn-unity::Light` (Unity 2019.4 layout, 264 bytes) and
`DayNightLight`; `PrefabNode::lights` / `day_night_light`;
`sn-inspect prefab --lights` (census of every placed prefab). Decoded the
point and spot light programs (`docs/formats/lighting.md` § Point and spot
light passes). Found `GraphicsSettings.m_LightsUseLinearIntensity` off:
lights are `linear(colour × intensity)` in Unity's passes; our sun in the
surface pass was `linear(colour) × intensity` → new `SkyState::sun_light`
(fog, shafts and water keep the script value, as the game).

**Verified (2026-10-08):** synthetic tests (Light, truncation); real-data
test `prefab_lights` (a glowing kelp prefab: 4 point lights, intensity 1.5,
range 15, colour as UnityPy; the safe shallows "Bounce" light and its day
curves). Census: 226 of 1,369 placed prefabs carry lights; ~9,970 lights
in the world's placements (9,186 point, 561 spot, 123 directional, 99 with
shadows). Sun in the light pass 0.886 → 0.780 (red) at 09:36; sunlit sand
3.5 % darker, shadowed sand unchanged (`out/m8e-sunfix.png`). Tests pass.

**Dead ends:** a first benchmark after the change saved no new screenshot
(an earlier client was still running and held the exe); the comparison
used the old image and showed "no change" — redone.

## 2026-10-08 — M8c7 (first pass): object specular, glow and biome skies

**What:** decoded MarmosetUBER's deferred pass and the Marmoset sky
classes (`docs/formats/lighting.md` § Objects). New: `sn-unity::marmo`
(`MarmoSky`, `SkyApplier`, `MarmoSkiesPrefabs`, unit tests on synthetic
bytes), `sn-assets::marmo_skies` (biome skies + global sky),
`PrefabNode::sky_applier`. The client's object shader now ports the UBER
G-buffer: specular from `_SpecTex`/`_SpecColor`/`_SpecInt`/`_Fresnel`, gloss
from `_Shininess`, glow from `_Illum` with day/night strengths
(`_UweLocalLightScalar`, new texel 11 of the light parameters), the sky's
camera exposure, SH ambient and unlit flag; no Unity ambient on UBER
objects (it was added before; the shader doesn't). Materials are made per
(material, sky); the sky is picked per placed object from its biome.

**Verified (2026-10-08):** real-data test `marmo_skies_and_sky_appliers`:
37 skies for 145 biomes, values equal to the UnityPy readout. Benchmark at
the lifepod: 1,537 UBER materials (1,482 specular maps, 559 glow maps),
materials made for 13 skies; `out/m8c7-start.png` vs `m8c6-start.png`:
terrain sand unchanged (mean RGB 142/179/164 vs 141/177/162), the floating
boulder darker (58/77/68 vs 79/95/88: no Unity ambient), glowing grass
tips greener. Frame time 13.2–16.6 ms over three runs (11.1 ms last
round), but runs vary a lot right now (shafts 1.9 → 2.7 ms GPU with no
change to them); **not A/B tested**. Tests, clippy, fmt: pass. **Not
compared** with matched game screenshots.

**Dead ends:** the shader-index keywords from the program's preceding bytes
were wrong (they belong to neighbouring programs); variants were matched by
diffing bytecode instead.

## 2026-10-08 — M8c6 (first pass): sun shadows, shadowed light shafts

**What:** the game's shadow settings read (`QualitySettings` High: 4
cascades over 50 m, soft; `docs/formats/lighting.md` § Sun shadows). The
sun now casts Bevy shadows with these cascade bounds and Gaussian
filtering; the game lighting already used them. Terrain and objects beyond
the nearest level of detail don't cast (`NotShadowCaster`, updated when a
batch changes level). The light shafts now sample the shadow map through
Bevy's view bind group (the pattern of Bevy's volumetric fog), one
comparison per step.

**Verified (2026-10-08):** `out/m8c6-start.png` (the floating boulder's
shadow on the sand), `out/m8c6-shafts.png` (shafts cut into beams).
Benchmark at the lifepod: 11.1 ms mean (21 ms before limiting the
casters; 9.5 ms without shadows). Shafts 1.9 ms GPU. Tests, clippy, fmt:
pass.

**Dead ends:** shadows from every entity cost ~11 ms of CPU (Bevy prepares
shadow draws per entity and cascade); the soft filter in the shafts' 200
samples per pixel cost 4.8 ms.

## 2026-10-08 — M8c5: stars

**What:** decoded uSky's star field (`StarField`, the `StarsData`
catalogue in the game's resources, shader `Hidden/uSky/Stars`;
`docs/formats/sky.md` § Stars) and ported it: `sn-unity::parse_text_asset`,
`sn-assets::resource_bytes`, `stars.rs` (catalogue, unit test),
`stars.wgsl` + a pass in `sky_dome.rs` (after the fog, before the water).

**Verified (2026-10-08):** catalogue 218,640 bytes (9110 × 6 floats;
real-data test); 2460 stars kept; `out/m8c5-stars.png` (midnight: stars,
the planet). Tests, clippy, fmt: pass. **Not compared** with matched game
shots; ours look fainter than the user's (the game's bloom is not ported).

## 2026-10-08 — M8c5 (first pass): light shafts under water

**What:** decoded the game's `WaterSunShaftsOnCamera` (class, the scene's
values, both shader passes; `docs/formats/lighting.md` § Light shafts) and
ported it: `sun_shafts.rs` + `sun_shafts.wgsl` (half-resolution ray march
through the caustics projected along the sun, added after the water
surface). Also: `cast` is reserved in WGSL (again).

**Verified (2026-10-08):** `out/m8c5-shafts-sun.png`: faint vertical
shafts in the middle distance looking towards the sun; GPU 0.37 ms. Tests,
clippy, fmt: pass. **Not compared** with matched game screenshots; weaker
than the user's kelp shot, likely because the game's shafts are cut by the
sun's shadows (not ported yet, M8c6).

## 2026-10-08 — M8c4 (first pass): High quality waves (FFT)

**What:** the user plays with Water quality High (also bloom + lens dirt,
DoF, motion blur, AO, SSR, FXAA, dithering), so the waves are simulated, not
baked. Decoded `WaterDisplacementGenerator` (Phillips spectrum, the spectrum
update compute shader, the displacement packing; `docs/formats/water.md`
§ High quality waves). `sn-unity`: `FftWaves` (the generator's settings).
`sn-client`: `water_fft.rs` (initial spectrum, 2 unit tests), `water_fft.wgsl`
(spectrum update, our own FFT, packing), `--water-quality medium|high`
(default high). Plan for the remaining effects added to DESIGN (M8c5, M8d).

**Verified (2026-10-08):** scene values equal UnityPy's (wind 600 cm/s at
45°, choppy 1.3, amplitude 0.35, min wave 0.01; real-data test). Screenshot
`out/m8c4-high.png` next to the user's open-sea shot: finer, textured waves,
sparse foam. GPU: FFT 0.12 ms, surface 0.42 ms. Tests, clippy, fmt: pass.
**Hypothesis:** the FFT's sign (from the game's twiddle phase) and the
row/column orientation; a mirror would mirror the wind direction.

## 2026-10-08 — M8c3: game units, colour grading, Unity's ambient

**What:** after the user's screenshots (their game: High water quality)
showed our night far too bright:
- The game's post-processing found (`docs/formats/lighting.md` § Units):
  colour grading off by default (clamp, no tonemapping), bloom on. Our
  image now holds the game's values (one light unit = 1.0, was 2.55 from
  Bevy's sun lux and exposure) and `--color-grading off|neutral|aces`
  (default off) picks the tonemapper (neutral/aces: Bevy's nearest, not
  exact).
- Unity's own flat ambient (`CurrentSkyColor`, added by the G-buffer pass)
  now lit: it was missing, so surfaces were too dark.

**Verified (2026-10-08):** mean colours, ours vs the user's screenshots
(not the same places or times): night horizon (15, 21, 28) vs (5, 8, 16)
(was (31, 39, 51)); day sky (95, 142, 198) vs (111, 160, 222); underwater
fog (58, 115, 180) vs (63, 153, 221); sand (58, 113, 123) vs (108, 147, 120).
Tests, clippy, fmt: pass. **Needs matched screenshots** (same place and
time) to go further.

## 2026-10-08 — M8c3 (first pass): the game's lighting, caustics

**What:**
- Compared with the user's game screenshots (day, dusk and night, above and
  below; different places than ours). Sky and underwater fog are close;
  missing were caustics, the game's ambient and sunlight model, stars,
  object emission.
- Decoded the game's deferred lighting shader
  (`Hidden/Internal-DeferredShadingCustom`, directional cookie variant):
  `docs/formats/lighting.md`.
- `sn-unity`: `parse_resource_container` (`ResourceManager`);
  `WaterSurface.caustics_size`. `sn-assets`: `resource_texture`,
  `water_caustics` (64 frames). Real-data test extended.
- `sn-client`: `game_light.rs` + `game_light.wgsl`: the game's lighting
  (caustics, sunlight attenuated under water with the colour cast, top and
  bottom ambient, water glow as ambient, Blinn specular) replaces Bevy's PBR
  lighting in the terrain and object shaders. Terrain specular colour now
  used (SIG red or albedo red × spec colour). Per-frame values go through a
  small float texture rewritten on the GPU (no material updates).
- Sky fixes: bottom ambient (`colorOffset`'s ground branch); the cloud shade
  colour now includes the exposure (`colorOffset` applies it).

**Verified (2026-10-08, RTX 3080, 2400×1350):**
1. Caustics: 64 frames `WaterCaustics00…63`, 256² DXT1, 9 mips.
2. Screenshots `out/m8c3-start.png` (caustics on the sand, turquoise),
   `out/m8c3-below.png`, `out/m8c3-dusk-under.png` (green, orange glints, as
   the user's dusk shot), `out/m8c3-night-under.png` (dark navy).
3. `--benchmark 120 --gpu-timings` at the lifepod: mean 8.7 ms (was 11.0
   with Bevy's lighting), opaque pass 1.3 ms.
4. Workspace tests, real-data tests (5), clippy, fmt: pass.

**Not done:** per-pixel water settings (camera's used), sun shadows, object
specular/gloss/emission maps, light shafts, stars.

**Dead end:** `cast` is a reserved word in WGSL (shader failed to compile on
the first run).

## 2026-10-07 — M8c2: the game's sky (dome and sky map); sun direction fix

**What:**
- Decoded uSky's skybox and sky-map shaders and `uSkyManager`'s inputs;
  written up in `docs/formats/sky.md`.
- `sn-unity`: `SkyManager` now reads every field to the object's end
  (`SkyDome`: planet, clouds, night sky, moon, textures). `sn-assets`:
  `sky_textures`. Real-data test `sky_system_values` extended.
- `sn-client`: `sky_common.wgsl` (the sky function), `sky_dome.rs` +
  `sky_dome.wgsl` (sky map 256² with mips each frame; the dome where the
  depth buffer is empty, before the fog). The water surface reflects the sky
  map (its mean-colour stand-in is gone).
- **Fix (M8b):** the game's sky, day/night factors, water fog and water
  surface use `uSkyManager.SunDir`, a plain hour angle, not the directional
  light's direction (which always points down, moon-like at night). We used
  the light's: wrong by 7° at 09:36 and a day sky at night. Unit test
  `water_sun_turns_with_the_hour`. Also the fog clamp no longer renormalises
  (as the game).
- Fix: `Texture2D.color_space` doc comment had the meaning backwards (1 =
  sRGB colour, 0 = linear data; confirmed on terrain albedos vs normal maps).
  The code was right.
- Benchmarks no longer attach the free camera (mouse/keyboard input in the
  window had moved the camera during earlier screenshots — the likely cause
  of the odd M8c1 start shot).

**Verified (2026-10-07, RTX 3080, 2400×1350):**
1. `uSkyManager` read to its last byte; values equal UnityPy's.
2. Screenshots: `out/m8c2-above.png` (09:36, blue sky, clouds),
   `out/m8c2-up.png` (sun disc and halo), `out/m8c2-dusk.png` (20:30, orange
   clouds, glint path), `out/m8c2-night.png` (23:00, dark sky),
   `out/m8c2-below.png` (sky through the surface), `out/m8c2-start.png`
   (lifepod, unchanged). **Not compared with the game yet.**
3. `--benchmark 300 --gpu-timings` at the lifepod: sky 0.07 ms, water surface
   0.14 ms, mean 11.0 ms.
4. Workspace tests, real-data tests (5), clippy, fmt: pass.

**Dead end:** the first sky came out washed-out grey: our `beta_r()` helper
returns the game's `BetaR` × 1000 (the form the ambient colour code needs);
the sky shader needs `BetaR` itself.

## 2026-10-07 — M8c1: the water surface, ported from the game

**What:**
- Decoded the game's water surface: `WaterSurface` (class and scene values),
  its four shaders (surface, interpolate, normals, foam; render states from
  the shaders' parsed form), where the 64 baked wave frames and the foam
  textures are; written up in `docs/formats/water.md` § Water surface.
- `sn-unity`: `WaterSurface` reader, `AnimationCurve` (+ evaluate),
  `SkyManager` now reads on to `meanSkyColor`. 2 unit tests.
- `sn-assets`: `water_surface` (settings, frames in catalog order, foam
  textures), `Assets::catalog_object` (an object named by a catalog
  location; the prefab loader now uses it too). Real-data test
  `water_surface_values`, mean sky colour added to `sky_system_values`.
- `sn-client`: the fog model moved into `water_common.wgsl` (shared, same
  maths); `water_surface.rs` + `water_sim.wgsl` + `water_surface.wgsl`: the
  per-frame displacement / normal (mips) / foam maps and the surface, drawn
  after the fog pass over a copy of the fogged image. `--no-water-surface`.
  `--no-water-fog` now keeps the fog uniform (fog switched off in it). The
  benchmark logs the final camera position.

**Verified (2026-10-07, RTX 3080, 2400×1350):**
1. Our `WaterSurface` reading ends exactly at the object's last byte; values
   equal UnityPy's (patch 2000 cm, 5 s, gloss 400, curve 3.98 at 47 m, …);
   64 frames `WaterFrame00…63` in order, 256² RGBA32 linear; foam 1024²
   and 512² DXT1 sRGB. Displacement over all frames: x −65…72, y −71…75,
   z −67…68 cm.
2. `--benchmark 300 --gpu-timings` at the lifepod: wave maps 0.05 ms,
   surface (copy + draw) 0.55 ms, fog 0.08 ms; mean 10.8 ms.
3. Screenshots `out/m8c1-above.png` (camera at y = 4), `out/m8c1-below.png`
   (y = −6, looking up), `out/m8c1-start.png` (lifepod, unchanged: surface
   out of view); camera positions confirmed in the log. Near-white (foam)
   pixels: 1.9 % of the water from above, 2.8 % from below. **Not compared
   with the game yet.**
4. All tests (workspace, real-data `sn-assets`), clippy, fmt: pass.

**Not done / known:** no sky dome yet, so the reflected sky is the mean sky
colour and the sky above water is still our clear colour (M8c2); clip map
(cut-outs for bases/lifepod, shore foam) not read; "High" quality FFT waves
not ported. Our mesh differs from the game's adaptive patches.

**Dead end / unexplained:** one benchmark screenshot at the lifepod start
showed an above-water view; three later runs at the same start (camera
position logged) were right. Not reproduced; the window had a different
size in the odd runs (2400 vs 1600 wide). The uncommitted work of this
milestone was also lost once by a local repository reset and redone.

## 2026-10-07 — Build fix: debug `sn-client` failed to link (no code change)

**What happened:** `cargo build -p sn-client` (debug) failed with `LNK1120:
211 unresolved externals`, all Bevy generic instantiations with
`.llvm.<hash>` suffixes. Cause: stale/corrupt incremental artifacts in
`target/debug`, most likely from two cargo builds writing the same target
directory at once (a cargo build was waiting on the file lock during the
fix). Release builds were not affected.

**Fix:** `cargo clean -p sn-client`, then rebuild. Verified: debug and release
`--benchmark 30` both run (start area 1,393 batches; 89 / 95 fps).

**Not ours:** the Vulkan loader logs `ERROR … Failed to open JSON file …
EOSOverlayVkLayer-Win64.json` at start-up: a Vulkan layer registered by the
Epic Games overlay whose file is missing. Harmless.

## 2026-10-07 — M8b: the game's sky values (sun, ambient, sky fog) for the fog and the sun light

**What:**
- `sn-unity::sky`: Unity `Gradient` (read + evaluate), `SkyLight`
  (`uSkyLight`), `SkyManager` (`uSkyManager` up to the sky fog);
  `sn-assets::sky`. 2 unit tests, real-data test `sky_system_values`.
- `sn-client/sky.rs`: clock → sky timeline, sun path, day/night factors,
  sun colour × intensity, top ambient (with the class's Rayleigh colour
  offset), sky fog; `--time` (default 09:36, a new game's start). The fog
  now uses these instead of placeholders, and Bevy's sun takes the game's
  direction and colour (8000 lux per game light unit). 3 unit tests.
- How the layouts were found: UnityPy's type-tree generator on the game's
  assembly, once, in a throwaway venv in `out/` (`TypeTreeGeneratorAPI`).

**Verified (2026-10-07, RTX 3080):**
1. Our readers give exactly UnityPy's values (sun intensity 1.37, exposure
   0.66, sun direction −141°, gradient key times, …).
2. Our Rayleigh port reproduces the class's reference coefficients
   (5.81, 13.57, 33.13); noon sun points straight down, 06:00 is 25° up.
3. At 09:36: sky timeline 10.40 h, sun 73° up, colour (0.89, 0.73, 0.54),
   top ambient (0.042, 0.064, 0.098).
4. `--benchmark 120 --gpu-timings`: 15,276 objects, mean 9.4 ms; fog pass
   0.3 ms GPU. Screenshots `out/m8b-sky-overview.png`, `out/m8b-sky-closeup.png`
   (warmer sunlit sand). **Not compared with the game yet.**
5. Unit, real-data tests, clippy, fmt: pass.

**Dead ends:** UnityPy could not resolve the scene's scripts (they live in
another file); the objects were found with our own reader and decoded with
UnityPy by class name. The first attempt also read path id 390 from the
wrong serialized file of the bundle (204 bytes; the scene's is 716).

## 2026-10-07 — M8b (first pass): the game's underwater fog; plan for missing objects

**What:**
- Decoded the game's water fog (the compiled fog shader, its constant layout,
  the scene's `WaterscapeVolume` values, and how `WaterBiomeManager` fills its
  volume textures); written up in `docs/formats/water.md`.
- `sn-unity::WaterscapeVolume`, `sn-assets::water_volume`.
- `sn-client`: HDR camera with a readable depth buffer; `water.rs` (water at
  the camera from the biome map, batch overrides and the 145 biome settings,
  blended over 16 m cells, into a per-frame uniform) and a full-screen pass
  `water_fog.wgsl` before tone mapping, replacing Bevy's distance fog.
  `--fog-unit` (light calibration), `--no-water-fog` (HDR without the pass),
  `--gpu-timings` (GPU time per render pass at the end of a measurement).
- Roadmap: M7d spawn slots (the 90,289 `EntitySlotsPlaceholder`s; likely
  much of the missing small vegetation), M7e terrain grass, M7f scenes (the
  Aurora is the `aurora.unity` scene: 3,189 GameObjects; Lifepod 5 is in
  `escapepod.unity` and placed at run time) and skinned meshes; M8b next
  steps (sky values, atmosphere volumes, calibration against the game); M8c
  surface lighting, caustics, water surface, sky.

**Verified (2026-10-07, RTX 3080, game closed):**
1. `--benchmark 300 --gpu-timings` twice: identical object counts (15,276),
   0 warnings; mean 10.1 / 9.95 ms (≈ 100 fps; ≈ 8 ms before M8b), peak
   1.6 GiB; GPU time of the fog pass 0.062 / 0.067 ms (opaque pass 1.3 ms).
2. `--flythrough 1700 -80 0`: mean 4.62 ms (217 fps), worst 137 ms, 1.59 GiB.
3. Screenshots `out/m8b-overview.png`, `out/m8b-closeup.png`: turquoise
   near the seabed, blue into the distance, kelp silhouettes in the haze.
   **Not compared with the game yet** (sky values are placeholders).
4. Unit tests (1 new: coefficients), clippy, fmt; all real-data tests
   (incl. the scene's fog values) pass.

**Dead ends:** the first measurements (140–220 ms per frame, then two
`DeviceLost` crashes) were taken while `Subnautica.exe` and another
`sn-client` were running on the same GPU; with the GPU free, no crash and
the pass costs 0.06 ms. Several later runs sat at ≈ 16.6 ms regardless of
the shader (presentation pacing, not GPU work); `--gpu-timings` was added to
measure passes directly.

## 2026-10-07 — M8a: biomes and water settings, headless

**What:**
- `sn-world::biomes`: `BiomeMap` (`biomeMap.bin`: size from its last two
  bytes), `parse_biome_names` (`biomes.csv`), `BatchRootSettings` (a batch's
  override biome and fog from `LargeWorldBatchRoot`). 3 tests.
- `sn-unity::water`: `WaterBiomeManager` and `WaterSettings`.
- `sn-assets::water_biomes` (main scene); `main_scene` helper shared with the
  terrain materials. `sn-install::read_biome_map`.
- `sn-inspect biomes` and `biomes --at X Y Z`; real-data test
  `water_biomes_and_biome_map`.
- M8 split into M8a (data), M8b (underwater look), M8c (surface, caustics,
  sky) in `docs/DESIGN.md`; findings in `docs/formats/water.md`.
- How it was found: the game's classes read with `ilspycmd` into the
  gitignored `out/decompiled/` (same tool as for `Voxeland` earlier); nothing
  copied.

**Verified (2026-10-07):**
1. `sn-inspect biomes`: 145 biomes with plausible values (e.g. Safe Shallows
   absorption 125/20/4, 28 °C; lava 80 °C); map 1024 × 1024, 4 voxels per
   cell, 16 indices used; 467 batches override their biome. Biomes without
   settings: only `void` (open ocean) and `EmperorFacility` (the game falls
   back to its defaults for those).
2. `biomes --at`: lifepod (0, −10, 0) → `safeShallows`; (1700, −80, 0) →
   `crashZone`. Other places not checked against the game.
3. Unit, real-data tests, clippy, fmt: pass.

## 2026-10-07 — M7c: world objects in the client

**What:**
- `sn-client/objects.rs`: a worker thread (own asset index, catalog,
  `prefabs.db`) reads a batch's baked cells, loads each placed prefab once
  and sends new textures, materials, meshes (one per sub-mesh, only the
  vertices it uses; z flipped, winding reversed, tangent w negated) and the
  batch's instances. The main thread uploads assets once and spawns one
  entity per placed mesh part. Cell level *n* is shown while its batch's
  terrain level of detail is ≤ *n* (our choice). The worker forgets loaded
  bundles beyond 512 MiB (`Assets::trim_cache`).
- `object_look.rs` + `object.wgsl`: standard material with `_MainTex` ×
  `_Color` (texture scale/offset), alpha clip (`MARMO_ALPHA_CLIP`,
  `_Cutoff`), blending (render queue ≥ 2500, `MARMO_ALPHA`, `WBOIT`,
  `_ALPHAPREMULTIPLY_ON`), two-sided when `_MyCullVariable` = 0, and the
  game's DXT5nm normal maps through Bevy's mikktspace helpers.
- Shared texture upload (`textures.rs`, moved out of `terrain_look.rs`).
- `Prefab::visible_nodes` falls back to the most detailed LOD level with a
  plain mesh (LOD 0 is sometimes only skinned, e.g. `BrainCoral`); nodes
  record `skinned`. `sn-inspect prefab --placed` reports skinned prefabs.
- Not drawn: creatures (`WorldEntities/Creatures/…`: they move and animate;
  their spawn points were drawn frozen, e.g. Reefbacks hanging in the water),
  skinned meshes, extra (multi-pass) materials, LOD levels > the best one.
- `--no-objects`; stats log objects per cell level; benchmark/flythrough wait
  for objects as well as terrain.
- `sn-client` now depends on `sn-unity` (reads object materials).

**Verified (2026-10-07, RTX 3080, 1600×900):**
1. `--benchmark 120` twice: 15,276 entities (cell levels 0–3: 4,670 / 4,333 /
   1,582 / 4,691) from 775 prefabs, 0 warnings, identical in both runs;
   mean 8.27 / 8.06 ms (121 / 124 fps), worst 18.9 ms; peak memory 1.57 GiB
   (terrain only: 6.1 ms, 1.16 GiB).
2. `--flythrough 1700 -80 0`: mean 3.94 ms (254 fps), p95 6.4 ms, worst 136 ms
   (spikes while assets upload; not investigated), peak 1.59 GiB; objects
   despawn and respawn per level as the camera moves.
3. Screenshots (`out/m7c-overview.png`, `out/m7c-closeup.png`): coral and
   plants sit on the matching purple/orange terrain patches, kelp and a wreck
   piece in place; the floating boulder in the overview is
   `FloatingStone4_Floaters` (the game's floating rocks, minus the Floater
   creatures). **Not compared with the game side by side.**
4. `cargo test --workspace` (2 new tests: mirrored transforms, level
   mapping), clippy, fmt, real-data tests: pass.

**Dead ends:** first run peaked at 4.31 GiB: the worker's bundle cache kept
every decompressed bundle, and every sub-mesh copied its mesh's full vertex
arrays. Cache cap + per-sub-mesh vertices: 1.6 GiB. The double-sided flag
bit in Bevy is `1 << 4`, not `2`; caught before running.

## 2026-10-07 — M7b: prefabs → meshes

**What:**
- `sn-unity`: `Catalog` (Addressables catalog: own JSON + base64 readers),
  `Mesh` + `MeshGeometry` (interleaved vertex streams with every vertex
  format, and Unity's compressed meshes), `MeshFilter`, `MeshRenderer`,
  `GameObject`, `TransformNode`, `LodGroup`, `AssetBundleManifest`.
  7 unit tests (incl. a corrupted-catalog test).
- `sn-assets`: `Assets::catalog`, `Assets::prefab` (key → bundle →
  container → hierarchy with meshes, materials, LOD levels, active flags),
  `Assets::mesh` (inline or `.resS` vertex data).
- Fixed on the way: externals named `Library/…` (capital L) didn't resolve
  to Unity's built-in resources; `Assets::standalone` keyed files by their
  relative path, so `FileRef::file` could panic; `resource_range` could
  overflow on a bad offset.
- `sn-inspect prefab <KEY>` (hierarchy, OBJ export to `out/prefabs/`) and
  `prefab --placed [--oracle]`.
- Opt-in real-data test `prefabs_resolve_and_meshes_decode` in `sn-assets`.
- Format notes: `docs/formats/unity.md` (prefab/mesh classes, prefabs,
  Addressables). No new dependencies.

**Verified (2026-10-07):**
1. `sn-inspect prefab --placed`: all 1,369 placed prefabs load (324 with LOD
   groups, 457 draw nothing); 3,263 distinct meshes, 16.6 M vertices, 13.9 M
   triangles; 0 errors; 5.4 s.
2. Oracle: the same 3,263 meshes decoded by UnityPy (throwaway script in
   `out/`) agree on name, vertex count and triangles per sub-mesh; position
   sums agree to float rounding (max 1.29 on sums up to 2.6 × 10⁷).
3. OBJ exports have plausible sizes (coral 0.4–3 m, Aurora hallway
   44 × 19 × 58 m). Opened in Blender by the user (2026-10-07): the models
   came in far from the origin (export bug, fixed, see below); not re-checked
   in Blender after the fix — bounds now start at the pivot.
4. `cargo test --workspace`, clippy, fmt: pass; real-data tests pass.

**Dead ends / surprises:** the first export of `Spiral_blue_thing_cluster_07`
was 10 km wide: its root has scale 0.0001 and the export left the root out.
The placements carry that scale too, which confirms that a placement
replaces the root transform; stand-alone exports now apply the root's
rotation and scale. The user's Blender check then showed models far from
the origin: the export had also applied the root's *position* (where the
prefab sat in the artist's scene, e.g. −702, −105, −772). Left out now;
pivots sit at the origin (corals rest on y ≈ 0). 52
meshes are compressed (first run: 52 errors), so the compressed form had to
be decoded rather than skipped.

## 2026-10-06 — M7a: world object placements, headless

**What:**
- `sn-world`: `wire` (minimal protobuf wire reader, checked, byte offsets in
  errors) and `entities`: `ObjectTree` (batch objects), `BatchCells` (baked
  cells), `parse_prefab_database`, world transforms through the parent chain.
  `BatchCoord::from_file_name` (octree names now use it too). 6 + 3 new tests,
  including every truncation and byte flip of a sample.
- `sn-install`: `read_batch_objects`, `read_batch_cells`,
  `read_prefab_database`, `object_batches`, `cell_batches`; directory scans
  share one helper.
- `sn-inspect entities <X> <Y> <Z>` and `entities --all`.
- Opt-in real-data test `crates/sn-install/tests/entities.rs`.
- Format notes: `docs/formats/entities.md`.
No new dependencies: the wire reader is ~150 lines of our own.

**How the format was found:** throwaway Python dumpers in `out/` (not
committed) tried a grammar on all files before any Rust was written.

**Verified (2026-10-06):**
1. `sn-inspect entities --all`: 2,975 batch-object files (5,779 objects) and
   1,606 cell files (437,003 cells, 414,067 objects), 0 errors, 0 unresolved
   ClassIds, 0 missing parents, about 1.0 s. Two runs print the same hash
   (`660cae260060d787`).
2. Positions: 63 batch roots are stored off their batch corner (moved back;
   hypothesis in the format doc), 1,473 cell objects sit at exactly the
   world origin, 32 others are outside their batch (30 within 13 m). Cell
   roots lie on a 16 m (level 0) / 32 m (levels 1–3) grid.
3. `cargo test --workspace`, clippy, fmt: pass. Real-data test passes
   (`cargo test --release -p sn-install -- --ignored`).

**Dead ends:** the first prototype read one cells file and took header field
2 (256) for a constant; across all files it is the cell count, and the
version is 9 or 10. Caught by the real-data test, which asserted version 9.

## 2026-10-06 — M6b: the game's per-level cap on material layers

**Why:** the redone M6b drew every type of every chunk at every distance:
~8,400 blended draws in the start area, ~9 ms per frame. The game doesn't.
`StreamingAssets/SNUnmanagedData/clipmaps-{high,medium,low}.json` (the
terrain clipmap settings) cap the types per chunk per level (`maxBlockTypes`,
"high": 32, 8, 2, 1, 1), and the chunk mesher keeps the most-used types
(by face count) before sorting by layer. The files also confirm
`chunkMeshRes` 16 and the 9-vertex mesh at level 0 only.

**What:** `LayerSettings::max_types`; the client caps our levels 0–3 at
32, 2, 1, 1 (matched to the game's levels by sample spacing; that mapping is
ours). One new test in `sn-mesh` (8 there now).

**Verified (2026-10-06, RTX 3080):**
1. `cargo test --workspace`, clippy, fmt: pass.
2. `sn-client --benchmark 120`: start area 1,393 batches, 2,050 meshes (was 10,076),
   7.8 M triangles; mean 6.06 ms (165 fps, was 12.4 ms), p95 6.5 ms.
3. `--flythrough 1700 -80 0`, three runs: mean 15.4, 3.2 and 3.8 ms. The first
   run is an outlier (the GPU sat at 225 MHz idle clocks; not investigated
   further); the other two give 315 / 260 fps. Peak memory 1.16 GiB.
4. Visual: near terrain unchanged (`out/client-benchmark.png` vs
   `out/client-benchmark-uncapped.png`); far chunks show only their dominant
   materials. **Not compared against the real game.**

## 2026-10-06 — M6b redone: the game's own material layering and terrain shader

**Why:** the first M6b (entry below) didn't match the game, and broke docs.
- It painted the batch's "base" material (lowest layer, ties by triangle count)
  under the *whole batch*, then dithered the other materials over it. Where an
  overlay faded, a material from elsewhere in the batch showed through: the
  random green patches on sand in its screenshot.
- The Laplacian smoothing and Bayer-dither `discard` were invented. The game
  uses real alpha blending, and none of the material's border settings were
  used.
- Its MODLOG/README claims ("completely eliminated", 300+ fps with blending)
  were not backed by the screenshot. It also deleted the README's controls,
  licence and credits sections, rewrote the DESIGN tree glyphs, put a literal
  `\n\n` into `terrain-materials.md`, and added a stray space to the M4 entry.
  All restored.

M6 mistakes, also fixed:
- The cap texture was used for every face with a large |normal.y|, so
  overhangs and cave ceilings got sand. The game uses the cap only on upward
  slopes, with a ragged switch to the side texture (`_CapBorderBlend*` + cap
  alpha).
- Projections were in Bevy space with other axes: X (z,y) / Z (x,y) instead
  of the game's X (y,z) / Z (y,x) in Unity space. The textures were mirrored
  and rotated against the game.
- The triplanar exponent was a fixed 4, not each material's `_TriplanarBlendRange`.
- Material colours were used as stored (sRGB). The game is linear-colour-space
  (PlayerSettings, read with UnityPy), so they are converted.
- Doc claimed both kinds share one shader and listed 133 plain / 100 cap types.
  Really there are 3 shaders and 114 plain / 119 cap/side types.

**What:**
- Found how the game does it (documented in `docs/formats/terrain-materials.md`):
  property values of all materials (new `sn-inspect terrain-materials --props`),
  the terrain shaders' Direct3D bytecode (LZ4 blob in the `Shader` object,
  disassembled with Windows' `d3dcompiler_47.dll`, by throwaway scripts in
  `out/`), the scene's `Voxeland.chunkSize` (16), and the chunk mesher in
  `Assembly-CSharp.dll` (read for behaviour only).
- `sn-mesh::build_layers` (new, pure, 7 tests): per-chunk layers ordered by
  (`layer`, type id), weights = share of adjacent faces, 9-vertex faces at
  level of detail 0.
- `sn-assets`: materials carry blend settings, SIG maps, specular colours and
  emission scales.
- `sn-client`: rank 0 opaque, later ranks alpha-blended in order (shared
  bounding box per batch plus a depth bias = rank). `terrain.wgsl` is a port of
  the game's shader: cap/side by slope, border alpha from weight + splotch,
  border tint, SIG emission, gloss → roughness. 4 new tests. Stats log the
  number of meshes on screen.

**Verified (2026-10-06, RTX 3080, 1600×900):**
1. `cargo test --workspace`, `cargo clippy --workspace --all-targets`: pass, no warnings.
2. `sn-inspect terrain-materials`: result OK, 211 textures (183 + 28 SIG maps).
   Real-data test `sn-assets` (`-- --ignored`) updated to 211 textures, 119 cap/side types; passes.
3. `sn-client --benchmark 120`: start area 1,393 batches, 10,076 meshes,
   9.7 M triangles; mean 12.4 ms (81 fps), p95 15.6 ms. With the blended layers
   skipped (temporary experiment): 1,653 meshes, 3.3 ms. So the ~8,400 blended
   draws cost ~9 ms. Without face subdivision: 5.4 M triangles but no faster
   (14.4 ms), so the cost is draw calls, not triangles.
4. `--flythrough 1700 -80 0`: mean 5.1 ms (198 fps), p95 16.8 ms, worst 82 ms,
   peak memory 1.29 GiB. LOD-0 meshing mean 427 ms (was ~400 ms).
5. Visual (`out/client-benchmark.png` vs `out/client-benchmark-m6b.png`): no
   foreign patches, coral/grass borders are ragged blobs instead of voxel
   steps, and cliff tops are sand turning into rock by slope. The game itself
   was not run side by side: **not compared against the real game.**

**Not done:** specular colour (the game's deferred lighting is custom; unchecked),
lava flow animation, the game's lighting/fog (M8), and fewer draw calls
(merge layers across batches, or one bindless/texture-array material).

**Dead ends:** the first M6b's screen-space dither (below) was dropped in
favour of real alpha blending. Reading the shader's binding table, our first
parse got the offsets wrong: entries are 4-byte aligned from the start of the
blob, not from the start of the file.

## 2026-10-06 — M6b: soft blending between terrain materials (superseded; see above)

**What:**
- `sn-client`: soft blending between adjacent terrain materials, eliminating blocky voxel borders.
- Material layer hierarchy: terrain materials in Subnautica specify `VoxelandBlockType.layer` (e.g.
  `Sand02ToCoral15` = -100, `SS_SandToRock` = -99, `Sand02` = 0). Lower layers serve as the base
  strata foundation; higher layers are overlays.
- Mesh partitioning (`split_by_material`): builds an adjacency graph, computes normalized per-vertex
  material shares, and performs 1 step of Laplacian smoothing. The base material covers the entire
  batch (uvs = [1.0, 0.0], 100% solid, prevents cracks/holes). Overlay materials extend across
  boundaries and carry smoothed blend weights in vertex UVs (`Mesh::ATTRIBUTE_UV_0.x`).
- Shader (`terrain.wgsl`): added a 4×4 Bayer screen-space ordered dither discard when `blend_w < 0.999`.
  Fragments with `smoothstep(0.0, 1.0, blend_w)` below the Bayer threshold are discarded, revealing the
  underlying base material beneath. Discard avoids transparency sorting artifacts and z-fighting,
  keeping full depth buffer correctness.
- `sn-inspect materials`: displays the `layer` column in material listings and region census.
- Unit tests: `test_split_by_material_single_material` and `test_split_by_material_blending` in `sn-client`.

**Verified (2026-10-06):**
1. Benchmark on real Subnautica assets (`cargo run -p sn-client --release -- --benchmark 60`):
   Start area loaded in 1.72 s (1,393 batches, 6,967,128 triangles).
   60 frames rendered at mean 3.05 ms (328 fps), p95 3.64 ms, worst 4.03 ms. Peak memory 0.96 GiB.
2. Visual comparison: `out/client-benchmark.png` vs pre-M6b `out/client-benchmark-before-m6b.png`.
   Blocky voxel staircases at material boundaries are completely eliminated; sand, rock, and coral
   blend naturally and smoothly.
3. Tests & clippy: 40 tests passed across workspace (including blending unit tests). Clippy clean.

## 2026-10-06 — M6: textures and real terrain materials

**What:**
- `sn-unity`: `Texture2D` (2019.4 layout) with `decode_rgba` (DXT1/5, BC4/5/7, RGBA32, RGB24, Alpha8, …);
  `PPtr`, `Material`, `MonoScript`, `MonoBehaviourHeader`; game scripts `Voxeland`, `VoxelandBlockType`,
  `VoxelandBlockTypePrefab`; `BundleDirectory` (header + directory only, from a file prefix).
- New dependency: `texture2ddecoder` 0.1.2 (MIT OR Apache-2.0, pure Rust): block-compressed texture decoding.
- New crate `sn-assets` (layer 3): bundle index from directory prefixes (5,467 bundles in ~0.5 s), cached
  loading of bundles *and* standalone files (`resources.assets`; `.resS` read by byte range), cross-file
  `PPtr` resolution (`archive:/CAB-…`, player files, `library/` → `Resources/`), `terrain_materials`.
- `sn-install`: `read_file_prefix`, `read_file_range`.
- `sn-inspect`: `textures [--pixels]`, `textures --census`, `terrain-materials`.
- `sn-client`: real terrain look. DXT textures uploaded compressed with mip chains, `terrain.wgsl` triplanar
  shader on top of Bevy's standard material (cap/side layers, DXT5nm normal maps, whiteout blend), built-in
  shader via `embedded_asset!`; `--debug-colours` keeps the old false colours.
- `docs/formats/terrain-materials.md`; Texture2D/Material layouts in `docs/formats/unity.md`.

**Findings:** texture formats are DXT5/DXT1 (+ a few RGBA32/Alpha8/RGB24, one BC7), **no crunch**. Type ids map to
block types from the scene's `Voxeland.types` (56) and `BlockPrefabs` prefabs keyed by `globalId` (233, plus 10 with
id 0, skipped). The layouts came from the game's own DLLs via UnityPy's type-tree generator (dev machine only).
Terrain materials are plain (`_MainTex`…) or cap/side blends (`_CapTexture`/`_SideTexture`…); normal maps are DXT5nm.

**Verified (2026-10-06):**
1. Texture metadata identical to UnityPy for 1,062 textures in 43 files; decoded pixel CRCs identical for 506
   textures (all formats in the sample). The first run differed only for Alpha8: I had guessed (255,255,255,a);
   UnityPy and the GPU give (0,0,0,a). Fixed.
2. `sn-inspect terrain-materials`: every type id used by the octrees (209 non-empty) has a material with cap and
   side textures; 183 textures, read in ~0.2 s.
3. Real-data test `sn-assets`: 233 materials, all textures decode completely, scales sane. Workspace: 47 tests,
   clippy clean.
4. Client: flythrough to the crater edge 374 fps mean, worst frame 45 ms, peak memory 0.90 GiB, 138 MiB of terrain
   textures on the GPU. Screenshots (gitignored) show sand with ripples, porous and striated rock, moss.

**Dead ends / bugs:** (a) only 56 of 210 types had materials until the `BlockPrefabs` were found; (b) the first
"conflict" check compared material objects, but the scene keeps its own copies; comparing names leaves 4 real
conflicts; (c) type 0 briefly got a material from prefabs with `globalId` 0, caught by the real-data test;
(d) references to `library/unity default resources` needed mapping to `Resources/`.
**Not done / known issues:** no soft blending between materials, so borders follow the voxel grid and patchy
materials (red grass) look blocky; new roadmap item M6b. `_SIGMap` (gloss/emission) unused. Map mirroring still
unverified. Underwater look (absorption, fog colour) is M8.

## 2026-10-06 — M5: Unity bundles and serialized files

**What:**
- New crate `sn-unity` (layer 1). `Bundle::parse` (UnityFS format 6–8; LZ4/LZ4HC/stored blocks; LZMA is
  reported as unsupported because the game has none) and `SerializedFile::parse` (versions 14–22: types, optional
  type trees, object table, externals) with `object_data`. Also `class_name` (built-in class ids) and
  `write_bundle` (synthetic bundles for tests). Bounds-checked reader: errors carry byte offsets.
- New dependency: `lz4_flex` 0.14 (MIT, pure Rust): LZ4 block decompression for bundles.
- `sn-install`: `data_dir`, `bundle_dir`, `read_file`, `serialized_files`, `bundles`.
- `sn-inspect unity <FILE>…` (canonical listing), `unity --types` (with class names), `unity --all`.
- `docs/formats/unity.md`.

**Findings:** Unity 2019.4.36f1, serialized format 21 and UnityFS 7 everywhere; **no type trees in any file**
(the risk from DESIGN.md is real); no LZMA. 5,472 files, 5,485 serialized files, 423,677 objects.

**Verified (2026-10-06):**
1. Oracle: UnityPy 1.25.4 in a temp venv (outside the repo), with a summary script in the same temp folder.
   Ran on 35 files: the 5 player files, the 5 largest bundles and 25 random bundles (seed 5).
   Our `sn-inspect unity` output is **identical** (395 lines: versions, type and object counts per class,
   object byte totals, externals, resource sizes).
2. `sn-inspect unity --all`: every file parses, 0 errors, 2.9 s.
3. `cargo test -p sn-unity -- --include-ignored`: synthetic round trips (with/without LZ4), every truncation
   and every single-byte corruption parses without panicking, and the real-data test (5,472 / 5,485 / 423,677,
   0 type trees) passes. Workspace: 42 tests, clippy clean.

**Dead end:** the first oracle diff failed only on Windows line endings (`\r\n` from Python), which had also
crept into the random file names. Fixed in the harness, not the code.
**Scope change:** Addressables catalog moved from M5 to M7 (first user; avoids JSON/base64 dependencies now).

## 2026-10-06 — M4: terrain streaming and levels of detail

**What:**
- `sn-octree`: `Octree::sample` / `Batch::sample` look up one voxel by walking down the tree, so coarse
  levels never expand whole batches.
- `sn-mesh`: `Field::with_step` (sample spacing 1/2/4/8) and `add_skirts` (strips hanging from open edges
  into the solid, to hide cracks between levels of detail).
- `sn-terrain`: works on parsed `TerrainBatch`es (sampling) instead of fully expanded grids; `batch_field` /
  `batch_mesh` take a level of detail (0..=3). `sn-install::load_batch` returns a validated `TerrainBatch`.
- `sn-client`: `TerrainStreamer`. Worker threads with a queue rebuilt on every 8 m of camera movement
  (missing batches first, then nearest); level of detail by distance to the batch box
  (100 / 260 / 600 / 1200 m, scaled by `--view`) with 20 m hysteresis; old mesh kept until the new
  one is ready (no holes); unload outside the view; parsed-batch cache with a 1,200-batch limit
  (least-recently-used eviction); at most 300k triangles uploaded per frame.
  New options `--start`, `--look`, `--view`, `--flythrough X Y Z [--speed]`; `--radius` removed.
- `sn-inspect`: `mesh --lod L`, and `voxel X Y Z` (what's at a world position + surfaces in that column).

**Verified (2026-10-06, RTX 3080, debug build):**
1. `sn-inspect mesh 12 18 12 --radius 1 --lod 0..3`: 2,031,606 / 493,862 / 118,880 / 27,316 triangles,
   0 open edges on batch seams at every level. Level 0 matches M2/M3 exactly (sampling = expanding).
2. `sn-client --flythrough 1700 -80 0` (lifepod → crater edge, 40 m/s, 42.5 s): process memory
   0.61–0.89 GiB throughout, batch cache capped at 1,200, start area loaded in 2.0–2.3 s, destination
   loaded 0.1–0.25 s after arrival. Request→screen latency per level: lod0 ~400 ms (meshing-bound),
   lod1 ~60 ms, lod2 ~15 ms, lod3 ~10 ms.
3. Frame times vary a lot between identical runs on this machine (mean 104 / 288 / 300 / 405 fps over 4 runs),
   so something else was loading the PC. Best run with the upload cap: p95 3.0 ms, worst 26 ms (before the
   cap: p95 5.3 ms, worst 64 ms).
4. `cargo test --workspace`: 37 passed (sampling = expanding; step scaling; skirts; seams closed at
   lod 0–2; coarser levels have fewer triangles; mixed levels crack and get skirts). clippy clean.

**Investigated, not bugs:** the flythrough's final screenshot is pure fog: the camera looks outward over the
crater-edge drop (seafloor 390 m below, per `sn-inspect voxel 1700 -80 0`). A mid-way screenshot showed a grid
of strips: the straight fly path was 22 m inside rock there (`voxel 800 -40 0` → solid, surface at −18), and the
strips are skirts seen from inside the rock. Players can't be there.
**Not tested:** interactive flying by a human; release build; whether the upload cap helps reliably (noisy machine).

## 2026-10-06 — M3: Bevy client with free-fly camera

**What:**
- `sn-install` (layer 3): `GameData` moved out of sn-inspect; adds `load_batch` (read + parse + rasterize).
- `sn-terrain` (layer 2, pure): `Neighbourhood` (batch + 26 neighbours), `batch_field`, `batch_mesh`,
  `debug_colour`. Moved out of sn-inspect's `terrain.rs`.
- `sn-world`: `VOXEL_WORLD_OFFSET`, `voxel_to_world`, `world_to_voxel` (offset still a hypothesis).
- `apps/sn-client`: Bevy 0.19.1 (+ `free_camera` feature for the built-in fly camera). Loads and meshes
  batches on background threads (all cores but one), splits meshes per material, converts Unity
  left-handed → Bevy right-handed (flip z, reverse winding), distance fog, a stats log every 2 s, and
  `--benchmark N` (N frames without vsync after loading → frame-time stats + screenshot → exit).
- New dependency: `bevy` 0.19.1, the engine chosen in DESIGN.md; latest stable (0.20 is still RC).

**Verified (2026-10-06, RTX 3080, debug build with opt-level 1):**
1. `sn-client --benchmark 600` on 3×3×3 batches around 12-18-12: 2,031,606 triangles,
   loaded in 7.25 s; mean 3.50 ms/frame (286 fps), 95th percentile 4.30 ms, worst 159.75 ms
   (the frame where all meshes upload at once).
2. Screenshot `out/client-benchmark.png` (gitignored) shows recognisable Safe Shallows seabed
   (pillars, arches, rolling sand), lit, false-coloured per type id.
3. `cargo test --workspace`: 30 passed (3 new sn-terrain tests: sphere across 8 batches closed,
   clamped apron adds no surface, absent centre). clippy `-D warnings` clean.
4. `sn-inspect mesh 7 17 10` after the refactor: same numbers as before (220,258 triangles, 0 unexpected open edges).

**Notes:** Vulkan prints a harmless "Loader Message" ERROR at start-up (driver loader noise).
**Not tested:** interactive controls (needs a human); release build; other GPUs; whether the map is mirrored.

## 2026-10-06 — M2: terrain to mesh (OBJ export)

**What:**
- New crate `sn-mesh` (no deps): `Field`, `surface_nets`, `edge_report` (topology checks).
  Chunking rule: the outer sample layer of a field is an apron; neighbouring fields
  overlap by 2 samples; vertices are computed in global coordinates, so they weld
  bit-exactly.
- `sn-octree`: `Voxel::signed_density`, `SURFACE_DENSITY`, `Batch::rasterize` → `BatchGrid`.
- `sn-inspect`: `density X Y Z` (evidence for density semantics), `mesh X Y Z [--radius R]`
  (OBJ + MTL with false colours per type into `out/`, plus hole/orientation checks);
  `octree --all` now also checks density against type world-wide.

**Findings:**
- Density ≤ 125 → empty, ≥ 126 → solid, with no exceptions across 263 M leaves. 0 = far from the surface.
  Gradient is about 15 units/voxel near the surface, compressed further out.
- Missing batch files are uniform (faces touching them are 0% or 100% solid), so
  the apron copies edge voxels outward ("clamp") when a neighbour is missing.
- World offset hypothesis is plausible: seabed at world (0,0) is y = −17; highest terrain y = +156 at (345, 907).

**Dead ends / bugs found by tests:**
- Sharing 1 sample layer between chunks leaves a strip of holes (120 open edges in the
  test). Fixed with the apron + 2-sample-overlap rule.
- Overflow in the neighbour lookup (`(-1) as usize + 1`). Caught at the first real run.

**Verified (2026-10-06):**
1. `cargo test --workspace`: 27 passed (sphere/torus closed with correct Euler characteristic,
   split-field weld identical to whole, materials, origins, batch placement).
2. `sn-inspect mesh 7 17 10`: 220,258 triangles, 0 flipped, 0 unexpected open edges, 99.8% facing water.
3. `sn-inspect mesh 12 18 12 --radius 1`: 27 batches, 2,031,606 triangles, 0 open edges along
   batch seams, 0 flipped, 99.6% facing water, 13.8 s (debug build), 145 MB OBJ.
4. clippy `-D warnings` clean; real-data test still passes.

**Not tested:** how it looks in Blender (human check pending). 1,395 edges are shared by 3+ triangles
(about 0.05%; a known surface-nets artefact at thin features). Mirroring vs the in-game map.

## 2026-10-06 — M1: decode every terrain octree

**What:** Cargo workspace (edition 2024, licence MIT OR Apache-2.0, dev profile
opt-level 1 / deps 3). New crates, all without dependencies:
- `sn-octree`: parse, validate, rasterize, encode (synthetic fixtures); `AxisOrder`;
  confirmed `GAME_CHILD_ORDER` / `GAME_OCTREE_ORDER` = ZYX.
- `sn-world`: `index.txt` parser, `BatchCoord`, partial edge-batch dimensions.
- `sn-inspect`: `index`, `octree X Y Z`, `octree --all`, `orient X Y Z`.
Docs: `docs/formats/optoctrees.md`, `docs/formats/world-index.md`, README, licence files.

**Findings:**
- `index.txt` (not `meta.txt`, as M0 said) holds the dimensions, followed by
  exactly 13,520 per-batch floats (655 non-zero, meaning unknown). `meta.txt` is `39` / `BlockPrefabs`.
- All 669,150 octrees validate; 300,431,310 nodes; largest 37,449 (complete tree).
- Child order and octree order are both ZYX (z fastest). `orient` on 7 batches:
  seam/interior disagreement ratio 0.76–1.11 for ZYX/ZYX, runner-up 1.4–10× worse.
- +Y is up: non-empty share per batch row goes from 100% (Y=7) to 0.18% (Y=19).

**Dead end:** The M1 criterion "≥ 0.95 agreement for each of 8 orderings" was
flawed. Inside one octree all axis orders score the same (0.9942 on 12-18-12),
and even a scrambled order scores 0.985. Seams are what distinguish orders; DESIGN.md is updated.
Also: random batches like 8-15-14 are uniform and carry no orientation signal,
so `orient` now reports them as INCONCLUSIVE.

**Environment issue:** linking failed because the Windows SDK was missing
(rustc doesn't find MSVC `link.exe` without it). The user installed SDK 10.0.26100.
Note: in Git Bash, `/usr/bin/link` shadows MSVC's linker, so run cargo from PowerShell/cmd.

**Verified (all run 2026-10-06):**
1. `cargo test --workspace`: 19 passed, 1 ignored (real data), without the game.
2. `SUBNAUTICA_DIR=… cargo test -p sn-octree -- --ignored`: passed (5,416 files, 5,259×125 + 157×75, max 37,449 nodes).
3. `sn-inspect octree 12 18 12`: node type histogram identical to the M0 Python probe (24 ids, 172:47194, …).
4. `sn-inspect orient` on 12-18-12, 7-17-10, 9-16-5, 4-16-11, 6-17-9, 5-16-19, 22-17-11: ZYX/ZYX best on all.
5. `cargo clippy --workspace --all-targets -- -D warnings`: clean.
6. `git status`: only source and docs.

**Not tested:** Linux/macOS; density/type semantics (M2).

## 2026-10-06 — Scope: desktop only

**What:** Dropped the browser/WASM target from DESIGN.md, AGENTS.md and .gitignore.
Removed `sn-vfs` (browser file access) in favour of a desktop `sn-install` crate;
networking switched from WebSocket to UDP; web milestones removed and the roadmap
renumbered (now M0–M12, ending in a playable multiplayer desktop demo).
**Why:** User decision — the browser made the project much harder for little gain now.
**Verified:** `git status --short` still lists only whitelisted files.

## 2026-10-06 — M0: repository bootstrap

**What:** Added `AGENTS.md`, whitelist `.gitignore`, `docs/DESIGN.md`, this log.
`git init` (no commits).

**Why:** Safety rules and architecture before any engine code.

**Probe of the install (read-only, throwaway Python script outside the repo):**
- Build: `__buildnumber.txt` = `10`, `__buildtime.txt` = `10/03/2025 17:26:27`, data in `SNUnmanagedData/Build18`.
- `meta.txt`: `4096 3200 4096` / `128 100 128` / `32` / `5 5 5`.
- `CompiledOctreesCache`: 5,416 files, all start with i32 version `4`.
- Parsing as `u16 count` + `count × 4-byte node` repeated: every file consumed exactly;
  5,259 files contain 125 octrees, 157 contain 75. Max nodes in one octree: 37,449.
- `compiled-batch-12-18-12.optoctrees` node type histogram (all nodes, incl. internal):
  `0:99021 1:46916 2:2502 3:3357 5:1559 7:8 8:4537 10:297 13:286 14:1697 19:865 20:209
  21:14 22:1503 35:2084 50:393 51:39 52:14 100:2224 101:2538 102:13880 104:805 105:991
  172:47194` (24 distinct ids). Reference values for M1 "Done when" item 4.

**Verified:** `git status --short` lists only the whitelisted files (see below).
**Not tested:** anything about node semantics (child order, density meaning) — that is M1/M2.
