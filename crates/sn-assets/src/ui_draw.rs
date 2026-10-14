//! A UI tree's draw list with its materials and text vertices (M9u4d,
//! M9u5a; `docs/formats/ui.md` § Draw order, § UI shaders and materials):
//! the layout, the images' and raw images' meshes, the texts laid out and
//! made into vertices with their glyphs made into the font atlases, every
//! graphic's material, then `sn-ui`'s draw list. `sn-inspect ui --draw`
//! checks it; the client draws it (M9u5c).
//!
//! Materials (from the decompiled `UnityEngine.UI.dll` and TMP):
//! - an `Image` or `RawImage` draws with `m_Material`, or the canvas's
//!   default material (`UI/Default`) without one;
//! - a text draws with `m_sharedMaterial` when that material's `_MainTex`
//!   is its font asset's atlas, else with the font asset's material
//!   (`TextMeshProUGUI.LoadFontAsset`); the padding is that material's
//!   (`GetPaddingForMaterial`, which also sets its scale ratios);
//! - characters from a fallback font draw in a sub-mesh with the
//!   fallback material (`TMP_MaterialManager.GetFallbackMaterial`).
//!
//! Glyphs are made the first time a text needs them, in tree order (the
//! game's order depends on which texts it shows first, so atlas positions
//! may differ from the game's; the UVs follow ours).

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use sn_ui::draw::DrawList;
use sn_ui::font::{Added, DynamicAtlas};
use sn_ui::material::UiMaterial;
use sn_ui::scaler::{Placement, ScalerSettings};
use sn_ui::text::{AtlasGlyph, FontSet};
use sn_unity::Value;

use crate::{Assets, FontAssets, Layouts, ObjectRef, Result, UiStartState, UiState, UiTree};

/// A texture's key in draw items and materials: a hash of its object.
pub fn texture_key(o: &ObjectRef) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    o.key().hash(&mut h);
    // Keep clear of the font atlases' keys.
    h.finish() & !(1 << 40)
}

/// The key of font asset `font`'s dynamic atlas (ours, not the stored
/// texture).
pub fn font_texture_key(font: usize) -> u64 {
    1 << 40 | font as u64
}

/// Every font asset's dynamic atlas, filled as texts need glyphs.
pub struct GlyphAtlases {
    pub atlases: Vec<DynamicAtlas>,
    /// Glyphs made, glyphs that found no room (multi-atlas is not
    /// ported), and the time spent making them.
    pub made: usize,
    pub full: usize,
    pub seconds: f64,
}

impl GlyphAtlases {
    /// Empty atlases of each asset's stored size, padding and sampling
    /// size.
    pub fn new(fonts: &FontAssets) -> Result<GlyphAtlases> {
        let mut atlases = Vec::with_capacity(fonts.assets.len());
        for a in &fonts.assets {
            let s = a.stored()?;
            atlases.push(DynamicAtlas::new(
                s.atlas_width,
                s.atlas_height,
                s.padding,
                s.point_size,
            ));
        }
        Ok(GlyphAtlases {
            atlases,
            made: 0,
            full: 0,
            seconds: 0.0,
        })
    }

    pub(crate) fn glyph(&mut self, set: &FontSet, font: usize, glyph: u32) -> Option<AtlasGlyph> {
        let atlas = self.atlases.get_mut(font)?;
        let file = &set.fonts.get(font)?.file;
        let start = std::time::Instant::now();
        let added = atlas.add_glyph(file, glyph);
        let g = match added {
            Added::Cached(g) => g,
            Added::New(g) => {
                self.made += 1;
                self.seconds += start.elapsed().as_secs_f64();
                g
            }
            Added::Full => {
                self.full += 1;
                return None;
            }
            Added::Missing => return None,
        };
        Some(AtlasGlyph {
            rect: g.rect,
            atlas_width: atlas.width,
            atlas_height: atlas.height,
        })
    }
}

/// A tree's draw list and what it draws with.
pub struct UiDraw {
    pub list: DrawList,
    /// Indexed by the items' `material`; 0 is `UI/Default`.
    pub materials: Vec<UiMaterial>,
    /// Sorting layer ids not in the TagManager.
    pub unknown_layers: Vec<i32>,
    /// What the text meshes met but do not port, with the text's path.
    pub unported: Vec<String>,
    /// Texts drawn, and their sub-meshes (fallback fonts).
    pub texts: usize,
    pub sub_meshes: usize,
    /// The image textures by key (font atlases are in [`GlyphAtlases`]).
    pub textures: HashMap<u64, ObjectRef>,
}

/// Materials by object, interned.
pub(crate) struct MaterialTable {
    pub(crate) list: Vec<UiMaterial>,
    pub(crate) by_key: HashMap<(std::path::PathBuf, String, i64), usize>,
    by_value: Vec<(UiMaterial, usize)>,
}

impl MaterialTable {
    pub(crate) fn new() -> MaterialTable {
        MaterialTable {
            list: vec![UiMaterial::ui_default()],
            by_key: HashMap::new(),
            by_value: Vec::new(),
        }
    }

