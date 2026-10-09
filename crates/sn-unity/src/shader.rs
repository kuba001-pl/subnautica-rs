//! `Shader` (class 48), Unity 2019.4: the parsed form (`m_ParsedForm`)
//! up to its name and dependencies: properties, sub-shaders and their
//! passes with the render state (blend, depth, cull, colour mask). The
//! compiled programs' bindings are walked over, not kept; the program
//! blobs after the parsed form are not read. Layout:
//! `docs/formats/unity.md` § Shaders.

use crate::Result;
use crate::reader::Reader;

/// A render-state value: fixed (`property` empty) or taken from the
/// material's float `property` (e.g. `Blend [_SrcBlend] [_DstBlend]`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderValue {
    pub value: f32,
    pub property: String,
}

/// One render target's blending (`SerializedShaderRTBlendState`). Values
/// are Unity's `BlendMode` (0 Zero, 1 One, 5 SrcAlpha, 10 OneMinusSrcAlpha,
/// …), `BlendOp` and `ColorWriteMask` (15 = RGBA, 0 = none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendState {
    pub src: ShaderValue,
    pub dst: ShaderValue,
    pub src_alpha: ShaderValue,
    pub dst_alpha: ShaderValue,
    pub op: ShaderValue,
    pub op_alpha: ShaderValue,
    pub color_mask: ShaderValue,
}

/// A pass's fixed-function state (`SerializedShaderState`), the parts we
/// use. `z_test` is Unity's `CompareFunction`, `cull` its `CullMode`
/// (0 off, 1 front, 2 back).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PassState {
    pub name: String,
    /// Render targets 0–7.
    pub blend: Vec<BlendState>,
    pub separate_blend: bool,
    pub z_test: ShaderValue,
    pub z_write: ShaderValue,
    pub cull: ShaderValue,
    pub offset_factor: ShaderValue,
    pub offset_units: ShaderValue,
    pub tags: Vec<(String, String)>,
    pub lod: i32,
    pub lighting: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderPass {
    /// 0 a normal pass, 1 `UsePass`, 2 `GrabPass`.
    pub kind: i32,
    pub state: PassState,
    /// Bit per program stage present (vertex, fragment, …).
    pub program_mask: u32,
    /// Sub-programs (variants) per stage: vertex, fragment, geometry,
    /// hull, domain, ray tracing.
    pub sub_programs: [usize; 6],
    pub use_name: String,
    pub name: String,
    pub texture_name: String,
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubShader {
    pub passes: Vec<ShaderPass>,
    pub tags: Vec<(String, String)>,
    pub lod: i32,
}

/// A declared property (`SerializedProperty`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderProperty {
    pub name: String,
    pub description: String,
    pub attributes: Vec<String>,
    /// 0 colour, 1 vector, 2 float, 3 range, 4 texture.
    pub kind: i32,
    pub flags: u32,
    pub default: [f32; 4],
    /// For textures: `white`, `black`, `bump`, `gray`, …
    pub default_texture: String,
    pub texture_dim: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shader {
    /// The shader's name as written in its source, e.g. `UWE/Particles/UBER`
    /// (`m_ParsedForm.m_Name`).
    pub name: String,
    pub properties: Vec<ShaderProperty>,
    pub sub_shaders: Vec<SubShader>,
    pub custom_editor: String,
    pub fallback: String,
}

impl Shader {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Shader> {
        let mut r = Reader::new(data, big_endian);
        let _object_name = r.aligned_string()?;
        let n = r.count(40)?;
        let mut properties = Vec::with_capacity(n);
        for _ in 0..n {
            properties.push(property(&mut r)?);
        }
        r.align(4)?;
        let n = r.count(12)?;
        let mut sub_shaders = Vec::with_capacity(n);
        for _ in 0..n {
            sub_shaders.push(sub_shader(&mut r)?);
        }
        r.align(4)?;
        let name = r.aligned_string()?;
        let custom_editor = r.aligned_string()?;
        let fallback = r.aligned_string()?;
        Ok(Shader {
            name,
            properties,
            sub_shaders,
            custom_editor,
            fallback,
        })
    }

    /// The first sub-shader's passes: the ones Unity uses on a desktop GPU
    /// that runs any of them (later sub-shaders are fallbacks).
    pub fn passes(&self) -> &[ShaderPass] {
        self.sub_shaders.first().map_or(&[], |s| &s.passes)
    }
}

fn strings(r: &mut Reader) -> Result<Vec<String>> {
    let n = r.count(4)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(r.aligned_string()?);
    }
    r.align(4)?;
    Ok(out)
}

