//! Animation classes, Unity 2019.4.36f1 layouts (from UnityPy's type
//! database; the files have no type trees): `AnimationClip` (74), `Avatar`
//! (90), `Animator` (95), `AnimatorController` (91). Every reader here must
//! reach the object's last byte, or it fails. See
//! `docs/formats/animation.md`.
//!
//! Only "generic" animation is kept in full: the game has no humanoid
//! avatar (their human parts are read past and must be empty or are
//! ignored).

use crate::objects::PPtr;
use crate::reader::Reader;
use crate::{ErrorKind, Result};

pub const ANIMATION_CLIP: i32 = 74;
pub const AVATAR: i32 = 90;
pub const ANIMATOR_CONTROLLER: i32 = 91;
pub const ANIMATOR: i32 = 95;

/// `GenericBinding.typeID` of a `Transform` (its `attribute` says which
/// property: [`ATTR_POSITION`] …).
pub const BIND_TRANSFORM: i32 = 4;
pub const ATTR_POSITION: u32 = 1;
pub const ATTR_ROTATION: u32 = 2;
pub const ATTR_SCALE: u32 = 3;
pub const ATTR_EULER: u32 = 4;

fn finish<T>(r: &Reader, data: &[u8], value: T) -> Result<T> {
    if r.pos() != data.len() {
        return Err(r.error(ErrorKind::Invalid(format!(
            "{} bytes left after the object",
            data.len() - r.pos()
        ))));
    }
    Ok(value)
}

fn vec_of<T>(
    r: &mut Reader,
    min: usize,
    mut f: impl FnMut(&mut Reader) -> Result<T>,
) -> Result<Vec<T>> {
    let n = r.count(min)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

fn f32s(r: &mut Reader) -> Result<Vec<f32>> {
    vec_of(r, 4, |r| r.f32())
}

fn u32s(r: &mut Reader) -> Result<Vec<u32>> {
    vec_of(r, 4, |r| r.u32())
}

fn i32s(r: &mut Reader) -> Result<Vec<i32>> {
    vec_of(r, 4, |r| r.i32())
}

fn floats<const N: usize>(r: &mut Reader) -> Result<[f32; N]> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = r.f32()?;
    }
    Ok(out)
}

fn bool_(r: &mut Reader) -> Result<bool> {
    Ok(r.u8()? != 0)
}

/// Unity's `xform`: translation, rotation (x, y, z, w), scale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Xform {
    pub t: [f32; 3],
    pub q: [f32; 4],
    pub s: [f32; 3],
}

fn xform(r: &mut Reader) -> Result<Xform> {
    Ok(Xform {
        t: floats(r)?,
        q: floats(r)?,
        s: floats(r)?,
    })
}

// ---------------------------------------------------------------- clips

/// One key of a streamed curve: from `time` until the curve's next key the
/// value is `((a·dt + b)·dt + c)·dt + d`, `dt = t − time`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamedKey {
    pub time: f32,
    pub coeff: [f32; 4],
}

/// `DenseClip`: `frame_count` frames of `curve_count` values each, sampled
/// at `sample_rate` from `begin_time`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DenseClip {
    pub frame_count: i32,
    pub curve_count: u32,
    pub sample_rate: f32,
    pub begin_time: f32,
    pub samples: Vec<f32>,
}

/// One property a clip animates (`GenericBinding`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenericBinding {
    /// CRC-32 of the object's path below the animator ("" for itself).
    pub path: u32,
    /// For a Transform: [`ATTR_POSITION`] …; for a blend shape weight
    /// (class 137, custom type 20) the CRC-32 of the channel's name; else
    /// CRC-32 of the property name (e.g. `m_Intensity`).
    pub attribute: u32,
    pub script: PPtr,
    /// Class id of the animated component (4 Transform, 137 skinned mesh
    /// renderer, 108 light, 114 script, …).
    pub type_id: i32,
    pub custom_type: u8,
    pub is_pptr_curve: bool,
}

