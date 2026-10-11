//! Dropping an item: where it is allowed (`Inventory.CanDropItemHere`)
//! and where it lands (`Inventory.RayCast`, then
//! `Pickupable.FindDropPosition`). Water parks are not ported (there are
//! none yet).

use crate::collide::World;
use crate::{Q, V3};

/// Where the player is, as `CanDropItemHere` asks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropPlace {
    /// In a sub or a base (`currentSub`).
    pub in_sub: bool,
    /// In the lifepod (`escapePod`).
    pub in_escape_pod: bool,
    /// Piloting a vehicle.
    pub piloting: bool,
    /// Out of the water in a Precursor base (`precursorOutOfWater`).
    pub precursor_out_of_water: bool,
}

/// `CanDropItemHere`.
pub fn can_drop_here(place: DropPlace) -> bool {
    !(place.in_sub || place.in_escape_pod || place.piloting || place.precursor_out_of_water)
}

/// `InternalDropItem`'s `RayCast(camera, forward, 10°, 0.75, 1.5)`.
pub const DROP_ANGLE: f64 = 10.0;
pub const DROP_MIN: f64 = 0.75;
pub const DROP_MAX: f64 = 1.5;
/// `RayCast` stops this far before what it hits.
pub const DROP_RAY_BACK: f64 = 0.2;

/// `Inventory.RayCast` with `InternalDropItem`'s numbers: a ray from the
/// camera, turned by up to 10° (`Quaternion.Euler(10 cx, 10 cy, 0)` about
/// the world's axes), `0.75 + 0.75 value` long, stopped 0.2 m before a
/// hit in `groups` (our cast stops up to `collide::SKIN` short of a
/// surface, the game's at it). `value` and `circle` are the draws
/// (`Random.Range(0.75, 1.5)`'s value, `Random.insideUnitCircle`).
pub fn drop_ray(
    world: &World,
    groups: u32,
    origin: V3,
    forward: V3,
    value: f64,
    circle: (f64, f64),
) -> V3 {
    let dir = forward.normalized().unwrap_or(V3::ZERO);
    let mut distance = DROP_MIN + (DROP_MAX - DROP_MIN) * value;
    let ray = Q::euler(DROP_ANGLE * circle.0, DROP_ANGLE * circle.1, 0.0).rotate(dir);
    if let Some((d, _)) = world.cast(groups, origin, ray, 0.0, distance + DROP_RAY_BACK) {
        distance = (d - DROP_RAY_BACK).max(0.0);
    }
    origin + ray * distance
}

/// `FindDropPosition`'s sphere and reach.
pub const DROP_SPHERE: f64 = 0.3;
pub const DROP_REACH: f64 = 5.0;
/// It stops this far before the contact.
pub const DROP_SPHERE_BACK: f64 = 0.15;

/// `Pickupable.FindDropPosition(camera, desired)`: a 0.3 m sphere cast
/// from the camera towards `desired`, up to 5 m (not just to `desired`);
/// the first contact moves the point to 0.15 m before it. A contact at
/// distance 0 (already touching at the camera) is ignored; the game then
/// takes the next contact, we keep `desired` (**not tested** how often
/// that differs).
pub fn find_drop_position(world: &World, groups: u32, camera: V3, desired: V3) -> V3 {
    let Some(dir) = (desired - camera).normalized() else {
        return desired;
    };
    match world.cast(groups, camera, dir, DROP_SPHERE, DROP_REACH) {
        // The contact point is on the sphere's surface, towards the shape.
        Some((d, hit)) if d > 0.0 => {
            camera + dir * d - hit.normal * DROP_SPHERE - dir * DROP_SPHERE_BACK
        }
        _ => desired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::{Body, MOVE, SKIN, Shape};

    /// A wall: the plane z = `z`, facing -z, as two big triangles.
    fn wall(z: f32) -> World {
        let (a, b, c, d) = (
            [-50.0, -50.0, z],
            [50.0, -50.0, z],
            [50.0, 50.0, z],
            [-50.0, 50.0, z],
        );
        let mut w = World::new();
        // Both windings, so that the side does not matter.
        w.insert(
            1,
            Body::new(
                [[a, b, c], [a, c, d], [a, c, b], [a, d, c]],
                Vec::<Shape>::new(),
            ),
        );
        w
    }

    fn close(a: V3, b: V3) -> bool {
        (a - b).length() < 1e-6
    }

    /// Our casts stop up to `SKIN` short of a surface.
    fn near(a: V3, b: V3) -> bool {
        (a - b).length() <= SKIN + 1e-9
    }

    #[test]
    fn no_drop_in_the_pod_a_sub_or_while_piloting() {
        assert!(can_drop_here(DropPlace::default()));
        for p in [
            DropPlace {
                in_escape_pod: true,
                ..Default::default()
            },
            DropPlace {
                in_sub: true,
                ..Default::default()
            },
            DropPlace {
                piloting: true,
                ..Default::default()
            },
            DropPlace {
                precursor_out_of_water: true,
                ..Default::default()
            },
        ] {
            assert!(!can_drop_here(p));
        }
    }

    #[test]
    fn the_ray_reaches_its_length_in_open_water() {
        let w = World::new();
        let z = V3::new(0.0, 0.0, 1.0);
        assert!(close(
            drop_ray(&w, MOVE, V3::ZERO, z, 0.0, (0.0, 0.0)),
            z * 0.75
        ));
        assert!(close(
            drop_ray(&w, MOVE, V3::ZERO, z * 3.0, 1.0, (0.0, 0.0)),
            z * 1.5
        ));
        // Turned 10° about y (circle y = 1): yaw to the right.
        let p = drop_ray(&w, MOVE, V3::ZERO, z, 1.0, (0.0, 1.0));
        let r = 10f64.to_radians();
        assert!(close(p, V3::new(r.sin(), 0.0, r.cos()) * 1.5), "{p:?}");
    }

    #[test]
    fn the_drop_point_stops_before_a_wall() {
        let w = wall(1.0);
        let z = V3::new(0.0, 0.0, 1.0);
        // The ray would reach 1.5 m: it stops 0.2 m before the wall.
        let p = drop_ray(&w, MOVE, V3::ZERO, z, 1.0, (0.0, 0.0));
        assert!(near(p, z * 0.8), "{p:?}");
        // The sphere (0.3 m) touches the wall at 0.7 m; the point is
        // 0.15 m back from the contact at 1.0 m.
        let q = find_drop_position(&w, MOVE, V3::ZERO, p);
        assert!(near(q, z * 0.85), "{q:?}");
        // With the wall beyond the sphere's reach the point stays.
        let w = wall(10.0);
        assert!(close(find_drop_position(&w, MOVE, V3::ZERO, z), z));
        assert!(close(find_drop_position(&w, MOVE, z, z), z));
    }

    #[test]
    fn a_wall_at_the_camera_drops_at_the_camera() {
        let w = wall(0.1);
        let z = V3::new(0.0, 0.0, 1.0);
        assert!(close(
            drop_ray(&w, MOVE, V3::ZERO, z, 0.5, (0.0, 0.0)),
            V3::ZERO
        ));
        // Nothing in the cast's groups: no wall.
        let p = drop_ray(&w, 0, V3::ZERO, z, 0.0, (0.0, 0.0));
        assert!(close(p, z * 0.75));
    }
}