/// `map<string, string>`: maps are not aligned after their array.
fn tag_map(r: &mut Reader) -> Result<Vec<(String, String)>> {
    let n = r.count(8)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push((r.aligned_string()?, r.aligned_string()?));
    }
    Ok(out)
}

fn property(r: &mut Reader) -> Result<ShaderProperty> {
    Ok(ShaderProperty {
        name: r.aligned_string()?,
        description: r.aligned_string()?,
        attributes: strings(r)?,
        kind: r.i32()?,
        flags: r.u32()?,
        default: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
        default_texture: r.aligned_string()?,
        texture_dim: r.i32()?,
    })
}

fn sub_shader(r: &mut Reader) -> Result<SubShader> {
    let n = r.count(100)?;
    let mut passes = Vec::with_capacity(n);
    for _ in 0..n {
        passes.push(pass(r)?);
    }
    r.align(4)?;
    Ok(SubShader {
        passes,
        tags: tag_map(r)?,
        lod: r.i32()?,
    })
}

/// Unity stores `<noninit>` as the property of a fixed value.
const NO_PROPERTY: &str = "<noninit>";

fn value(r: &mut Reader) -> Result<ShaderValue> {
    let value = r.f32()?;
    let mut property = r.aligned_string()?;
    if property == NO_PROPERTY {
        property.clear();
    }
    Ok(ShaderValue { value, property })
}

fn blend(r: &mut Reader) -> Result<BlendState> {
    Ok(BlendState {
        src: value(r)?,
        dst: value(r)?,
        src_alpha: value(r)?,
        dst_alpha: value(r)?,
        op: value(r)?,
        op_alpha: value(r)?,
        color_mask: value(r)?,
    })
}

fn state(r: &mut Reader) -> Result<PassState> {
    let name = r.aligned_string()?;
    let mut blends = Vec::with_capacity(8);
    for _ in 0..8 {
        blends.push(blend(r)?);
    }
    let separate_blend = r.u8()? != 0;
    r.align(4)?;
    let _z_clip = value(r)?;
    let z_test = value(r)?;
    let z_write = value(r)?;
    let cull = value(r)?;
    let offset_factor = value(r)?;
    let offset_units = value(r)?;
    let _alpha_to_mask = value(r)?;
    // Stencil: three ops of four values, read mask, write mask, ref.
    for _ in 0..15 {
        value(r)?;
    }
    // Fog start, end, density, colour (four values and a name).
    for _ in 0..7 {
        value(r)?;
    }
    let _fog_colour_name = r.aligned_string()?;
    let _fog_mode = r.i32()?;
    let _gpu_program_id = r.i32()?;
    let tags = tag_map(r)?;
    let lod = r.i32()?;
    let lighting = r.u8()? != 0;
    r.align(4)?;
    Ok(PassState {
        name,
        blend: blends,
        separate_blend,
        z_test,
        z_write,
        cull,
        offset_factor,
        offset_units,
        tags,
        lod,
        lighting,
    })
}

/// A vector of fixed-size items, skipped.
fn skip_items(r: &mut Reader, item_bytes: usize) -> Result<()> {
    let n = r.count(item_bytes)?;
    r.bytes(n * item_bytes)?;
    r.align(4)
}