impl GenericBinding {
    /// How many curves the property takes: 3 for a position, scale or
    /// Euler angles, 4 for a rotation, else 1.
    pub fn curve_count(&self) -> usize {
        if self.type_id == BIND_TRANSFORM && !self.is_pptr_curve {
            match self.attribute {
                ATTR_POSITION | ATTR_SCALE | ATTR_EULER => 3,
                ATTR_ROTATION => 4,
                _ => 1,
            }
        } else {
            1
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationEvent {
    pub time: f32,
    pub function: String,
    pub data: String,
    pub object: PPtr,
    pub float: f32,
    pub int: i32,
    pub message_options: i32,
}

/// `AnimationClip` (class 74).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationClip {
    pub name: String,
    pub legacy: bool,
    pub compressed: bool,
    /// Editor curves kept in the build (only legacy clips have them):
    /// rotation, compressed rotation, Euler, position, scale, float and
    /// object curves, counted, not kept.
    pub editor_curves: [usize; 7],
    pub sample_rate: f32,
    pub wrap_mode: i32,
    pub bounds: ([f32; 3], [f32; 3]),
    /// Streamed curves, by curve index (curves `0..streamed.len()`).
    pub streamed: Vec<Vec<StreamedKey>>,
    /// Dense curves (the next `dense.curve_count` indices).
    pub dense: DenseClip,
    /// Constant curves (the last indices), one value each.
    pub constant: Vec<f32>,
    pub start_time: f32,
    pub stop_time: f32,
    pub orientation_offset_y: f32,
    pub level: f32,
    pub cycle_offset: f32,
    pub average_angular_speed: f32,
    pub average_speed: [f32; 3],
    pub index_array: Vec<i32>,
    /// Per value: (start, stop).
    pub value_array_delta: Vec<(f32, f32)>,
    pub value_array_reference_pose: Vec<f32>,
    pub mirror: bool,
    pub loop_time: bool,
    pub loop_blend: bool,
    pub loop_blend_orientation: bool,
    pub loop_blend_position_y: bool,
    pub loop_blend_position_xz: bool,
    pub start_at_origin: bool,
    pub keep_original_orientation: bool,
    pub keep_original_position_y: bool,
    pub keep_original_position_xz: bool,
    pub height_from_feet: bool,
    pub bindings: Vec<GenericBinding>,
    pub pptr_curve_mapping: Vec<PPtr>,
    pub has_generic_root_transform: bool,
    pub has_motion_float_curves: bool,
    pub events: Vec<AnimationEvent>,
}

impl AnimationClip {
    /// Curves in all three blocks.
    pub fn curve_count(&self) -> usize {
        self.streamed.len() + self.dense.curve_count as usize + self.constant.len()
    }

    /// Curves the bindings take; equal to [`curve_count`](Self::curve_count)
    /// in every clip of the game.
    pub fn bound_curve_count(&self) -> usize {
        self.bindings.iter().map(|b| b.curve_count()).sum()
    }

    pub fn parse(data: &[u8], big_endian: bool) -> Result<AnimationClip> {
        let mut r = Reader::new(data, big_endian);
        let r = &mut r;
        let name = r.aligned_string()?;
        let legacy = bool_(r)?;
        let compressed = bool_(r)?;
        bool_(r)?; // use high quality curve
        r.align(4)?;
        let mut editor_curves = [0; 7];
        // Keyframe sizes: time, value/in/out slopes, weighted mode, in/out
        // weights.
        editor_curves[0] = skip_curves(r, 4 * (1 + 4 * 3 + 1 + 4 * 2), false)?;
        editor_curves[1] = vec_of(r, 4, skip_compressed_rotation)?.len();
        editor_curves[2] = skip_curves(r, 4 * (1 + 3 * 3 + 1 + 3 * 2), false)?;
        editor_curves[3] = skip_curves(r, 4 * (1 + 3 * 3 + 1 + 3 * 2), false)?;
        editor_curves[4] = skip_curves(r, 4 * (1 + 3 * 3 + 1 + 3 * 2), false)?;
        editor_curves[5] = skip_curves(r, 4 * 7, true)?;
        editor_curves[6] = vec_of(r, 4, skip_pptr_curve)?.len();
        let sample_rate = r.f32()?;
        let wrap_mode = r.i32()?;
        let bounds = (floats(r)?, floats(r)?);
        r.u32()?; // muscle clip size
        skip_human_pose(r)?;
        for _ in 0..4 {
            xform(r)?; // start, stop, left and right foot start
        }
        let average_speed = floats(r)?;
        // m_Clip
        let stream_at = r.pos();
        let words = u32s(r)?;
        let streamed_count = r.u32()?;
        let streamed = parse_streamed(&words, streamed_count, big_endian).map_err(|mut e| {
            e.offset += stream_at + 4;
            e
        })?;
        let dense = DenseClip {
            frame_count: r.i32()?,
            curve_count: r.u32()?,
            sample_rate: r.f32()?,
            begin_time: r.f32()?,
            samples: f32s(r)?,
        };
        let expected =
            (dense.frame_count.max(0) as usize).saturating_mul(dense.curve_count as usize);
        if dense.samples.len() != expected {
            return Err(r.error(ErrorKind::Invalid(format!(
                "dense clip: {} samples for {} frames of {} curves",
                dense.samples.len(),
                dense.frame_count,
                dense.curve_count
            ))));
        }
        let constant = f32s(r)?;
        let [
            start_time,
            stop_time,
            orientation_offset_y,
            level,
            cycle_offset,
            average_angular_speed,
        ] = floats(r)?;
        let index_array = i32s(r)?;
        let value_array_delta = vec_of(r, 8, |r| Ok((r.f32()?, r.f32()?)))?;
        let value_array_reference_pose = f32s(r)?;
        let mut flags = [false; 11];
        for f in &mut flags {
            *f = bool_(r)?;
        }
        r.align(4)?;
        let bindings = vec_of(r, 24, |r| {
            let path = r.u32()?;
            let attribute = r.u32()?;
            let script = PPtr::read(r)?;
            let type_id = r.i32()?;
            let custom_type = r.u8()?;
            let is_pptr_curve = bool_(r)?;
            r.align(4)?;
            Ok(GenericBinding {
                path,
                attribute,
                script,
                type_id,
                custom_type,
                is_pptr_curve,
            })
        })?;
        let pptr_curve_mapping = vec_of(r, 12, PPtr::read)?;
        let has_generic_root_transform = bool_(r)?;
        let has_motion_float_curves = bool_(r)?;
        r.align(4)?;
        let events = vec_of(r, 28, |r| {
            Ok(AnimationEvent {
                time: r.f32()?,
                function: r.aligned_string()?,
                data: r.aligned_string()?,
                object: PPtr::read(r)?,
                float: r.f32()?,
                int: r.i32()?,
                message_options: r.i32()?,
            })
        })?;
        let [
            mirror,
            loop_time,
            loop_blend,
            loop_blend_orientation,
            loop_blend_position_y,
            loop_blend_position_xz,
            start_at_origin,
            keep_original_orientation,
            keep_original_position_y,
            keep_original_position_xz,
            height_from_feet,
        ] = flags;
        let clip = AnimationClip {
            name,
            legacy,
            compressed,
            editor_curves,
            sample_rate,
            wrap_mode,
            bounds,
            streamed,
            dense,
            constant,
            start_time,
            stop_time,
            orientation_offset_y,
            level,
            cycle_offset,
            average_angular_speed,
            average_speed,
            index_array,
            value_array_delta,
            value_array_reference_pose,
            mirror,
            loop_time,
            loop_blend,
            loop_blend_orientation,
            loop_blend_position_y,
            loop_blend_position_xz,
            start_at_origin,
            keep_original_orientation,
            keep_original_position_y,
            keep_original_position_xz,
            height_from_feet,
            bindings,
            pptr_curve_mapping,
            has_generic_root_transform,
            has_motion_float_curves,
            events,
        };
        finish(r, data, clip)
    }
}

/// The streamed block: frames of `f32 time, u32 key count`, each key
/// `u32 curve, f32 a, b, c, d`. Keys go to their curve in time order.
/// Offsets in errors are relative to the block's first word.
fn parse_streamed(
    words: &[u32],
    curve_count: u32,
    big_endian: bool,
) -> Result<Vec<Vec<StreamedKey>>> {
    // Re-read the words as bytes in the file's order, so offsets are bytes.
    let bytes: Vec<u8> = words
        .iter()
        .flat_map(|w| {
            if big_endian {
                w.to_be_bytes()
            } else {
                w.to_le_bytes()
            }
        })
        .collect();
    let mut r = Reader::new(&bytes, big_endian);
    let mut curves = vec![Vec::new(); curve_count as usize];
    let mut last_time = f32::NEG_INFINITY;
    while r.pos() < bytes.len() {
        let time = r.f32()?;
        if time.is_nan() || time < last_time {
            return Err(r.error(ErrorKind::Invalid(format!(
                "streamed frame time {time} after {last_time}"
            ))));
        }
        last_time = time;
        let n = r.count(20)?;
        for _ in 0..n {
            let at = r.pos();
            let curve = r.u32()?;
            let coeff = floats(&mut r)?;
            let Some(keys) = curves.get_mut(curve as usize) else {
                return Err(crate::Error {
                    offset: at,
                    kind: ErrorKind::Invalid(format!(
                        "streamed key for curve {curve} of {curve_count}"
                    )),
                });
            };
            keys.push(StreamedKey { time, coeff });
        }
    }
    Ok(curves)
}

/// Reads past a vector of editor curves (`AnimationCurve` + path, and for
/// float curves attribute, class id and script); returns the count.
fn skip_curves(r: &mut Reader, key_bytes: usize, float_curve: bool) -> Result<usize> {
    let n = r.count(4)?;
    for _ in 0..n {
        let keys = r.count(key_bytes)?;
        r.bytes(keys * key_bytes)?;
        r.i32()?;
        r.i32()?;
        r.i32()?; // pre/post infinity, rotation order
        if float_curve {
            r.aligned_string()?; // attribute
        }
        r.aligned_string()?; // path
        if float_curve {
            r.i32()?; // class id
            PPtr::read(r)?; // script
        }
    }
    Ok(n)
}

fn skip_packed_bits(r: &mut Reader, range: bool, bit_size: bool) -> Result<()> {
    r.u32()?; // items
    if range {
        r.f32()?;
        r.f32()?;
    }
    let n = r.count(1)?;
    r.bytes(n)?;
    r.align(4)?;
    if bit_size {
        r.u8()?;
        r.align(4)?;
    }
    Ok(())
}

fn skip_compressed_rotation(r: &mut Reader) -> Result<()> {
    r.aligned_string()?;
    skip_packed_bits(r, false, true)?; // times
    skip_packed_bits(r, false, false)?; // values (packed quaternions)
    skip_packed_bits(r, true, true)?; // slopes
    r.i32()?;
    r.i32()?;
    Ok(())
}

fn skip_pptr_curve(r: &mut Reader) -> Result<()> {
    let n = r.count(16)?;
    r.bytes(n * 16)?;
    r.aligned_string()?; // attribute
    r.aligned_string()?; // path
    r.i32()?;
    PPtr::read(r)?;
    Ok(())
}

fn skip_hand_pose(r: &mut Reader) -> Result<()> {
    xform(r)?;
    f32s(r)?;
    floats::<4>(r)?;
    Ok(())
}

/// `HumanPose` (the clip's delta pose): empty in generic clips.
fn skip_human_pose(r: &mut Reader) -> Result<()> {
    xform(r)?; // root
    floats::<3>(r)?; // look-at position
    floats::<4>(r)?; // look-at weight
    let goals = r.count(64)?;
    for _ in 0..goals {
        xform(r)?;
        floats::<6>(r)?; // weight t, weight r, hint t, hint weight
    }
    skip_hand_pose(r)?;
    skip_hand_pose(r)?;
    f32s(r)?; // DoF
    let n = r.count(12)?; // TDoF (float3)
    r.bytes(n * 12)?;
    Ok(())
}

// ---------------------------------------------------------------- avatar

/// `Avatar` (class 90): its generic skeleton and name table. The human
/// part is read past.
#[derive(Clone, Debug, PartialEq)]
pub struct Avatar {
    pub name: String,
    /// Parent of each skeleton node (−1 for the root).
    pub skeleton_parents: Vec<i32>,
    /// Path hash of each skeleton node.
    pub skeleton_ids: Vec<u32>,
    pub skeleton_pose: Vec<Xform>,
    pub default_pose: Vec<Xform>,
    /// Nodes of the human skeleton (0 in every avatar of the game).
    pub human_nodes: usize,
    pub root_motion_bone: i32,
    /// Path hash → path, as `AnimatorController::names`.
    pub names: Vec<(u32, String)>,
}

fn skeleton(r: &mut Reader) -> Result<(Vec<i32>, Vec<u32>)> {
    let nodes = vec_of(r, 8, |r| {
        let parent = r.i32()?;
        r.i32()?; // axes id
        Ok(parent)
    })?;
    let ids = u32s(r)?;
    // Axes: pre and post rotation, sign, limit min and max, length, type.
    let axes = r.count(4 * 19)?;
    r.bytes(axes * 4 * 19)?;
    Ok((nodes, ids))
}

fn pose(r: &mut Reader) -> Result<Vec<Xform>> {
    vec_of(r, 40, xform)
}

impl Avatar {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Avatar> {
        let mut r = Reader::new(data, big_endian);
        let r = &mut r;
        let name = r.aligned_string()?;
        r.u32()?; // avatar size
        let (skeleton_parents, skeleton_ids) = skeleton(r)?;
        let skeleton_pose = pose(r)?;
        let default_pose = pose(r)?;
        u32s(r)?; // skeleton name ids
        // m_Human
        xform(r)?;
        let (human, _) = skeleton(r)?;
        pose(r)?;
        i32s(r)?;
        i32s(r)?; // hands
        i32s(r)?; // human bone index
        f32s(r)?; // human bone mass
        floats::<8>(r)?; // scale, twists, stretches, feet spacing
        for _ in 0..3 {
            bool_(r)?;
        }
        r.align(4)?;
        i32s(r)?;
        i32s(r)?; // human skeleton (reverse) index arrays
        let root_motion_bone = r.i32()?;
        xform(r)?;
        skeleton(r)?;
        pose(r)?;
        i32s(r)?;
        let names = names(r)?;
        // m_HumanDescription
        let bones = r.count(4)?;
        for _ in 0..bones {
            r.aligned_string()?;
            r.aligned_string()?;
            floats::<10>(r)?;
            bool_(r)?;
            r.align(4)?;
        }
        let skeleton_bones = r.count(4)?;
        for _ in 0..skeleton_bones {
            r.aligned_string()?;
            r.aligned_string()?;
            floats::<10>(r)?;
        }
        floats::<8>(r)?;
        r.aligned_string()?;
        for _ in 0..3 {
            bool_(r)?;
        }
        r.align(4)?;
        let avatar = Avatar {
            name,
            skeleton_parents,
            skeleton_ids,
            skeleton_pose,
            default_pose,
            human_nodes: human.len(),
            root_motion_bone,
            names,
        };
        finish(r, data, avatar)
    }
}

fn names(r: &mut Reader) -> Result<Vec<(u32, String)>> {
    vec_of(r, 8, |r| Ok((r.u32()?, r.aligned_string()?)))
}

// ---------------------------------------------------------------- animator

/// `Animator` (class 95).
#[derive(Clone, Debug, PartialEq)]
pub struct Animator {
    pub game_object: PPtr,
    pub enabled: bool,
    pub avatar: PPtr,
    pub controller: PPtr,
    /// 0 always animate, 1 cull update transforms, 2 cull completely.
    pub culling_mode: i32,
    /// 0 normal, 1 animate physics, 2 unscaled time.
    pub update_mode: i32,
    pub apply_root_motion: bool,
    pub linear_velocity_blending: bool,
    pub has_transform_hierarchy: bool,
    pub allow_constant_clip_sampling_optimization: bool,
    pub keep_state_on_disable: bool,
}

impl Animator {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Animator> {
        let mut r = Reader::new(data, big_endian);
        let r = &mut r;
        let game_object = PPtr::read(r)?;
        let enabled = bool_(r)?;
        r.align(4)?;
        let avatar = PPtr::read(r)?;
        let controller = PPtr::read(r)?;
        let culling_mode = r.i32()?;
        let update_mode = r.i32()?;
        let apply_root_motion = bool_(r)?;
        let linear_velocity_blending = bool_(r)?;
        r.align(4)?;
        let has_transform_hierarchy = bool_(r)?;
        let allow_constant_clip_sampling_optimization = bool_(r)?;
        let keep_state_on_disable = bool_(r)?;
        r.align(4)?;
        let a = Animator {
            game_object,
            enabled,
            avatar,
            controller,
            culling_mode,
            update_mode,
            apply_root_motion,
            linear_velocity_blending,
            has_transform_hierarchy,
            allow_constant_clip_sampling_optimization,
            keep_state_on_disable,
        };
        finish(r, data, a)
    }
}

// ---------------------------------------------------------------- controller

/// `AnimatorControllerParameterType` as stored in `m_Values`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Float,
    Int,
    Bool,
    Trigger,
    /// Positions, rotations, scales (internal values, not parameters).
    Other(u32),
}

impl ParamKind {
    pub fn from_u32(v: u32) -> ParamKind {
        match v {
            1 => ParamKind::Float,
            3 => ParamKind::Int,
            4 => ParamKind::Bool,
            9 => ParamKind::Trigger,
            other => ParamKind::Other(other),
        }
    }
}

/// One parameter (`ValueConstant`): its name hash, kind and index into the
/// default value array of its kind (triggers share the bools').
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Param {
    pub id: u32,
    pub kind: ParamKind,
    pub index: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefaultValues {
    pub positions: Vec<[f32; 3]>,
    pub quaternions: Vec<[f32; 4]>,
    pub scales: Vec<[f32; 3]>,
    pub floats: Vec<f32>,
    pub ints: Vec<i32>,
    pub bools: Vec<bool>,
}

/// `AnimatorConditionMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConditionMode {
    If,
    IfNot,
    Greater,
    Less,
    /// Exit time (not used as a condition in the game's data).
    ExitTime,
    Equals,
    NotEqual,
    Other(u32),
}

impl ConditionMode {
    pub fn from_u32(v: u32) -> ConditionMode {
        match v {
            1 => ConditionMode::If,
            2 => ConditionMode::IfNot,
            3 => ConditionMode::Greater,
            4 => ConditionMode::Less,
            5 => ConditionMode::ExitTime,
            6 => ConditionMode::Equals,
            7 => ConditionMode::NotEqual,
            other => ConditionMode::Other(other),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    pub mode: ConditionMode,
    /// The parameter's name hash.
    pub param: u32,
    pub threshold: f32,
    pub exit_time: f32,
}

/// `TransitionInterruptionSource`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interruption {
    None,
    Source,
    Destination,
    SourceThenDestination,
    DestinationThenSource,
    Other(i32),
}

impl Interruption {
    pub fn from_i32(v: i32) -> Interruption {
        match v {
            0 => Interruption::None,
            1 => Interruption::Source,
            2 => Interruption::Destination,
            3 => Interruption::SourceThenDestination,
            4 => Interruption::DestinationThenSource,
            other => Interruption::Other(other),
        }
    }
}

/// Destination indices at or above this are selector states (entry or
/// exit nodes): `SELECTOR_BASE + selector index`.
pub const SELECTOR_BASE: u32 = 30000;

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub conditions: Vec<Condition>,
    /// A state index, or [`SELECTOR_BASE`] + a selector index.
    pub destination: u32,
    pub full_path_id: u32,
    pub id: u32,
    pub user_id: u32,
    pub duration: f32,
    pub offset: f32,
    pub exit_time: f32,
    pub has_exit_time: bool,
    pub has_fixed_duration: bool,
    pub interruption: Interruption,
    pub ordered_interruption: bool,
    pub can_transition_to_self: bool,
}

/// `BlendTreeType` of a node with children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendType {
    Simple1D,
    SimpleDirectional2D,
    FreeformDirectional2D,
    FreeformCartesian2D,
    Direct,
    Other(u32),
}

