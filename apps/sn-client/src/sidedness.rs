//! Keep model-space face orientation under reflected object transforms.
//! Mesh variants share all attributes; only triangle winding changes.

use std::collections::HashMap;

use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::Face;

use crate::object_look::ObjectMaterial;

#[derive(Resource, Default)]
pub(crate) struct ReflectedMeshes {
    reversed: HashMap<AssetId<Mesh>, Handle<Mesh>>,
    originals: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

fn reverse_triangles(indices: &mut Indices) {
    match indices {
        Indices::U16(indices) => {
            for triangle in indices.chunks_exact_mut(3) {
                triangle.swap(1, 2);
            }
        }
        Indices::U32(indices) => {
            for triangle in indices.chunks_exact_mut(3) {
                triangle.swap(1, 2);
            }
        }
    }
}

fn reversed_mesh(mesh: &Mesh) -> Option<Mesh> {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let mut reversed = mesh.clone();
    reverse_triangles(reversed.try_indices_mut().ok()?);
    Some(reversed)
}

/// Run after transform propagation, so nested and animated scale signs
/// count. Switching back restores the original handle, without repeatedly
/// reversing an already reversed mesh. The original is never modified.
/// Only parts whose transform or mesh changed are checked (plus parts whose
/// mesh data was not available yet); the logged census is recounted only
/// when parts appear, disappear, switch material or flip.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn repair_winding(
    mut cache: ResMut<ReflectedMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Res<Assets<ObjectMaterial>>,
    mut parts: ParamSet<(
        Query<(
            &GlobalTransform,
            &mut Mesh3d,
            &MeshMaterial3d<ObjectMaterial>,
        )>,
        Query<
            Entity,
            (
                With<MeshMaterial3d<ObjectMaterial>>,
                Or<(
                    Changed<GlobalTransform>,
                    Changed<Mesh3d>,
                    Added<MeshMaterial3d<ObjectMaterial>>,
                )>,
            ),
        >,
        Query<
            (),
            (
                With<Mesh3d>,
                Or<(Added<Mesh3d>, Changed<MeshMaterial3d<ObjectMaterial>>)>,
            ),
        >,
    )>,
    mut removed: RemovedComponents<Mesh3d>,
    mut retry: Local<Vec<Entity>>,
    mut last: Local<Option<[usize; 5]>>,
) {
    let _perf = crate::perf::span("sidedness::repair_winding");
    let mut recount = !parts.p2().is_empty() | (removed.read().count() > 0);
    let retried = retry.len();
    let mut pending = std::mem::take(&mut *retry);
    pending.extend(parts.p1().iter());
    let mut objects = parts.p0();
    pending.sort_unstable();
    pending.dedup();
    for entity in pending {
        let Ok((transform, mut mesh, _)) = objects.get_mut(entity) else {
            continue;
        };
        let original = cache
            .originals
            .get(&mesh.0.id())
            .cloned()
            .unwrap_or_else(|| mesh.0.clone());
        let reflected = transform.affine().matrix3.determinant() < 0.0;
        let wanted = if reflected {
            if let Some(reversed) = cache.reversed.get(&original.id()) {
                reversed.clone()
            } else {
                let Some(reversed) = meshes.get(&original).and_then(reversed_mesh) else {
                    retry.push(entity);
                    continue;
                };
                let reversed = meshes.add(reversed);
                cache.originals.insert(reversed.id(), original.clone());
                cache.reversed.insert(original.id(), reversed.clone());
                reversed
            }
        } else {
            original
        };
        if mesh.0.id() != wanted.id() {
            mesh.0 = wanted;
            recount = true;
        }
    }
    recount |= retry.len() != retried;
    if !recount && last.is_some() {
        return;
    }
    let mut counts = [0; 5]; // off, front, back, reflected, unsupported
    for (transform, _, material) in &objects {
        if let Some(material) = materials.get(&material.0) {
            let index = match material.base.cull_mode {
                None => 0,
                Some(Face::Front) => 1,
                Some(Face::Back) => 2,
            };
            counts[index] += 1;
        }
        if transform.affine().matrix3.determinant() < 0.0 {
            counts[3] += 1;
        }
    }
    counts[4] = retry.len();
    if *last != Some(counts) {
        *last = Some(counts);
        info!(
            "objects: face orientation: {} off, {} front, {} back; {} reflected instances, {} reversed mesh variants, {} unsupported",
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            cache.reversed.len(),
            counts[4]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    #[test]
    fn late_material_and_delayed_mesh_are_repaired_without_a_transform_change() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<ObjectMaterial>>()
            .init_resource::<ReflectedMeshes>()
            .add_systems(Update, repair_winding);
        let original = app.world().resource::<Assets<Mesh>>().reserve_handle();
        let entity = app
            .world_mut()
            .spawn((
                GlobalTransform::from(Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0))),
                Mesh3d(original.clone()),
            ))
            .id();
        app.update();
        app.world_mut()
            .entity_mut(entity)
            .insert(MeshMaterial3d::<ObjectMaterial>(Handle::default()));
        app.update();
        assert_eq!(
            app.world().get::<Mesh3d>(entity).unwrap().0.id(),
            original.id()
        );
        let mut geometry = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        geometry.insert_indices(Indices::U32(vec![0, 1, 2]));
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(original.id(), geometry)
            .unwrap();
        app.update();
        let repaired = app.world().get::<Mesh3d>(entity).unwrap().0.id();
        assert_ne!(repaired, original.id());
        for _ in 0..3 {
            app.update();
            assert_eq!(app.world().get::<Mesh3d>(entity).unwrap().0.id(), repaired);
        }
        assert_eq!(app.world().resource::<ReflectedMeshes>().reversed.len(), 1);
    }

    #[test]
    fn already_extracted_mesh_is_rejected_without_panicking() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let _gpu = mesh.take_gpu_data().unwrap();
        assert!(reversed_mesh(&mesh).is_none());
    }

    #[test]
    fn delayed_reflected_instance_can_reverse_an_object_mesh_after_gpu_extraction() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<ObjectMaterial>>()
            .init_resource::<ReflectedMeshes>()
            .add_systems(Update, repair_winding);
        let mesh = crate::objects::build_mesh(crate::objects::MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            tangents: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            indices: vec![0, 1, 2],
            joints: Vec::new(),
            weights: Vec::new(),
            morphs: Vec::new(),
        });
        let original = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        app.world_mut().spawn((
            GlobalTransform::IDENTITY,
            Mesh3d(original.clone()),
            MeshMaterial3d::<ObjectMaterial>(Handle::default()),
        ));
        app.update();
        // Exercise Bevy's extraction lifetime decision before any reflected
        // instance exists. Old render-only object meshes lose their data here.
        {
            let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
            let mut mesh = meshes.get_mut(&original).unwrap();
            let _gpu = if mesh.asset_usage.contains(RenderAssetUsages::MAIN_WORLD) {
                mesh.clone()
            } else {
                mesh.take_gpu_data().unwrap()
            };
        }
        for _ in 0..32 {
            app.update();
        }
        let entity = app
            .world_mut()
            .spawn((
                GlobalTransform::from(Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0))),
                Mesh3d(original.clone()),
                MeshMaterial3d::<ObjectMaterial>(Handle::default()),
            ))
            .id();
        app.update();
        let reversed = &app.world().get::<Mesh3d>(entity).unwrap().0;
        assert_ne!(reversed.id(), original.id());
        let meshes = app.world().resource::<Assets<Mesh>>();
        assert_eq!(
            meshes
                .get(reversed)
                .unwrap()
                .try_indices()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [0, 2, 1]
        );
        assert_eq!(
            meshes
                .get(&original)
                .unwrap()
                .try_indices()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn reflected_triangle_winding_agrees_with_transformed_normal_for_all_scale_signs() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::Y];
        for x in [-2.0, 2.0] {
            for y in [-3.0, 3.0] {
                for z in [-4.0, 4.0] {
                    let transform = Mat4::from_scale(Vec3::new(x, y, z));
                    let mut indices = Indices::U32(vec![0, 1, 2]);
                    if transform.determinant() < 0.0 {
                        reverse_triangles(&mut indices);
                    }
                    let indices: Vec<_> = indices.iter().collect();
                    let world = [0, 1, 2].map(|i| transform.transform_point3(points[indices[i]]));
                    let normal = transform
                        .inverse()
                        .transpose()
                        .transform_vector3(Vec3::Z)
                        .normalize();
                    assert!((world[1] - world[0]).cross(world[2] - world[0]).dot(normal) > 0.0);
                }
            }
        }
    }

    #[test]
    fn reversing_preserves_geometry_and_is_reversible_for_both_index_widths() {
        for mut indices in [
            Indices::U16(vec![0, 1, 2, 2, 1, 3]),
            Indices::U32(vec![0, 1, 2, 2, 1, 3]),
        ] {
            let original: Vec<_> = indices.iter().collect();
            reverse_triangles(&mut indices);
            assert_eq!(indices.iter().collect::<Vec<_>>(), [0, 2, 1, 2, 3, 1]);
            reverse_triangles(&mut indices);
            assert_eq!(indices.iter().collect::<Vec<_>>(), original);
        }
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let reversed = reversed_mesh(&mesh).unwrap();
        assert_eq!(
            reversed.indices().unwrap().iter().collect::<Vec<_>>(),
            [0, 2, 1]
        );
        assert_eq!(
            mesh.indices().unwrap().iter().collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(
            matches!(reversed.attribute(Mesh::ATTRIBUTE_POSITION), Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) if *p == positions)
        );
    }

    #[test]
    fn scale_sign_changes_restore_the_original_and_share_one_variant() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<ObjectMaterial>>()
            .init_resource::<ReflectedMeshes>()
            .add_systems(Update, repair_winding);
        let mut geometry = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        geometry.insert_indices(Indices::U32(vec![0, 1, 2]));
        let original = app.world_mut().resource_mut::<Assets<Mesh>>().add(geometry);
        let reflected = GlobalTransform::from(Transform::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
        let entity = app
            .world_mut()
            .spawn((
                reflected,
                Mesh3d(original.clone()),
                MeshMaterial3d::<ObjectMaterial>(Handle::default()),
            ))
            .id();
        app.update();
        let reversed = app.world().get::<Mesh3d>(entity).unwrap().0.id();
        assert_ne!(reversed, original.id());
        app.update();
        assert_eq!(app.world().get::<Mesh3d>(entity).unwrap().0.id(), reversed);
        *app.world_mut().get_mut::<GlobalTransform>(entity).unwrap() = GlobalTransform::IDENTITY;
        app.update();
        assert_eq!(
            app.world().get::<Mesh3d>(entity).unwrap().0.id(),
            original.id()
        );
        *app.world_mut().get_mut::<GlobalTransform>(entity).unwrap() = reflected;
        app.update();
        assert_eq!(app.world().get::<Mesh3d>(entity).unwrap().0.id(), reversed);
        assert_eq!(app.world().resource::<ReflectedMeshes>().reversed.len(), 1);
        assert_eq!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(&original)
                .unwrap()
                .indices()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }
}
