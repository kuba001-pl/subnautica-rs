//! The project's physics and time settings in `globalgamemanagers`
//! (built-in classes, Unity 2019.4 layouts). The file has no type trees, so
//! the layouts are ours, checked by requiring the data to end with the last
//! field. See `docs/formats/gameplay.md` § Player movement.

use crate::objects::PPtr;
use crate::reader::Reader;
use crate::{ErrorKind, Result};

pub const TIME_MANAGER: i32 = 5;
pub const PHYSICS_MANAGER: i32 = 55;
pub const TAG_MANAGER: i32 = 78;
pub const RIGIDBODY: i32 = 54;

/// `Rigidbody` (class 54).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigidbody {
    pub game_object: PPtr,
    pub mass: f32,
    pub drag: f32,
    pub angular_drag: f32,
    pub use_gravity: bool,
    pub is_kinematic: bool,
    pub interpolate: u8,
    pub constraints: i32,
    pub collision_detection: i32,
}

impl Rigidbody {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Rigidbody> {
        let mut r = Reader::new(data, big_endian);
        let game_object = PPtr::read(&mut r)?;
        let mass = r.f32()?;
        let drag = r.f32()?;
        let angular_drag = r.f32()?;
        let use_gravity = r.u8()? != 0;
        let is_kinematic = r.u8()? != 0;
        let interpolate = r.u8()?;
        r.align(4)?;
        let b = Rigidbody {
            game_object,
            mass,
            drag,
            angular_drag,
            use_gravity,
            is_kinematic,
            interpolate,
            constraints: r.i32()?,
            collision_detection: r.i32()?,
        };
        at_end(&r, data)?;
        Ok(b)
    }
}

fn at_end(r: &Reader, data: &[u8]) -> Result<()> {
    if r.pos() != data.len() {
        return Err(r.error(ErrorKind::Invalid(format!(
            "{} bytes after the last field",
            data.len() - r.pos()
        ))));
    }
    Ok(())
}

/// `TimeManager` (class 5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeManager {
    /// `Time.fixedDeltaTime`, seconds.
    pub fixed_timestep: f32,
    pub maximum_allowed_timestep: f32,
    pub time_scale: f32,
    pub maximum_particle_timestep: f32,
}

impl TimeManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<TimeManager> {
        let mut r = Reader::new(data, big_endian);
        let t = TimeManager {
            fixed_timestep: r.f32()?,
            maximum_allowed_timestep: r.f32()?,
            time_scale: r.f32()?,
            maximum_particle_timestep: r.f32()?,
        };
        at_end(&r, data)?;
        Ok(t)
    }
}

/// `PhysicsManager` (class 55).
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsManager {
    pub gravity: [f32; 3],
    pub default_material: PPtr,
    pub bounce_threshold: f32,
    pub sleep_threshold: f32,
    pub default_contact_offset: f32,
    pub default_solver_iterations: i32,
    pub default_solver_velocity_iterations: i32,
    /// Raycasts and sweeps hit the back of mesh triangles.
    pub queries_hit_backfaces: bool,
    pub queries_hit_triggers: bool,
    pub enable_adaptive_force: bool,
    pub cloth_inter_collision_distance: f32,
    pub cloth_inter_collision_stiffness: f32,
    pub contacts_generation: i32,
    /// Bit `j` of row `i`: layers `i` and `j` collide.
    pub layer_collision_matrix: Vec<u32>,
    pub auto_simulation: bool,
    pub auto_sync_transforms: bool,
    pub reuse_collision_callbacks: bool,
    pub cloth_inter_collision_settings_toggle: bool,
    pub cloth_gravity: [f32; 3],
    pub contact_pairs_mode: i32,
    pub broadphase_type: i32,
    /// Centre and extent.
    pub world_bounds: [[f32; 3]; 2],
    pub world_subdivisions: i32,
    pub friction_type: i32,
    pub enable_enhanced_determinism: bool,
    pub enable_unified_heightmaps: bool,
    pub solver_type: i32,
    pub default_max_angular_speed: f32,
}

fn v3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

