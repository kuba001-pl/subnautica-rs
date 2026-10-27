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