/// A `SerializedProgram`: its sub-programs' bindings walked over; the
/// number of sub-programs returned.
fn program(r: &mut Reader) -> Result<usize> {
    let n = r.count(60)?;
    for _ in 0..n {
        let _blob_index = r.u32()?;
        skip_items(r, 2)?; // bind channels (source, target)
        let _source_map = r.i32()?;
        skip_items(r, 2)?; // global keyword indices
        skip_items(r, 2)?; // local keyword indices
        let _tier = r.u8()?;
        let _gpu_program_type = r.u8()?;
        r.align(4)?;
        skip_items(r, 16)?; // vector parameters
        skip_items(r, 16)?; // matrix parameters
        skip_items(r, 16)?; // texture parameters
        skip_items(r, 8)?; // buffer parameters
        let buffers = r.count(20)?;
        for _ in 0..buffers {
            let _name = r.i32()?;
            skip_items(r, 16)?; // matrices
            skip_items(r, 16)?; // vectors
            let structs = r.count(24)?;
            for _ in 0..structs {
                r.bytes(16)?; // name, index, array size, struct size
                skip_items(r, 16)?; // vector members
                skip_items(r, 16)?; // matrix members
            }
            r.align(4)?;
            let _size = r.i32()?;
        }
        r.align(4)?;
        skip_items(r, 8)?; // constant buffer bindings
        skip_items(r, 12)?; // UAV parameters
        skip_items(r, 8)?; // samplers
        let _requirements = r.i32()?;
    }
    r.align(4)?;
    Ok(n)
}

