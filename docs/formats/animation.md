# Animation: clips, controllers, avatars, animators

Described build: game build `10` (Unity 2019.4.36f1). Reader:
`crates/sn-unity/src/anim.rs` (M7f4a); runtime: `crates/sn-anim` (M7f4b).
Inspector: `sn-inspect anim <prefab key | scene:NAME> [--states]`.
Layouts were written from UnityPy's Unity 2019.4.36f1 type database (the
files have no type trees, `unity.md`). Each fact is marked **confirmed**
(with how) or **hypothesis**.

## What the game has — confirmed (census 2026-10-10)

Over every bundle and `.assets` file (`cargo test -p sn-unity --test
real_data every_animation -- --ignored`; numbers also counted with UnityPy):

| Class | id | Count |
|---|---|---|
| `Animator` | 95 | 600 |
| `AnimatorController` | 91 | 289 |
| `Avatar` | 90 | 334 |
| `AnimationClip` | 74 | 2,294 (137 MB) |
| `Animation` (legacy) | 111 | 40, all in UI prefabs |
| `AnimatorOverrideController` | 221 | 0 |

Every object of the first four classes parses to its last byte with our
readers (the readers fail on leftover bytes).

**No humanoid rig:** all 334 avatars have an empty human skeleton. Every
clip is *generic*: its curves are bound to properties of objects below the
animator. Clips in the build keep no editor curves except the 20 legacy
ones (rotation 10, Euler 5, position 7, scale 12, float 9 curve lists).

## AnimationClip (74) — confirmed

`string name, bool legacy, bool compressed, bool high quality (align 4)`,
seven editor curve lists (quaternion, compressed rotation, Euler,
position, scale, float, object reference), `f32 sample rate, i32 wrap
mode, AABB bounds, u32 muscle clip size`, then the **muscle clip**:
- a human delta pose (root `xform`, look-at position and weight, goals,
  two hand poses, DoF and TDoF arrays): empty in generic clips;
- four `xform`s (start, stop, left and right foot start), `float3`
  average speed. An `xform` here is `float3 t, float4 q, float3 s`
  (40 bytes);
- **the clip data:** a streamed block (`u32[]` words, `u32` curve count),
  a dense block (`i32` frame count, `u32` curve count, `f32` sample rate,
  `f32` begin time, `f32[]` samples, frame after frame) and a constant
  block (`f32[]`, one value per curve);
- `f32` start time, stop time, orientation offset y, level, cycle offset,
  average angular speed; `i32[]` index array; `(f32, f32)[]` value array
  delta; `f32[]` reference pose; 11 flags (mirror, **loop time**, loop
  blend, three loop-blend axes, start at origin, three keep-original
  flags, height from feet; align 4).

Then `m_ClipBindingConstant` (bindings, `PPtr[]` object curve mapping),
two flags (has generic root transform, has motion float curves) and the
events (`f32` time, function and data strings, `PPtr`, `f32`, `i32`,
`i32` message options).

**Curve order:** curves are numbered streamed first, then dense, then
constant; the bindings list properties in that same order. A Transform
position, scale or Euler binding takes 3 curves, a rotation 4
(quaternion x, y, z, w), everything else 1. Confirmed: in all 2,294 clips
the bindings' curve counts add up exactly to the three blocks' curves.

**Streamed block:** the words are a byte stream of frames, each `f32 time,
u32 key count`, then per key `u32 curve index, f32 a, b, c, d`. A key
holds the curve from its time up to the curve's next key as the cubic
`((a·dt + b)·dt + c)·dt + d` with `dt = t − key time` (so `d` is the value
at the key). The first frame is at `−FLT_MAX` (2,069 clips) and holds every
curve's starting value; the last one is a terminator at `+FLT_MAX`.
Confirmed: frames come in time order, every key names a curve below the
curve count, every stream ends exactly at a frame boundary, and the cubic
of each key reaches the next key's value (5,113,013 keys; largest gap below
10⁻³ relative except 16 *stepped* keys, `a = b = c = 0`, where the value
jumps on purpose).

**Quaternions are blended as 4 numbers.** Sampled mid-segment, a rotation
curve's quaternion can be shorter than 1 (99 % of samples within 0.07,
at worst exactly 1 − 1/√2 for a segment that turns by 180°): the curves are
interpolated component by component and Unity normalises the result
(**hypothesis** for where Unity normalises: after blending, as Mecanim
does for its blend trees).

**Dense block:** values of all its curves at `begin time + i / sample
rate`, frame `i`; between frames linear (**hypothesis**: the block holds no
slopes, so linear is the natural reading; not checked against the game).

### Bindings (`GenericBinding`) — confirmed

`u32 path, u32 attribute, PPtr script, i32 type id, u8 custom type, u8 is
object curve (align 4)`.
- `path` is the **CRC-32 (zlib)** of the object's path below the
  animator's GameObject (`"a/b"`, `""` for the animator's own object).
  Confirmed: the player's 77,373 bindings and the lifepod's 3,425 (LOD 0
  animator) all match a path of their hierarchy.