impl BlendType {
    pub fn from_u32(v: u32) -> BlendType {
        match v {
            0 => BlendType::Simple1D,
            1 => BlendType::SimpleDirectional2D,
            2 => BlendType::FreeformDirectional2D,
            3 => BlendType::FreeformCartesian2D,
            4 => BlendType::Direct,
            other => BlendType::Other(other),
        }
    }
}

/// A blend tree node: a leaf plays clip `clip` (index into the
/// controller's clips; `u32::MAX` for none); a node with children blends
/// them by `kind`.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendNode {
    pub kind: BlendType,
    pub param: u32,
    pub param_y: u32,
    pub children: Vec<u32>,
    pub thresholds: Vec<f32>,
    pub positions: Vec<[f32; 2]>,
    pub magnitudes: Vec<f32>,
    pub pair_vectors: Vec<[f32; 2]>,
    pub pair_avg_mag_inv: Vec<f32>,
    pub neighbors: Vec<Vec<u32>>,
    pub direct_params: Vec<u32>,
    pub normalized_blend_values: bool,
    pub clip: u32,
    pub duration: f32,
    pub cycle_offset: f32,
    pub mirror: bool,
}

impl BlendNode {
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub transitions: Vec<Transition>,
    /// Blend tree per motion set (index into `blend_trees`).
    pub blend_tree_index: Vec<i32>,
    /// Each blend tree's nodes; node 0 is the root.
    pub blend_trees: Vec<Vec<BlendNode>>,
    pub name_id: u32,
    pub path_id: u32,
    pub full_path_id: u32,
    pub tag_id: u32,
    pub speed_param: u32,
    pub mirror_param: u32,
    pub cycle_offset_param: u32,
    pub time_param: u32,
    pub speed: f32,
    pub cycle_offset: f32,
    pub ik_on_feet: bool,
    pub write_default_values: bool,
    pub looping: bool,
    pub mirror: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectorTransition {
    pub destination: u32,
    pub conditions: Vec<Condition>,
}

/// An entry or exit node of a (sub-)state machine.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectorState {
    pub transitions: Vec<SelectorTransition>,
    pub full_path_id: u32,
    pub is_entry: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateMachine {
    pub states: Vec<State>,
    pub any_state_transitions: Vec<Transition>,
    pub selectors: Vec<SelectorState>,
    pub default_state: u32,
    pub synchronized_layer_count: u32,
}

/// `AnimatorLayerBlendingMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerBlending {
    Override,
    Additive,
    Other(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub state_machine: u32,
    pub synchronized_layer: u32,
    pub body_mask: [u32; 3],
    /// (path hash, weight); empty: no mask.
    pub skeleton_mask: Vec<(u32, f32)>,
    /// The layer's name hash.
    pub binding: u32,
    pub blending: LayerBlending,
    pub default_weight: f32,
    pub ik_pass: bool,
    pub synced_layer_affects_timing: bool,
}

/// `AnimatorController` (class 91).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorController {
    pub name: String,
    pub layers: Vec<Layer>,
    pub state_machines: Vec<StateMachine>,
    pub params: Vec<Param>,
    pub defaults: DefaultValues,
    /// Name hash → name (states, transitions, parameters, layers).
    pub names: Vec<(u32, String)>,
    pub clips: Vec<PPtr>,
    pub state_machine_behaviours: Vec<PPtr>,
    pub multi_threaded: bool,
}