impl PhysicsManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PhysicsManager> {
        let mut r = Reader::new(data, big_endian);
        let gravity = v3(&mut r)?;
        let default_material = PPtr::read(&mut r)?;
        let bounce_threshold = r.f32()?;
        let sleep_threshold = r.f32()?;
        let default_contact_offset = r.f32()?;
        let default_solver_iterations = r.i32()?;
        let default_solver_velocity_iterations = r.i32()?;
        let queries_hit_backfaces = r.u8()? != 0;
        let queries_hit_triggers = r.u8()? != 0;
        let enable_adaptive_force = r.u8()? != 0;
        r.align(4)?;
        let cloth_inter_collision_distance = r.f32()?;
        let cloth_inter_collision_stiffness = r.f32()?;
        let contacts_generation = r.i32()?;
        let n = r.count(4)?;
        if n != 32 {
            return Err(r.error(ErrorKind::Invalid(format!(
                "layer matrix has {n} rows, expected 32"
            ))));
        }
        let mut layer_collision_matrix = Vec::with_capacity(n);
        for _ in 0..n {
            layer_collision_matrix.push(r.u32()?);
        }
        let auto_simulation = r.u8()? != 0;
        let auto_sync_transforms = r.u8()? != 0;
        let reuse_collision_callbacks = r.u8()? != 0;
        let cloth_inter_collision_settings_toggle = r.u8()? != 0;
        r.align(4)?;
        let cloth_gravity = v3(&mut r)?;
        let contact_pairs_mode = r.i32()?;
        let broadphase_type = r.i32()?;
        let world_bounds = [v3(&mut r)?, v3(&mut r)?];
        let world_subdivisions = r.i32()?;
        let friction_type = r.i32()?;
        let enable_enhanced_determinism = r.u8()? != 0;
        let enable_unified_heightmaps = r.u8()? != 0;
        r.align(4)?;
        let solver_type = r.i32()?;
        let default_max_angular_speed = r.f32()?;
        at_end(&r, data)?;
        Ok(PhysicsManager {
            gravity,
            default_material,
            bounce_threshold,
            sleep_threshold,
            default_contact_offset,
            default_solver_iterations,
            default_solver_velocity_iterations,
            queries_hit_backfaces,
            queries_hit_triggers,
            enable_adaptive_force,
            cloth_inter_collision_distance,
            cloth_inter_collision_stiffness,
            contacts_generation,
            layer_collision_matrix,
            auto_simulation,
            auto_sync_transforms,
            reuse_collision_callbacks,
            cloth_inter_collision_settings_toggle,
            cloth_gravity,
            contact_pairs_mode,
            broadphase_type,
            world_bounds,
            world_subdivisions,
            friction_type,
            enable_enhanced_determinism,
            enable_unified_heightmaps,
            solver_type,
            default_max_angular_speed,
        })
    }

    /// Whether layers `a` and `b` collide (layers outside 0..32: no).
    pub fn collides(&self, a: u32, b: u32) -> bool {
        b < 32
            && self
                .layer_collision_matrix
                .get(a as usize)
                .is_some_and(|row| row & (1 << b) != 0)
    }
}

/// `TagManager` (class 78): tag and layer names.
#[derive(Clone, Debug, PartialEq)]
pub struct TagManager {
    pub tags: Vec<String>,
    /// 32 entries; unnamed layers are empty.
    pub layers: Vec<String>,
    /// Sorting layers: name and unique id (a build stores no `locked`).
    pub sorting_layers: Vec<(String, u32)>,
}