    /// A derived material (a text's, after its ratios are set; a fallback):
    /// equal ones share an index.
    pub(crate) fn intern(&mut self, m: UiMaterial) -> usize {
        if let Some((_, i)) = self.by_value.iter().find(|(v, _)| *v == m) {
            return *i;
        }
        self.list.push(m.clone());
        let i = self.list.len() - 1;
        self.by_value.push((m, i));
        i
    }
}

impl Assets<'_> {
    /// A material as the UI draws it. Textures get [`texture_key`], except
    /// a font asset's atlas, which gets [`font_texture_key`].
    pub fn ui_material(&self, m: &ObjectRef, fonts: &FontAssets) -> Result<UiMaterial> {
        let (_, data) = m.data()?;
        let mat = sn_unity::Material::parse(data, m.file.file().big_endian)
            .map_err(|e| format!("Material: {e}"))?;
        let shader = match self.resolve(&m.file, mat.shader)? {
            Some(s) => {
                let (_, d) = s.data()?;
                sn_unity::Shader::parse(d, s.file.file().big_endian)
                    .map_err(|e| format!("Shader: {e}"))?
                    .name
            }
            None => format!("(external shader {})", mat.shader.path_id),
        };
        let mut out = UiMaterial {
            name: mat.name.clone(),
            shader,
            keywords: mat
                .keywords
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            floats: mat.floats.iter().cloned().collect::<BTreeMap<_, _>>(),
            colors: mat.colors.iter().cloned().collect::<BTreeMap<_, _>>(),
            textures: BTreeMap::new(),
        };
        for t in &mat.textures {
            let key = match self.resolve(&m.file, t.texture)? {
                Some(o) => fonts
                    .assets
                    .iter()
                    .position(|a| a.atlas_textures.iter().any(|x| x.key() == o.key()))
                    .map_or_else(|| texture_key(&o), font_texture_key),
                None => 0,
            };
            out.textures.insert(t.name.clone(), key);
            out.colors.insert(
                format!("{}_ST", t.name),
                [t.scale[0], t.scale[1], t.offset[0], t.offset[1]],
            );
        }
        Ok(out)
    }

    /// Font asset `font`'s own material (`TMP_Asset.material`).
    pub(crate) fn font_material(
        &self,
        fonts: &FontAssets,
        font: usize,
    ) -> Result<Option<UiMaterial>> {
        let Some(a) = fonts.assets.get(font) else {
            return Ok(None);
        };
        let Some(p) = a
            .fields
            .get("material")
            .and_then(Value::as_pptr)
            .filter(|p| !p.is_null())
        else {
            return Ok(None);
        };
        match self.resolve(&a.object.file, p)? {
            Some(o) => Ok(Some(self.ui_material(&o, fonts)?)),
            None => Ok(None),
        }
    }

    /// Every enabled `uGUI_CanvasScaler` of a tree, by node (M9u5b).
    pub fn ui_scalers(&self, tree: &UiTree) -> Result<Vec<(usize, ScalerSettings)>> {
        let mut out = Vec::new();
        for (node, c) in tree.scripts("uGUI_CanvasScaler") {
            let (_, d) = c.object.data()?;
            let s = sn_unity::CanvasScaler::parse(d, c.object.file.file().big_endian)
                .map_err(|e| format!("{}: uGUI_CanvasScaler: {e}", tree.nodes[node].path))?;
            if !s.enabled {
                continue;
            }
            out.push((
                node,
                ScalerSettings {
                    reference_resolution: s.reference_resolution,
                    mode: s.mode,
                    distance: s.distance,
                    scale_mode: s.scale_mode,
                },
            ));
        }
        Ok(out)
    }

    /// The draw list of `tree` with materials and text vertices, its
    /// canvases at their stored sizes; glyphs go into `atlases`.
    /// `sorting_layers` is the TagManager's list.
    pub fn ui_draw(
        &self,
        layouts: &Layouts,
        tree_in: &UiTree,
        fonts: &FontAssets,
        set: &FontSet,
        atlases: &mut GlyphAtlases,
        sorting_layers: &[(String, u32)],
    ) -> Result<UiDraw> {
        self.ui_draw_placed(
            layouts,
            tree_in,
            fonts,
            set,
            atlases,
            sorting_layers,
            &HashMap::new(),
            &UiStartState::default(),
        )
    }

    /// [`Assets::ui_draw`] with canvases sized and scaled as their scalers
    /// place them (`placements` by node, from `sn_ui::scaler::place`)
    /// before the layout, and the scripts' first-frame changes (`start`,
    /// from [`Assets::ui_start_state`]; empty for the stored state).
    #[allow(clippy::too_many_arguments)]
    pub fn ui_draw_placed(
        &self,
        layouts: &Layouts,
        tree_in: &UiTree,
        fonts: &FontAssets,
        set: &FontSet,
        atlases: &mut GlyphAtlases,
        sorting_layers: &[(String, u32)],
        placements: &HashMap<usize, Placement>,
        start: &UiStartState,
    ) -> Result<UiDraw> {
        self.ui_scene(layouts, tree_in, fonts, sorting_layers)?
            .draw(self, fonts, set, atlases, placements, &UiState::from(start))
    }
}
