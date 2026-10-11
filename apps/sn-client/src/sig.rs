//! Surface inputs for the two opaque world-object SIG shaders.
//! Rules confirmed from the installed deferred programs; see materials.md.

use sn_unity::Material;

use crate::objects::ShaderInfo;

pub const PLAIN: &str = "UWE/SIG";
pub const CAPPED: &str = "UWE/SIG Triplanar with Capping";

#[derive(Clone, Debug)]
pub struct SigLook {
    pub capped: bool,
    pub albedo: Option<u32>,
    pub normal: Option<u32>,
    pub map: Option<u32>,
    pub map_st: [f32; 4],
    pub enabled: bool,
    pub side: Option<u32>,
    pub side_normal: Option<u32>,
    pub specular: [f32; 4],
    pub color: [f32; 4],
    pub cap_color: [f32; 4],
    pub cap_specular: [f32; 4],
    pub border_tint: [f32; 4],
    pub scales: [f32; 4],
    pub cap_border: [f32; 4],
    pub inner_border: [f32; 4],
    pub gloss: f32,
    pub emission: f32,
}

pub fn read(
    material: &Material,
    shader: &ShaderInfo,
    mut texture: impl FnMut(&str, Option<bool>) -> Option<(u32, [f32; 4])>,
) -> Option<SigLook> {
    let capped = match shader.name.as_str() {
        PLAIN => false,
        CAPPED => true,
        _ => return None,
    };
    let value = |name: &str| {
        material
            .float(name)
            .or_else(|| shader.defaults.get(name).map(|v| v[0]))
            .unwrap_or(0.0)
    };
    let color = |name: &str| {
        material
            .color(name)
            .or_else(|| shader.defaults.get(name).copied())
            .unwrap_or([1.0; 4])
    };
    // Capping declares SIG slots but its compiled programs never sample them.
    let enabled = !capped && material.keywords.split_whitespace().any(|k| k == "UWE_SIG");
    let map = enabled.then(|| texture("_SIGMap", None)).flatten();
    Some(SigLook {
        capped,
        albedo: texture(if capped { "_CapTexture" } else { "_MainTex" }, Some(true)).map(|t| t.0),
        normal: texture(if capped { "_CapBumpMap" } else { "_BumpMap" }, Some(false)).map(|t| t.0),
        map: map.map(|t| t.0),
        map_st: map.map_or([1.0, 1.0, 0.0, 0.0], |t| t.1),
        enabled,
        side: capped
            .then(|| texture("_SideTexture", Some(true)))
            .flatten()
            .map(|t| t.0),
        side_normal: capped
            .then(|| texture("_SideBumpMap", Some(false)))
            .flatten()
            .map(|t| t.0),
        specular: color("_SpecColor"),
        color: color("_Color"),
        cap_color: color("_CapColor"),
        cap_specular: color("_CapSpecColor"),
        border_tint: color("_BorderTint"),
        scales: [
            value("_CapScale"),
            value("_SideScale"),
            value("_TriplanarBlendRange"),
            0.0,
        ],
        cap_border: [
            value("_CapBorderBlendRange"),
            value("_CapBorderBlendOffset"),
            value("_CapBorderBlendAngle"),
            0.0,
        ],
        inner_border: [
            value("_InnerBorderBlendRange"),
            value("_InnerBorderBlendOffset"),
            0.0,
            0.0,
        ],
        gloss: value("_Gloss"),
        emission: value("_EmissionScale"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material() -> Material {
        Material {
            name: "synthetic".into(),
            shader: sn_unity::PPtr {
                file_id: 0,
                path_id: 0,
            },
            keywords: String::new(),
            custom_render_queue: -1,
            tags: vec![],
            textures: vec![],
            floats: vec![],
            colors: vec![],
        }
    }

    fn shader(name: &str) -> ShaderInfo {
        ShaderInfo {
            name: name.into(),
            defaults: [
                ("_Gloss".into(), [0.37, 0.0, 0.0, 0.0]),
                ("_EmissionScale".into(), [1.7, 0.0, 0.0, 0.0]),
                ("_CapColor".into(), [0.2, 0.3, 0.4, 0.5]),
            ]
            .into(),
        }
    }

    #[test]
    fn exact_shader_selection_and_keyword_gate() {
        let mut m = material();
        let mut calls = vec![];
        let mut sample = |name: &str, _: Option<bool>| {
            calls.push(name.to_owned());
            Some((7, [2.0, 3.0, 0.25, 0.75]))
        };
        assert!(read(&m, &shader("UWE/SIG Terrain Grass"), &mut sample).is_none());
        let plain = read(&m, &shader(PLAIN), &mut sample).unwrap();
        assert!(!plain.enabled);
        assert_eq!(plain.map, None);
        assert!(!calls.iter().any(|s| s == "_SIGMap"));
        m.keywords = "UNRELATED UWE_SIG".into();
        let plain = read(&m, &shader(PLAIN), |_, _| {
            Some((17, [2.0, 3.0, 0.25, 0.75]))
        })
        .unwrap();
        assert!(plain.enabled);
        assert_eq!(plain.map, Some(17));
        assert_eq!(plain.map_st, [2.0, 3.0, 0.25, 0.75]);
        assert_eq!(plain.emission, 1.7);
    }

    #[test]
    fn capped_slots_defaults_and_overrides() {
        let mut m = material();
        m.keywords = "UWE_SIG".into();
        m.floats.push(("_Gloss".into(), 0.62));
        let mut calls = vec![];
        let look = read(&m, &shader(CAPPED), |name, srgb| {
            calls.push((name.to_owned(), srgb));
            Some((calls.len() as u32, [1.0, 1.0, 0.0, 0.0]))
        })
        .unwrap();
        assert!(look.capped);
        assert!(!look.enabled);
        assert_eq!(look.gloss, 0.62);
        assert_eq!(look.cap_color, [0.2, 0.3, 0.4, 0.5]);
        assert_eq!(
            calls,
            vec![
                ("_CapTexture".into(), Some(true)),
                ("_CapBumpMap".into(), Some(false)),
                ("_SideTexture".into(), Some(true)),
                ("_SideBumpMap".into(), Some(false)),
            ]
        );
        assert_eq!(look.albedo, Some(1));
        assert_eq!(look.side, Some(3));
        assert_eq!(look.map, None);
    }

    #[test]
    fn enabled_missing_sig_uses_shader_default_instead_of_disabling_variant() {
        let mut m = material();
        m.keywords = "UWE_SIG".into();
        let look = read(&m, &shader(PLAIN), |_, _| None).unwrap();
        assert!(look.enabled);
        assert_eq!(look.map, None);
        assert_eq!(look.map_st, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(look.gloss, 0.37);
    }

    #[test]
    fn global_sky_relighting_does_not_enable_marmoset_on_sig() {
        for kind in [1.0, 2.0] {
            let mut params = crate::object_look::ObjectParams {
                sig_settings: bevy::prelude::Vec4::new(kind, 0.3, 0.6, 1.0),
                ..Default::default()
            };
            crate::objects::apply_sky(&mut params, None);
            assert_eq!(params.sky_flags, bevy::prelude::Vec4::ZERO);
            assert_eq!(params.sig_settings.x, kind);
        }
    }
}