fn condition(r: &mut Reader) -> Result<Condition> {
    Ok(Condition {
        mode: ConditionMode::from_u32(r.u32()?),
        param: r.u32()?,
        threshold: r.f32()?,
        exit_time: r.f32()?,
    })
}

fn transition(r: &mut Reader) -> Result<Transition> {
    let conditions = vec_of(r, 16, condition)?;
    let destination = r.u32()?;
    let full_path_id = r.u32()?;
    let id = r.u32()?;
    let user_id = r.u32()?;
    let duration = r.f32()?;
    let offset = r.f32()?;
    let exit_time = r.f32()?;
    let has_exit_time = bool_(r)?;
    let has_fixed_duration = bool_(r)?;
    r.align(4)?;
    let interruption = Interruption::from_i32(r.i32()?);
    let ordered_interruption = bool_(r)?;
    let can_transition_to_self = bool_(r)?;
    r.align(4)?;
    Ok(Transition {
        conditions,
        destination,
        full_path_id,
        id,
        user_id,
        duration,
        offset,
        exit_time,
        has_exit_time,
        has_fixed_duration,
        interruption,
        ordered_interruption,
        can_transition_to_self,
    })
}

fn vec2s(r: &mut Reader) -> Result<Vec<[f32; 2]>> {
    vec_of(r, 8, floats::<2>)
}