- For Transforms (`type id` 4), `attribute` is 1 position, 2 rotation
  (quaternion), 3 scale, 4 Euler angles (`custom type` 4). For other
  components it is the CRC-32 of the property name (e.g.
  `blendShape.<name>`).
- Bound properties over all clips: Transform 319,290 (+ 89 Euler),
  blend-shape weights (type 137, custom 20) 1,674, light values (108,
  custom 25) 151, script fields (114) 185, renderer material values (23,
  custom 22) 32, animator parameters (95, custom 8) 78, `GameObject`
  active (1) 17, `RectTransform` (224, custom 28) 14.

## Avatar (90) — confirmed

`string name, u32 size`, then the avatar constant: skeleton (`{i32 parent,
i32 axes}[]` nodes, `u32[]` path hashes, axes of 76 bytes each), skeleton
pose and default pose (`xform[]`), `u32[]` skeleton name ids, the human
part (root `xform`, skeleton, pose, two hands, bone index and mass, 8
floats, 3 flags), two index arrays, root-motion bone index and `xform`,
root-motion skeleton, its pose and index array; then `m_TOS` (path hash →
path) and the human description (bones, skeleton bones, 8 floats, root
motion bone name, 3 flags). We keep the generic skeleton and the name
table; bindings are matched against the actual hierarchy instead.

## Animator (95) — confirmed

`PPtr game object, u8 enabled (align 4), PPtr avatar, PPtr controller, i32
culling mode, i32 update mode, bool apply root motion, bool linear velocity
blending (align 4), bool has transform hierarchy, bool allow constant clip
sampling optimisation, bool keep state on disable (align 4)`. The player's
(`Player/body/player_view`) culls its transforms when off screen (mode 1);
the lifepod's LOD 0 model always animates (0), its LOD 1 culls completely
(2).

## AnimatorController (91) — confirmed

`string name, u32 size`, the controller constant, `m_TOS` (name hash →
name), `PPtr[]` clips, state machine behaviour ranges and indices,
`PPtr[]` behaviours, `bool` multi-threaded (align 4).

The controller constant:
- **layers:** `u32 state machine, u32 synchronised layer, u32[3] body
  mask, {u32 path hash, f32 weight}[] skeleton mask, u32 name hash, i32
  blending (0 override, 1 additive), f32 default weight, bool IK pass,
  bool synced layer affects timing (align 4)`.
- **state machines:** states, any-state transitions, selector states,
  `u32` default state, `u32` synchronised layer count.
- **state:** transitions, `i32[]` blend tree per motion set, blend trees
  (each a node list, node 0 the root), `u32` name, path, full path and tag
  hashes, `u32` speed, mirror, cycle offset and time parameter hashes,
  `f32` speed, cycle offset, flags IK on feet, **write defaults**, loop,
  mirror (align 4).
- **transition:** conditions (`u32 mode, u32 parameter hash, f32
  threshold, f32 exit time`), `u32` destination, full path id, id, user
  id, `f32` duration, offset, exit time, `bool` has exit time, has fixed
  duration (align 4), `i32` interruption source, `bool` ordered
  interruption, can transition to self (align 4).
- **destinations:** below 30,000 a state index of the same machine;
  30,000 + *n* the machine's selector *n* (entry or exit node). Two exit
  selectors in the game point at `0xFFFFFFFF` (nowhere). Confirmed: every
  destination of the game's controllers is in range by this rule.
