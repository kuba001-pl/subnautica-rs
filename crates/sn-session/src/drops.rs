//! Where the owner's dropped items land (M9d6a): one drop
//! (`Inventory.InternalDropItem`) and a death's (`LoseItems`, one
//! `InternalDropItem` per item), as the client computes them with its
//! collision and sends them; the server takes them as given
//! (`docs/DESIGN.md` "M9d3 plan"). Pure: the world and the draws come in.

use sn_protocol::{DeathDrop, DropAt, ItemId, quant};
use sn_sim::collide::{MOVE, World};
use sn_sim::inventory::{drop_ray, find_drop_position, random_rotation};
use sn_sim::{Q, V3};

/// The draws (`Random.value`s in [0, 1)): SplitMix64. The game's `Random`
/// is unseeded, so any seed is the game's behaviour.
#[derive(Clone, Debug, Default)]
pub struct Draws(pub u64);

impl Draws {
    pub fn value(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `Random.insideUnitCircle`: drawn uniformly in the unit disc.
    pub fn inside_unit_circle(&mut self) -> (f64, f64) {
        for _ in 0..64 {
            let (x, y) = (2.0 * self.value() - 1.0, 2.0 * self.value() - 1.0);
            if x * x + y * y <= 1.0 {
                return (x, y);
            }
        }
        (0.0, 0.0)
    }
}

/// One drop, with what led to it (for logs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dropped {
    pub at: DropAt,
    /// `RayCast`'s point (before `FindDropPosition`).
    pub ray: V3,
    pub value: f64,
    pub circle: (f64, f64),
}

/// The wire's rotation of `q`.
fn wire(q: Q) -> [i16; 4] {
    let s = quant::QUATERNION_SCALE;
    [q.x, q.y, q.z, q.w].map(|c| quant::fixed(c, s))
}

/// `InternalDropItem` from the camera: `drop_ray` (the length's value,
/// then the circle), then `find_drop_position`, in the solid group; the
/// rotation `Random.rotation` if `randomize` (three more draws), else
/// identity (our stand-in for the rotation the item had).
pub fn drop_at(
    world: &World,
    camera: V3,
    forward: V3,
    randomize: bool,
    draws: &mut Draws,
) -> Dropped {
    let value = draws.value();
    let circle = draws.inside_unit_circle();
    let ray = drop_ray(world, MOVE, camera, forward, value, circle);
    let point = find_drop_position(world, MOVE, camera, ray);
    let q = if randomize {
        random_rotation(draws.value(), draws.value(), draws.value())
    } else {
        Q::IDENTITY
    };
    Dropped {
        at: DropAt {
            point: point.to_f32(),
            rotation: wire(q),
        },
        ray,
        value,
        circle,
    }
}

/// A death's drops (`LoseItems`): the items of the lose list in order
/// (`ItemsState::lose_list`, each with its `randomizeRotationWhenDropped`),
/// each dropped from the same camera with its own draws. The origin is the
/// camera; each point is a millimetre offset from it (every drop is
/// within 1.5 m of the camera, well inside ±32 m).
pub fn death_drops(
    world: &World,
    camera: V3,
    forward: V3,
    items: &[(ItemId, bool)],
    draws: &mut Draws,
) -> ([f32; 3], Vec<DeathDrop>) {
    let origin = camera.to_f32();
    let scale = quant::DROP_OFFSET_SCALE;
    let drops = items
        .iter()
        .map(|&(_, randomize)| {
            let d = drop_at(world, camera, forward, randomize, draws);
            let offset = [0, 1, 2]
                .map(|i| quant::fixed(f64::from(d.at.point[i]) - f64::from(origin[i]), scale));
            DeathDrop {
                offset,
                rotation: d.at.rotation,
            }
        })
        .collect();
    (origin, drops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::death_point;

    #[test]
    fn draws_stay_in_range() {
        let mut d = Draws(7);
        for _ in 0..10_000 {
            let v = d.value();
            assert!((0.0..1.0).contains(&v));
            let (x, y) = d.inside_unit_circle();
            assert!(x * x + y * y <= 1.0);
        }
    }

    #[test]
    fn a_death_drops_each_item_near_the_camera() {
        // Open water: every drop is 0.75–1.5 m from the camera, within
        // 10° of the forward.
        let world = World::new();
        let camera = V3::new(10.0, -5.0, 3.0);
        let forward = V3::new(0.0, 0.0, 1.0);
        let items = [(ItemId(1), true), (ItemId(2), false), (ItemId(3), true)];
        let (origin, drops) = death_drops(&world, camera, forward, &items, &mut Draws(3));
        assert_eq!(drops.len(), 3);
        assert_eq!(origin, camera.to_f32());
        for d in &drops {
            let (p, _) = death_point(origin, d);
            let v = V3::from_f32(p) - camera;
            let len = v.length();
            assert!((0.749..=1.501).contains(&len), "{len}");
            let angle = (v.dot(forward) / len).acos().to_degrees();
            assert!(angle <= 14.2 + 1e-6, "{angle}");
        }
        // Without randomizeRotationWhenDropped: identity.
        assert_eq!(drops[1].rotation, [0, 0, 0, 32_767]);
        assert_ne!(drops[0].rotation, drops[2].rotation);
        // The same draws give the same drops.
        let again = death_drops(&world, camera, forward, &items, &mut Draws(3));
        assert_eq!(again.1, drops);
    }
}