fn pass(r: &mut Reader) -> Result<ShaderPass> {
    // Name indices: map<string, int>, not kept.
    let n = r.count(8)?;
    for _ in 0..n {
        r.aligned_string()?;
        r.i32()?;
    }
    let kind = r.i32()?;
    let state = state(r)?;
    let program_mask = r.u32()?;
    let mut sub_programs = [0; 6];
    for s in &mut sub_programs {
        *s = program(r)?;
    }
    let _instancing = r.u8()?;
    let _procedural_instancing = r.u8()?;
    r.align(4)?;
    Ok(ShaderPass {
        kind,
        state,
        program_mask,
        sub_programs,
        use_name: r.aligned_string()?,
        name: r.aligned_string()?,
        texture_name: r.aligned_string()?,
        tags: tag_map(r)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes Unity 2019.4 serialized values (little-endian).
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        fn i32(&mut self, v: i32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn f32(&mut self, v: f32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn align(&mut self) {
            while self.0.len() % 4 != 0 {
                self.0.push(0);
            }
        }
        fn str(&mut self, s: &str) {
            self.i32(s.len() as i32);
            self.0.extend_from_slice(s.as_bytes());
            self.align();
        }
        fn value(&mut self, v: f32, property: &str) {
            self.f32(v);
            self.str(property);
        }
        fn tags(&mut self, tags: &[(&str, &str)]) {
            self.i32(tags.len() as i32);
            for (k, v) in tags {
                self.str(k);
                self.str(v);
            }
        }
        /// A program with one sub-program holding one item of every kind.
        fn program(&mut self, sub_programs: i32) {
            self.i32(sub_programs);
            for _ in 0..sub_programs {
                self.i32(7); // blob index
                self.i32(1);
                self.0.extend_from_slice(&[0, 3]);
                self.align();
                self.i32(0); // source map
                self.i32(3);
                self.0.extend_from_slice(&[1, 0, 2, 0, 3, 0]);
                self.align();
                self.i32(0);
                self.0.extend_from_slice(&[0, 4]);
                self.align();
                for _ in 0..3 {
                    self.i32(1);
                    self.0.extend_from_slice(&[9; 16]);
                }
                self.i32(1);
                self.0.extend_from_slice(&[9; 8]);
                self.i32(1); // one constant buffer
                self.i32(2);
                self.i32(0);
                self.i32(1);
                self.0.extend_from_slice(&[9; 16]);
                self.i32(1); // one struct
                self.0.extend_from_slice(&[9; 16]);
                self.i32(0);
                self.i32(1);
                self.0.extend_from_slice(&[9; 16]);
                self.i32(64);
                self.i32(0);
                self.i32(0);
                self.i32(1);
                self.0.extend_from_slice(&[9; 8]);
                self.i32(0); // requirements
            }
        }
    }

    fn shader_bytes() -> Vec<u8> {
        let mut w = W::default();
        w.str("");
        // One texture property.
        w.i32(1);
        w.str("_MainTex");
        w.str("Base (RGB)");
        w.i32(1);
        w.str("NoScaleOffset");
        w.i32(4);
        w.i32(0);
        for v in [0.0, 0.0, 0.0, 0.0] {
            w.f32(v);
        }
        w.str("white");
        w.i32(2);
        // One sub-shader with one pass.
        w.i32(1);
        w.i32(1);
        w.i32(1); // name indices
        w.str("_Color");
        w.i32(3);
        w.i32(0); // normal pass
        w.str("FORWARD");
        for rt in 0..8 {
            let (src, dst) = if rt == 0 { (5.0, 1.0) } else { (1.0, 0.0) };
            w.value(src, if rt == 0 { "_SrcBlend" } else { "" });
            w.value(dst, "<noninit>");
            w.value(1.0, "");
            w.value(0.0, "");
            w.value(0.0, "");
            w.value(0.0, "");
            w.value(15.0, "");
        }
        w.0.push(0);
        w.align();
        w.value(1.0, ""); // z clip
        w.value(4.0, ""); // z test
        w.value(0.0, "_ZWrite");
        w.value(2.0, "");
        w.value(0.0, "");
        w.value(0.0, "");
        w.value(0.0, "");
        for _ in 0..22 {
            w.value(0.0, "");
        }
        w.str("");
        w.i32(0);
        w.i32(0);
        w.tags(&[("LightMode", "ForwardBase")]);
        w.i32(0);
        w.0.push(1);
        w.align();
        w.i32(3); // program mask
        w.program(2);
        w.program(1);
        for _ in 0..4 {
            w.program(0);
        }
        w.0.extend_from_slice(&[0, 0]);
        w.align();
        w.str("");
        w.str("FORWARD");
        w.str("");
        w.tags(&[]);
        w.tags(&[("Queue", "Transparent+101")]);
        w.i32(100);
        w.str("UWE/Test Shader");
        w.str("");
        w.str("Diffuse");
        // Dependencies and the program blobs follow (not read).
        w.i32(0);
        w.0
    }

    #[test]
    fn reads_the_parsed_form() {
        let s = Shader::parse(&shader_bytes(), false).unwrap();
        assert_eq!(s.name, "UWE/Test Shader");
        assert_eq!(s.fallback, "Diffuse");
        assert_eq!(s.properties.len(), 1);
        let p = &s.properties[0];
        assert_eq!((p.name.as_str(), p.kind), ("_MainTex", 4));
        assert_eq!(p.attributes, ["NoScaleOffset"]);
        assert_eq!((p.default_texture.as_str(), p.texture_dim), ("white", 2));
        assert_eq!(s.sub_shaders.len(), 1);
        assert_eq!(
            s.sub_shaders[0].tags,
            [("Queue".into(), "Transparent+101".into())]
        );
        assert_eq!(s.sub_shaders[0].lod, 100);
        let pass = &s.passes()[0];
        assert_eq!(pass.name, "FORWARD");
        assert_eq!(pass.sub_programs, [2, 1, 0, 0, 0, 0]);
        assert_eq!(pass.program_mask, 3);
        let st = &pass.state;
        assert_eq!(st.blend.len(), 8);
        assert_eq!(st.blend[0].src.value, 5.0);
        assert_eq!(st.blend[0].src.property, "_SrcBlend");
        assert_eq!(st.blend[0].dst.value, 1.0);
        assert_eq!(st.blend[0].dst.property, "");
        assert_eq!(st.blend[0].color_mask.value, 15.0);
        assert_eq!(st.blend[1].src.value, 1.0);
        assert_eq!(
            (st.z_test.value, st.z_write.property.as_str()),
            (4.0, "_ZWrite")
        );
        assert_eq!(st.cull.value, 2.0);
        assert_eq!(st.tags, [("LightMode".into(), "ForwardBase".into())]);
        assert!(st.lighting);
    }

    #[test]
    fn truncated_shaders_are_errors() {
        let b = shader_bytes();
        for len in [0, 3, 40, 200, b.len() / 2, b.len() - 20] {
            assert!(Shader::parse(&b[..len], false).is_err(), "length {len}");
        }
    }
}