- **blend tree node:** `u32 type, u32 parameter, u32 parameter y, u32[]
  children`, 1D thresholds, 2D data (positions, magnitudes, pair vectors,
  pair average inverse magnitudes, neighbour lists), direct data
  (parameters, normalised flag, align 4), `u32 clip` (index into the
  controller's clips), `f32 duration, f32 cycle offset, bool mirror`
  (align 4). A node without children is a leaf that plays its clip.
- **parameters** (`m_Values`): `u32 name hash, u32 type, u32 index` into
  the default values of the type: 1 float, 3 int, 4 bool, 9 trigger
  (triggers are stored with the bools). Default values: positions,
  quaternions, scales, floats, ints, bools (align 4).

Name hashes (`Animator.StringToHash`) are **CRC-32 (zlib)** of the name:
confirmed on the controller name tables (e.g. `lifepod_damage`,
`base.idle`). State full paths are `<layer>.<state>`.

What the controllers use (census): parameters float 540, bool 987,
trigger 113, int 2; conditions If 1,612, IfNot 1,166, Greater 26, Less 22,
Equals 6; layers override 369, additive 57, skeleton masks on 43; blend
nodes 1D 139, 2D simple directional 104, 2D freeform directional 5; 29
any-state transitions; 111 transition offsets; interruption by the source
26, the destination 49; write defaults off in 2 states; 1 state machine
behaviour.

## Animators we looked at — confirmed (`sn-inspect anim`)

- **The player** (`Player/body/player_view` in the main scene):
  `player_view_controller`, 9 layers (Base Modes, right arm, left, both,
  upperbody, Cinematics, Cinematics2, Cinematics3, Death), 201 parameters,
  511 clip slots (402 distinct clips). Avatar `player_viewAvatar`, 104
  skeleton nodes.
- **Lifepod 5** (`escapepod` scene): the hull's LOD 0 and LOD 1
  (`escape_pod_controller`, 8 layers: base, top hatch, bottom hatch,
  storage, first aid, circuit panel, player locator, fire extinguisher; 22
  parameters including `lifepod_damage`); the lights
  (`Life_Pod_lights_controller`, its clip switches 4 lights and their
  objects over 10.5 s); two wrenches with an avatar and no controller.

## Runtime (`sn-anim`, M7f4b)

Unity's Mecanim is not published. `crates/sn-anim` runs the controller
by the rules below, from Unity's manual and our reading of the data.
Unit tests on synthetic controllers check each rule
(`crates/sn-anim/tests/animator.rs`); the real-data test
`player_and_lifepod_animators_run` (sn-assets) runs the player's and the
lifepod's animators for 60 s: no NaN, quaternions of length 1.

- **Pose.** Every property any clip of the controller animates is a
  *slot* (path hash + position / rotation / scale / one float). Euler
  curves write the rotation slot (`Quaternion.Euler`: z, then x, then y).
  Defaults are the values before animation (the hierarchy's stored local
  Transforms; blend shape weights from the renderer; other properties 0
  until they are read). Confirmed: the player's 213 slots and the
  lifepod's 137 all find their object.
- **Clip time.** A state's normalised time grows by `dt × speed / length`,
  where length is its leaves' clip lengths × their duration factors,
  weighted (1 s without clips). A looping clip (`m_LoopTime`) samples at
  the fraction, others clamp to 0..1. Offsets add the state's, the leaf's
  and the clip's cycle offsets.
- **Blend trees:** 1D linear between neighbouring thresholds. 2D simple
  directional: barycentric in the triangle of the centre and the two
  children whose directions bracket the input (**hypothesis**).
  2D freeform directional: gradient bands in polar space with the pair
  vectors exactly as stored (confirmed against the stored arrays of the
  game's 5 such trees; how Unity uses them is a **hypothesis**). Leaf
  duration factors (`m_Duration`, 1 in 2,378 of 2,430 leaves, else 0.5,
  2, −1, ⅓, ⅔, 5, ¼, 0.1): length factor, negative plays backwards
  (**hypothesis**).
- **Transitions.** Checked in order: any-state transitions, then the
  current state's. A transition fires when all its conditions hold (If,
  IfNot on bools and triggers; Greater, Less, Equals on numbers) and its
  exit time is met. Exit time below 1: crossed by the fraction in this
  update, each loop; from 1 on: time at or past it (**hypothesis** for
  both). Duration in seconds (fixed) or × the source's length; the
  destination starts at the offset; the blend weight is linear in time.
  Triggers are cleared by the transition that uses them. Destinations of
  30,000 + n go through selector n (entry/exit nodes) to a state.
  Any-state transitions don't restart their own state when "can
  transition to self" is off.
- **Interruptions.** During a blend, any-state transitions and, by the
  active transition's interruption source, the source's (ordered: only
  those before it) and/or the destination's transitions may fire; the
  new blend starts from the layer's values frozen at that moment
  (**hypothesis**: that any-state transitions are checked during blends).
- **Layers**, in order over the defaults: the first always at weight 1;
  override: towards the layer's values by weight × mask; additive: adds
  each clip's change from its first frame (**hypothesis** for the
  reference pose). A layer acts on the slots some state of its machine
  animates; a skeleton mask leaves out Transforms not in it (other
  properties are never masked, **hypothesis**).
- **Write defaults.** A state with a motion and write defaults on puts
  the defaults on the layer's slots its clips leave out; off: they keep
  what is under them (the first layer: the last frame's value). **A
  state without a motion writes nothing**, so the layers below show
  through. Confirmed by the game's own design: the player's arm layers
  (weight 1) wait in the empty "Nothing" states while the base layer's
  idle moves the arms, and our first version (empty states writing
  defaults) left the player's arms frozen in the stored pose.
- **Cost:** the player's animator (9 layers, 213 slots) about 50 µs per
  update in a release build; the lifepod's (8 layers, 137 slots) about
  11 µs (`sn-inspect anim … --play`).

Not done: root motion (14 clips), animation events (555), mirroring (no
mirrored state or leaf in the game), IK (no humanoid), synchronised layers
(none in the game), direct and freeform cartesian blend trees (none in
the game).