fn blend_node(r: &mut Reader) -> Result<BlendNode> {
    let kind = BlendType::from_u32(r.u32()?);
    let param = r.u32()?;
    let param_y = r.u32()?;
    let children = u32s(r)?;
    let thresholds = f32s(r)?;
    let positions = vec2s(r)?;
    let magnitudes = f32s(r)?;
    let pair_vectors = vec2s(r)?;
    let pair_avg_mag_inv = f32s(r)?;
    let neighbors = vec_of(r, 4, u32s)?;
    let direct_params = u32s(r)?;
    let normalized_blend_values = bool_(r)?;
    r.align(4)?;
    let clip = r.u32()?;
    let duration = r.f32()?;
    let cycle_offset = r.f32()?;
    let mirror = bool_(r)?;
    r.align(4)?;
    Ok(BlendNode {
        kind,
        param,
        param_y,
        children,
        thresholds,
        positions,
        magnitudes,
        pair_vectors,
        pair_avg_mag_inv,
        neighbors,
        direct_params,
        normalized_blend_values,
        clip,
        duration,
        cycle_offset,
        mirror,
    })
}

fn state(r: &mut Reader) -> Result<State> {
    let transitions = vec_of(r, 4, transition)?;
    let blend_tree_index = i32s(r)?;
    let blend_trees = vec_of(r, 4, |r| vec_of(r, 4, blend_node))?;
    let mut ids = [0u32; 8];
    for v in &mut ids {
        *v = r.u32()?;
    }
    let speed = r.f32()?;
    let cycle_offset = r.f32()?;
    let ik_on_feet = bool_(r)?;
    let write_default_values = bool_(r)?;
    let looping = bool_(r)?;
    let mirror = bool_(r)?;
    r.align(4)?;
    let [
        name_id,
        path_id,
        full_path_id,
        tag_id,
        speed_param,
        mirror_param,
        cycle_offset_param,
        time_param,
    ] = ids;
    Ok(State {
        transitions,
        blend_tree_index,
        blend_trees,
        name_id,
        path_id,
        full_path_id,
        tag_id,
        speed_param,
        mirror_param,
        cycle_offset_param,
        time_param,
        speed,
        cycle_offset,
        ik_on_feet,
        write_default_values,
        looping,
        mirror,
    })
}

fn state_machine(r: &mut Reader) -> Result<StateMachine> {
    let states = vec_of(r, 4, state)?;
    let any_state_transitions = vec_of(r, 4, transition)?;
    let selectors = vec_of(r, 4, |r| {
        let transitions = vec_of(r, 8, |r| {
            Ok(SelectorTransition {
                destination: r.u32()?,
                conditions: vec_of(r, 16, condition)?,
            })
        })?;
        let full_path_id = r.u32()?;
        let is_entry = bool_(r)?;
        r.align(4)?;
        Ok(SelectorState {
            transitions,
            full_path_id,
            is_entry,
        })
    })?;
    Ok(StateMachine {
        states,
        any_state_transitions,
        selectors,
        default_state: r.u32()?,
        synchronized_layer_count: r.u32()?,
    })
}

fn layer(r: &mut Reader) -> Result<Layer> {
    let state_machine = r.u32()?;
    let synchronized_layer = r.u32()?;
    let body_mask = [r.u32()?, r.u32()?, r.u32()?];
    let skeleton_mask = vec_of(r, 8, |r| Ok((r.u32()?, r.f32()?)))?;
    let binding = r.u32()?;
    let blending = match r.i32()? {
        0 => LayerBlending::Override,
        1 => LayerBlending::Additive,
        other => LayerBlending::Other(other),
    };
    let default_weight = r.f32()?;
    let ik_pass = bool_(r)?;
    let synced_layer_affects_timing = bool_(r)?;
    r.align(4)?;
    Ok(Layer {
        state_machine,
        synchronized_layer,
        body_mask,
        skeleton_mask,
        binding,
        blending,
        default_weight,
        ik_pass,
        synced_layer_affects_timing,
    })
}

impl AnimatorController {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<AnimatorController> {
        let mut r = Reader::new(data, big_endian);
        let r = &mut r;
        let name = r.aligned_string()?;
        r.u32()?; // controller size
        let layers = vec_of(r, 4, layer)?;
        let state_machines = vec_of(r, 4, state_machine)?;
        let params = vec_of(r, 12, |r| {
            Ok(Param {
                id: r.u32()?,
                kind: ParamKind::from_u32(r.u32()?),
                index: r.u32()?,
            })
        })?;
        let defaults = DefaultValues {
            positions: vec_of(r, 12, floats::<3>)?,
            quaternions: vec_of(r, 16, floats::<4>)?,
            scales: vec_of(r, 12, floats::<3>)?,
            floats: f32s(r)?,
            ints: i32s(r)?,
            bools: {
                let n = r.count(1)?;
                let b = r.bytes(n)?.iter().map(|&b| b != 0).collect();
                r.align(4)?;
                b
            },
        };
        let names = names(r)?;
        let clips = vec_of(r, 12, PPtr::read)?;
        // State machine behaviour ranges: (state id, layer) → (start, count).
        let ranges = r.count(16)?;
        r.bytes(ranges * 16)?;
        u32s(r)?;
        let state_machine_behaviours = vec_of(r, 12, PPtr::read)?;
        let multi_threaded = bool_(r)?;
        r.align(4)?;
        let c = AnimatorController {
            name,
            layers,
            state_machines,
            params,
            defaults,
            names,
            clips,
            state_machine_behaviours,
            multi_threaded,
        };
        finish(r, data, c)
    }

    /// The name a hash stands for, if the controller's table has it.
    pub fn name_of(&self, hash: u32) -> Option<&str> {
        self.names
            .iter()
            .find(|(h, _)| *h == hash)
            .map(|(_, n)| n.as_str())
    }
}

