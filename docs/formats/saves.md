# Our server saves (M9f)

This is our own format, never the original game save format.

## State research

**Confirmed (2026-10-27, read-only local decompile):**
`SaveLoadManager.SaveWorldStateToTemporaryStorageAsync` saves scene objects,
the global root and batch cells separately. Its save operation temporarily
unparents the player before serializing the world. `ProtobufSerializer`
uses versioned headers and object identity/class/parent references.
`Inventory` serializes item/equipment references; `Oxygen` stores oxygen;
`Survival` stores food/water. These observations identify state categories,
not a format to reproduce. No code or game content was copied.

**Confirmed (our source):** M9d's `ItemsState` already encodes the locker,
each player's inventory/equipment/known tech/luck, removed world keys,
outcrop damage, item class references, spawned objects and next item id.
Player movement is owner-simulated. M10 only sends life flags, not numeric
vitals, so M9f must add numeric vitals reporting and an own-player restore.
Food/water simulation does not exist yet and is outside this save's state.

See DESIGN's M9f plan for the persistence contract and implementation steps.

## Version 1 (our format)

**Confirmed by source and synthetic tests:** little endian fixed-width
integers, canonical LEB128 counts, UTF-8 length-prefixed strings and checked
finite floats use `sn-protocol::codec`. Header: bytes 0..8 magic `SNRSsave`,
8..10 save version (1), 10..12 protocol version, 12..20 FNV-1a-64 checksum
of the payload, then a varint payload length and the payload. Maximum total
file size is 64 MiB. FNV detects accidental corruption, not malicious edits.
Errors include absolute byte offsets, including nested message/item errors.

Payload in order: build string, rules fingerprint (u64), tick duration
(varint), world settings (existing protocol layout), elapsed ticks (varint),
SplitMix64 state (u64), next player id and player count (varints). Each
player: id, name, a length-prefixed Restore message (optional state and
optional numeric health/oxygen). Finally the existing ItemsState encoding
(protocol.md): inventories/equipment/charges, known and analyzed tech, luck,
locker contents, removed keys, outcrop hits, dropped/spawned objects and
next item id. Item classes are references resolved against each install.
No asset bytes or credentials are included. Protocol/build/rules/tick
mismatches fail; there is no migration or original-game save reader.

World events and connections restart empty; offline player identities/items
remain. World elapsed ticks continue, without simulating time while the
server is stopped. Active players become offline at load and rejoin by name.
Their inventory is sent in world chunks and own pose/vitals in Restore.
The current client's temporary hatch cinematic is cancelled at its saved
pose; velocity/rotation/look and numeric vitals restore. Death/respawn phase
resumes with its phase timer restarted. Breath/suffocation overlay timers
restart locally. Food/water, full cinematic continuation, UI state and
M11 entity ownership are not implemented. These are explicit scope limits,
not claims of byte-identical continuation of client simulation.

## Storage and recovery

The app chooses the platform user-data slot described in DESIGN's M9f plan;
`--save-dir` selects another slot, `--no-save` makes a fresh ephemeral server.
Both dedicated and embedded servers autosave every 30 s (`--autosave S`)
and flush on orderly shutdown. The dedicated server handles Ctrl-C;
`--run-for S` also shuts down cleanly. A forced kill only retains autosaves.
An OS file lock prevents two processes writing the same slot, including
a dedicated and embedded host. Runtime slot files: `world.sav`, previous
validated `world.sav.bak`, staged `world.sav.tmp`, and `server.lock`.

A save is validated against the current install before writing. Existing
primary must also validate, unless explicitly recovering a backup. Backup
and staging files are flushed before replacing the primary. Windows rename
cannot replace a file, so removal/rename has a crash window: missing primary
with backup/temp present refuses a fresh world. Corruption/incompatibility
refuses startup and retains files. `--load-backup` reads the backup explicitly
and preserves it while replacing the primary on the next save. Recovery may
lose progress since the previous save. Power-loss durability is not tested.
Rust 1.89+ is required by sn-server for standard-library file locking.