impl TagManager {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<TagManager> {
        let mut r = Reader::new(data, big_endian);
        let strings = |r: &mut Reader| -> Result<Vec<String>> {
            let n = r.count(4)?;
            (0..n).map(|_| r.aligned_string()).collect()
        };
        let tags = strings(&mut r)?;
        let layers = strings(&mut r)?;
        let n = r.count(8)?;
        let mut sorting_layers = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.aligned_string()?;
            sorting_layers.push((name, r.u32()?));
        }
        at_end(&r, data)?;
        Ok(TagManager {
            tags,
            layers,
            sorting_layers,
        })
    }

    /// `LayerMask.NameToLayer`.
    pub fn layer(&self, name: &str) -> Option<u32> {
        self.layers
            .iter()
            .position(|l| l == name)
            .and_then(|i| u32::try_from(i).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn physics_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
        let i = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
        for v in [0.0, -9.81, 0.0] {
            f(&mut b, v);
        }
        i(&mut b, 0);
        b.extend_from_slice(&0i64.to_le_bytes());
        for v in [2.0, 0.005, 0.01] {
            f(&mut b, v);
        }
        i(&mut b, 6);
        i(&mut b, 1);
        b.extend_from_slice(&[1, 1, 0, 0]);
        f(&mut b, 0.0);
        f(&mut b, 0.0);
        i(&mut b, 1);
        i(&mut b, 32);
        for row in 0..32u32 {
            // Everything collides except layer 19 with anything.
            let bits = if row == 19 { 0 } else { !(1u32 << 19) };
            b.extend_from_slice(&bits.to_le_bytes());
        }
        b.extend_from_slice(&[1, 0, 1, 0]);
        for v in [0.0, -9.81, 0.0] {
            f(&mut b, v);
        }
        i(&mut b, 0);
        i(&mut b, 0);
        for v in [0.0, 0.0, 0.0, 250.0, 250.0, 250.0] {
            f(&mut b, v);
        }
        i(&mut b, 8);
        i(&mut b, 0);
        b.extend_from_slice(&[0, 1, 0, 0]);
        i(&mut b, 1);
        f(&mut b, 7.0);
        b
    }

    #[test]
    fn physics_manager_round_trip() {
        let b = physics_bytes();
        let m = PhysicsManager::parse(&b, false).unwrap();
        assert_eq!(m.gravity, [0.0, -9.81, 0.0]);
        assert!(m.queries_hit_backfaces && m.queries_hit_triggers);
        assert!(m.collides(0, 30) && !m.collides(0, 19) && !m.collides(19, 19));
        assert!(!m.collides(40, 0) && !m.collides(0, 40));
        assert_eq!(m.world_bounds[1], [250.0; 3]);
        assert!(m.enable_unified_heightmaps && !m.enable_enhanced_determinism);
        assert_eq!(m.cloth_gravity, [0.0, -9.81, 0.0]);
        assert_eq!((m.solver_type, m.default_max_angular_speed), (1, 7.0));
        for n in 0..b.len() {
            assert!(PhysicsManager::parse(&b[..n], false).is_err(), "prefix {n}");
            let mut bad = b.clone();
            bad[n] = 0xff;
            let _ = PhysicsManager::parse(&bad, false);
        }
    }

    #[test]
    fn tag_manager() {
        let mut b = Vec::new();
        let s = |b: &mut Vec<u8>, t: &str| {
            b.extend_from_slice(&(t.len() as i32).to_le_bytes());
            b.extend_from_slice(t.as_bytes());
            b.resize(b.len().next_multiple_of(4), 0);
        };
        b.extend_from_slice(&1i32.to_le_bytes());
        s(&mut b, "Player");
        b.extend_from_slice(&3i32.to_le_bytes());
        for l in ["Default", "", "Useable"] {
            s(&mut b, l);
        }
        b.extend_from_slice(&1i32.to_le_bytes());
        s(&mut b, "Default");
        b.extend_from_slice(&7u32.to_le_bytes());
        let t = TagManager::parse(&b, false).unwrap();
        assert_eq!(t.layer("Useable"), Some(2));
        assert_eq!(t.layer("Nope"), None);
        assert_eq!(t.sorting_layers, vec![("Default".into(), 7)]);
        for n in 0..b.len() {
            assert!(TagManager::parse(&b[..n], false).is_err(), "prefix {n}");
        }
    }

    #[test]
    fn rigidbody() {
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&5i64.to_le_bytes());
        for v in [70.0f32, 2.5, 0.05] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0, 1, 1, 0]);
        b.extend_from_slice(&112i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        let r = Rigidbody::parse(&b, false).unwrap();
        assert_eq!((r.mass, r.drag), (70.0, 2.5));
        assert!(!r.use_gravity && r.is_kinematic && r.interpolate == 1);
        assert_eq!((r.constraints, r.collision_detection), (112, 2));
        for n in 0..b.len() {
            assert!(Rigidbody::parse(&b[..n], false).is_err());
        }
    }

    #[test]
    fn time_manager() {
        let b: Vec<u8> = [0.02f32, 0.333, 1.0, 0.03]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let t = TimeManager::parse(&b, false).unwrap();
        assert_eq!(t.fixed_timestep, 0.02);
        assert!(TimeManager::parse(&b[..15], false).is_err());
        let mut long = b.clone();
        long.push(0);
        assert!(TimeManager::parse(&long, false).is_err());
    }
}