/// `Animator.StringToHash` and the path hashes of bindings: CRC-32
/// (IEEE 802.3, the zlib one) of the UTF-8 bytes.
pub fn name_hash(name: &str) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in name.as_bytes() {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian writer with Unity's 4-byte alignment.
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        fn u32(&mut self, v: u32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn i32(&mut self, v: i32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn f32(&mut self, v: f32) -> &mut Self {
            self.0.extend(v.to_le_bytes());
            self
        }
        fn f32n(&mut self, v: &[f32]) -> &mut Self {
            for &x in v {
                self.f32(x);
            }
            self
        }
        fn b(&mut self, v: bool) -> &mut Self {
            self.0.push(v as u8);
            self
        }
        fn align(&mut self) -> &mut Self {
            while self.0.len() % 4 != 0 {
                self.0.push(0);
            }
            self
        }
        fn str(&mut self, s: &str) -> &mut Self {
            self.i32(s.len() as i32);
            self.0.extend(s.as_bytes());
            self.align()
        }
        fn pptr(&mut self, p: PPtr) -> &mut Self {
            self.i32(p.file_id);
            self.0.extend(p.path_id.to_le_bytes());
            self
        }
        fn f32v(&mut self, v: &[f32]) -> &mut Self {
            self.i32(v.len() as i32).f32n(v)
        }
        fn u32v(&mut self, v: &[u32]) -> &mut Self {
            self.i32(v.len() as i32);
            for &x in v {
                self.u32(x);
            }
            self
        }
        fn i32v(&mut self, v: &[i32]) -> &mut Self {
            self.i32(v.len() as i32);
            for &x in v {
                self.i32(x);
            }
            self
        }
        fn xform(&mut self, x: &Xform) -> &mut Self {
            self.f32n(&x.t).f32n(&x.q).f32n(&x.s)
        }
    }

    fn encode_clip(c: &AnimationClip) -> Vec<u8> {
        let mut w = W::default();
        w.str(&c.name).b(c.legacy).b(c.compressed).b(false).align();
        for _ in 0..7 {
            w.i32(0); // no editor curves
        }
        w.f32(c.sample_rate)
            .i32(c.wrap_mode)
            .f32n(&c.bounds.0)
            .f32n(&c.bounds.1);
        w.u32(0);
        // Empty human pose: root, look-at, goals, two hands, DoF, TDoF.
        w.xform(&Xform::default()).f32n(&[0.0; 7]).i32(0);
        for _ in 0..2 {
            w.xform(&Xform::default()).i32(0).f32n(&[0.0; 4]);
        }
        w.i32(0).i32(0);
        for _ in 0..4 {
            w.xform(&Xform::default());
        }
        w.f32n(&c.average_speed);
        // Streamed: frames grouped by time over all curves.
        let mut keys: Vec<(f32, u32, [f32; 4])> = c
            .streamed
            .iter()
            .enumerate()
            .flat_map(|(i, ks)| ks.iter().map(move |k| (k.time, i as u32, k.coeff)))
            .collect();
        keys.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut words = W::default();
        let mut i = 0;
        while i < keys.len() {
            let t = keys[i].0;
            let n = keys[i..].iter().take_while(|k| k.0 == t).count();
            words.f32(t).u32(n as u32);
            for k in &keys[i..i + n] {
                words.u32(k.1).f32n(&k.2);
            }
            i += n;
        }
        w.i32((words.0.len() / 4) as i32);
        w.0.extend(&words.0);
        w.u32(c.streamed.len() as u32);
        w.i32(c.dense.frame_count)
            .u32(c.dense.curve_count)
            .f32(c.dense.sample_rate)
            .f32(c.dense.begin_time)
            .f32v(&c.dense.samples);
        w.f32v(&c.constant);
        w.f32n(&[
            c.start_time,
            c.stop_time,
            c.orientation_offset_y,
            c.level,
            c.cycle_offset,
            c.average_angular_speed,
        ]);
        w.i32v(&c.index_array);
        w.i32(c.value_array_delta.len() as i32);
        for &(a, b) in &c.value_array_delta {
            w.f32(a).f32(b);
        }
        w.f32v(&c.value_array_reference_pose);
        for f in [
            c.mirror,
            c.loop_time,
            c.loop_blend,
            c.loop_blend_orientation,
            c.loop_blend_position_y,
            c.loop_blend_position_xz,
            c.start_at_origin,
            c.keep_original_orientation,
            c.keep_original_position_y,
            c.keep_original_position_xz,
            c.height_from_feet,
        ] {
            w.b(f);
        }
        w.align();
        w.i32(c.bindings.len() as i32);
        for b in &c.bindings {
            let at = w.0.len();
            w.u32(b.path)
                .u32(b.attribute)
                .pptr(b.script)
                .i32(b.type_id)
                .b(false)
                .b(b.is_pptr_curve)
                .align();
            w.0[at + 24] = b.custom_type;
        }
        w.i32(c.pptr_curve_mapping.len() as i32);
        for p in &c.pptr_curve_mapping {
            w.pptr(*p);
        }
        w.b(c.has_generic_root_transform)
            .b(c.has_motion_float_curves)
            .align();
        w.i32(c.events.len() as i32);
        for e in &c.events {
            w.f32(e.time)
                .str(&e.function)
                .str(&e.data)
                .pptr(e.object)
                .f32(e.float)
                .i32(e.int)
                .i32(e.message_options);
        }
        w.0
    }

    fn sample_clip() -> AnimationClip {
        let key = |time, d| StreamedKey {
            time,
            coeff: [0.0, 0.0, 1.0, d],
        };
        AnimationClip {
            name: "swim".into(),
            legacy: false,
            compressed: false,
            editor_curves: [0; 7],
            sample_rate: 30.0,
            wrap_mode: 0,
            bounds: ([0.0; 3], [1.0; 3]),
            streamed: vec![
                vec![key(f32::MIN, 0.0), key(0.0, 0.0), key(1.0, 1.0)],
                vec![key(f32::MIN, 2.0), key(0.5, 2.0)],
                vec![key(f32::MIN, 0.0)],
            ],
            dense: DenseClip {
                frame_count: 2,
                curve_count: 1,
                sample_rate: 30.0,
                begin_time: 0.0,
                samples: vec![0.25, 0.75],
            },
            constant: vec![1.0, 1.0, 1.0],
            start_time: 0.0,
            stop_time: 1.0,
            orientation_offset_y: 0.0,
            level: 0.0,
            cycle_offset: 0.25,
            average_angular_speed: 0.0,
            average_speed: [0.0; 3],
            index_array: vec![0, -1],
            value_array_delta: vec![(0.0, 1.0)],
            value_array_reference_pose: vec![],
            mirror: false,
            loop_time: true,
            loop_blend: false,
            loop_blend_orientation: false,
            loop_blend_position_y: false,
            loop_blend_position_xz: false,
            start_at_origin: false,
            keep_original_orientation: false,
            keep_original_position_y: false,
            keep_original_position_xz: false,
            height_from_feet: false,
            bindings: vec![
                GenericBinding {
                    path: name_hash("root/fin"),
                    attribute: ATTR_ROTATION,
                    script: PPtr::default(),
                    type_id: BIND_TRANSFORM,
                    custom_type: 0,
                    is_pptr_curve: false,
                },
                GenericBinding {
                    path: name_hash("root"),
                    attribute: ATTR_SCALE,
                    script: PPtr::default(),
                    type_id: BIND_TRANSFORM,
                    custom_type: 0,
                    is_pptr_curve: false,
                },
            ],
            pptr_curve_mapping: vec![],
            has_generic_root_transform: false,
            has_motion_float_curves: false,
            events: vec![AnimationEvent {
                time: 0.5,
                function: "OnStep".into(),
                data: String::new(),
                object: PPtr::default(),
                float: 0.0,
                int: 3,
                message_options: 0,
            }],
        }
    }

    #[test]
    fn clip_round_trip() {
        let c = sample_clip();
        let bytes = encode_clip(&c);
        let back = AnimationClip::parse(&bytes, false).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.curve_count(), 7);
        assert_eq!(back.bound_curve_count(), 7);
        for len in 0..bytes.len() {
            assert!(AnimationClip::parse(&bytes[..len], false).is_err(), "{len}");
        }
        // A trailing byte is an error too.
        let mut longer = bytes.clone();
        longer.extend([0; 4]);
        assert!(AnimationClip::parse(&longer, false).is_err());
    }

    #[test]
    fn streamed_keys_must_name_a_curve_and_go_forward() {
        let words = |v: &[u32]| v.to_vec();
        let t = |x: f32| x.to_bits();
        // One frame at t = 0 with a key for curve 0, one at t = 1 for curve 1.
        let ok = words(&[t(0.0), 1, 0, 0, 0, 0, t(5.0), t(1.0), 1, 1, 0, 0, 0, t(6.0)]);
        let curves = parse_streamed(&ok, 2, false).unwrap();
        assert_eq!(curves[0][0].coeff[3], 5.0);
        assert_eq!(curves[1][0].time, 1.0);
        // Curve 1 of 1: out of range, at the key's byte offset.
        let e = parse_streamed(&ok, 1, false).unwrap_err();
        assert_eq!(e.offset, 36);
        // Time going back.
        let back = words(&[t(1.0), 0, t(0.0), 0]);
        assert!(parse_streamed(&back, 1, false).is_err());
        // Cut inside a key.
        assert!(parse_streamed(&ok[..9], 2, false).is_err());
    }

    #[test]
    fn animator_layout() {
        let mut w = W::default();
        w.pptr(PPtr {
            file_id: 0,
            path_id: 7,
        })
        .b(true)
        .align();
        w.pptr(PPtr {
            file_id: 1,
            path_id: 8,
        })
        .pptr(PPtr {
            file_id: 2,
            path_id: 9,
        });
        w.i32(1)
            .i32(0)
            .b(false)
            .b(false)
            .align()
            .b(true)
            .b(true)
            .b(false)
            .align();
        let a = Animator::parse(&w.0, false).unwrap();
        assert!(a.enabled);
        assert_eq!(a.avatar.path_id, 8);
        assert_eq!(a.controller.path_id, 9);
        assert_eq!(a.culling_mode, 1);
        assert!(a.has_transform_hierarchy);
        for len in 0..w.0.len() {
            assert!(Animator::parse(&w.0[..len], false).is_err());
        }
    }

    fn encode_condition(w: &mut W, c: &Condition) {
        let mode = match c.mode {
            ConditionMode::If => 1,
            ConditionMode::IfNot => 2,
            ConditionMode::Greater => 3,
            ConditionMode::Less => 4,
            ConditionMode::ExitTime => 5,
            ConditionMode::Equals => 6,
            ConditionMode::NotEqual => 7,
            ConditionMode::Other(v) => v,
        };
        w.u32(mode).u32(c.param).f32(c.threshold).f32(c.exit_time);
    }

    fn encode_transition(w: &mut W, t: &Transition) {
        w.i32(t.conditions.len() as i32);
        for c in &t.conditions {
            encode_condition(w, c);
        }
        w.u32(t.destination)
            .u32(t.full_path_id)
            .u32(t.id)
            .u32(t.user_id)
            .f32(t.duration)
            .f32(t.offset)
            .f32(t.exit_time)
            .b(t.has_exit_time)
            .b(t.has_fixed_duration)
            .align();
        let i = match t.interruption {
            Interruption::None => 0,
            Interruption::Source => 1,
            Interruption::Destination => 2,
            Interruption::SourceThenDestination => 3,
            Interruption::DestinationThenSource => 4,
            Interruption::Other(v) => v,
        };
        w.i32(i)
            .b(t.ordered_interruption)
            .b(t.can_transition_to_self)
            .align();
    }

    fn encode_node(w: &mut W, n: &BlendNode) {
        let kind = match n.kind {
            BlendType::Simple1D => 0,
            BlendType::SimpleDirectional2D => 1,
            BlendType::FreeformDirectional2D => 2,
            BlendType::FreeformCartesian2D => 3,
            BlendType::Direct => 4,
            BlendType::Other(v) => v,
        };
        w.u32(kind).u32(n.param).u32(n.param_y).u32v(&n.children);
        w.f32v(&n.thresholds);
        w.i32(n.positions.len() as i32);
        for p in &n.positions {
            w.f32n(p);
        }
        w.f32v(&n.magnitudes);
        w.i32(n.pair_vectors.len() as i32);
        for p in &n.pair_vectors {
            w.f32n(p);
        }
        w.f32v(&n.pair_avg_mag_inv);
        w.i32(n.neighbors.len() as i32);
        for l in &n.neighbors {
            w.u32v(l);
        }
        w.u32v(&n.direct_params)
            .b(n.normalized_blend_values)
            .align();
        w.u32(n.clip)
            .f32(n.duration)
            .f32(n.cycle_offset)
            .b(n.mirror)
            .align();
    }

    fn encode_controller(c: &AnimatorController) -> Vec<u8> {
        let mut w = W::default();
        w.str(&c.name).u32(0);
        w.i32(c.layers.len() as i32);
        for l in &c.layers {
            w.u32(l.state_machine).u32(l.synchronized_layer);
            for m in l.body_mask {
                w.u32(m);
            }
            w.i32(l.skeleton_mask.len() as i32);
            for &(h, wt) in &l.skeleton_mask {
                w.u32(h).f32(wt);
            }
            let blending = match l.blending {
                LayerBlending::Override => 0,
                LayerBlending::Additive => 1,
                LayerBlending::Other(v) => v,
            };
            w.u32(l.binding)
                .i32(blending)
                .f32(l.default_weight)
                .b(l.ik_pass)
                .b(l.synced_layer_affects_timing)
                .align();
        }
        w.i32(c.state_machines.len() as i32);
        for sm in &c.state_machines {
            w.i32(sm.states.len() as i32);
            for s in &sm.states {
                w.i32(s.transitions.len() as i32);
                for t in &s.transitions {
                    encode_transition(&mut w, t);
                }
                w.i32v(&s.blend_tree_index);
                w.i32(s.blend_trees.len() as i32);
                for tree in &s.blend_trees {
                    w.i32(tree.len() as i32);
                    for n in tree {
                        encode_node(&mut w, n);
                    }
                }
                for id in [
                    s.name_id,
                    s.path_id,
                    s.full_path_id,
                    s.tag_id,
                    s.speed_param,
                    s.mirror_param,
                    s.cycle_offset_param,
                    s.time_param,
                ] {
                    w.u32(id);
                }
                w.f32(s.speed)
                    .f32(s.cycle_offset)
                    .b(s.ik_on_feet)
                    .b(s.write_default_values)
                    .b(s.looping)
                    .b(s.mirror)
                    .align();
            }
            w.i32(sm.any_state_transitions.len() as i32);
            for t in &sm.any_state_transitions {
                encode_transition(&mut w, t);
            }
            w.i32(sm.selectors.len() as i32);
            for sel in &sm.selectors {
                w.i32(sel.transitions.len() as i32);
                for t in &sel.transitions {
                    w.u32(t.destination).i32(t.conditions.len() as i32);
                    for c in &t.conditions {
                        encode_condition(&mut w, c);
                    }
                }
                w.u32(sel.full_path_id).b(sel.is_entry).align();
            }
            w.u32(sm.default_state).u32(sm.synchronized_layer_count);
        }
        w.i32(c.params.len() as i32);
        for p in &c.params {
            let kind = match p.kind {
                ParamKind::Float => 1,
                ParamKind::Int => 3,
                ParamKind::Bool => 4,
                ParamKind::Trigger => 9,
                ParamKind::Other(v) => v,
            };
            w.u32(p.id).u32(kind).u32(p.index);
        }
        let d = &c.defaults;
        w.i32(d.positions.len() as i32);
        d.positions.iter().for_each(|v| {
            w.f32n(v);
        });
        w.i32(d.quaternions.len() as i32);
        d.quaternions.iter().for_each(|v| {
            w.f32n(v);
        });
        w.i32(d.scales.len() as i32);
        d.scales.iter().for_each(|v| {
            w.f32n(v);
        });
        w.f32v(&d.floats).i32v(&d.ints);
        w.i32(d.bools.len() as i32);
        for &b in &d.bools {
            w.b(b);
        }
        w.align();
        w.i32(c.names.len() as i32);
        for (h, n) in &c.names {
            w.u32(*h).str(n);
        }
        w.i32(c.clips.len() as i32);
        for p in &c.clips {
            w.pptr(*p);
        }
        w.i32(0).i32(0); // behaviour ranges and indices
        w.i32(c.state_machine_behaviours.len() as i32);
        for p in &c.state_machine_behaviours {
            w.pptr(*p);
        }
        w.b(c.multi_threaded).align();
        w.0
    }

    fn leaf(clip: u32) -> BlendNode {
        BlendNode {
            kind: BlendType::Simple1D,
            param: u32::MAX,
            param_y: u32::MAX,
            children: vec![],
            thresholds: vec![],
            positions: vec![],
            magnitudes: vec![],
            pair_vectors: vec![],
            pair_avg_mag_inv: vec![],
            neighbors: vec![],
            direct_params: vec![],
            normalized_blend_values: false,
            clip,
            duration: 1.0,
            cycle_offset: 0.0,
            mirror: false,
        }
    }

    fn sample_controller() -> AnimatorController {
        let speed = name_hash("speed");
        let transition = Transition {
            conditions: vec![Condition {
                mode: ConditionMode::Greater,
                param: speed,
                threshold: 0.5,
                exit_time: 0.0,
            }],
            destination: 1,
            full_path_id: name_hash("Base.idle -> Base.swim"),
            id: 1,
            user_id: 0,
            duration: 0.25,
            offset: 0.0,
            exit_time: 0.9,
            has_exit_time: false,
            has_fixed_duration: true,
            interruption: Interruption::Destination,
            ordered_interruption: true,
            can_transition_to_self: false,
        };
        let state = |name: &str, transitions: Vec<Transition>, tree: Vec<BlendNode>| State {
            transitions,
            blend_tree_index: vec![0],
            blend_trees: vec![tree],
            name_id: name_hash(name),
            path_id: name_hash(&format!("Base.{name}")),
            full_path_id: name_hash(&format!("Base.{name}")),
            tag_id: 0,
            speed_param: 0,
            mirror_param: 0,
            cycle_offset_param: 0,
            time_param: 0,
            speed: 1.0,
            cycle_offset: 0.0,
            ik_on_feet: false,
            write_default_values: true,
            looping: false,
            mirror: false,
        };
        let mut blend = leaf(u32::MAX);
        blend.children = vec![1, 2];
        blend.param = speed;
        blend.thresholds = vec![0.0, 1.0];
        AnimatorController {
            name: "fish".into(),
            layers: vec![Layer {
                state_machine: 0,
                synchronized_layer: 0,
                body_mask: [u32::MAX; 3],
                skeleton_mask: vec![(name_hash("root"), 1.0)],
                binding: name_hash("Base"),
                blending: LayerBlending::Additive,
                default_weight: 1.0,
                ik_pass: false,
                synced_layer_affects_timing: false,
            }],
            state_machines: vec![StateMachine {
                states: vec![
                    state("idle", vec![transition], vec![leaf(0)]),
                    state("swim", vec![], vec![blend, leaf(0), leaf(1)]),
                ],
                any_state_transitions: vec![],
                selectors: vec![SelectorState {
                    transitions: vec![SelectorTransition {
                        destination: 0,
                        conditions: vec![],
                    }],
                    full_path_id: name_hash("Base"),
                    is_entry: true,
                }],
                default_state: 0,
                synchronized_layer_count: 1,
            }],
            params: vec![Param {
                id: speed,
                kind: ParamKind::Float,
                index: 0,
            }],
            defaults: DefaultValues {
                floats: vec![0.0],
                bools: vec![true],
                ..Default::default()
            },
            names: vec![(speed, "speed".into())],
            clips: vec![
                PPtr {
                    file_id: 0,
                    path_id: 3,
                },
                PPtr {
                    file_id: 0,
                    path_id: 4,
                },
            ],
            state_machine_behaviours: vec![],
            multi_threaded: true,
        }
    }

    #[test]
    fn controller_round_trip() {
        let c = sample_controller();
        let bytes = encode_controller(&c);
        let back = AnimatorController::parse(&bytes, false).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.name_of(name_hash("speed")), Some("speed"));
        for len in 0..bytes.len() {
            assert!(
                AnimatorController::parse(&bytes[..len], false).is_err(),
                "{len}"
            );
        }
    }

    #[test]
    fn name_hash_is_crc32() {
        // Known CRC-32 values (zlib's crc32).
        assert_eq!(name_hash(""), 0);
        assert_eq!(name_hash("123456789"), 0xCBF4_3926);
    }
}
