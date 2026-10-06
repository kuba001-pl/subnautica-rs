//! Loads and meshes terrain on background threads, then hands finished
//! meshes to Bevy.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sn_install::GameData;
use sn_terrain::{Neighbourhood, batch_mesh, batch_voxels, debug_colour};
use sn_world::{BatchCoord, voxel_to_world};

/// The triangles of one batch that share a material, in Bevy coordinates.
pub struct TerrainPart {
    pub material: u8,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

pub enum LoaderMessage {
    /// How many batches will be meshed.
    Total(usize),
    Batch {
        coord: BatchCoord,
        parts: Vec<TerrainPart>,
    },
    Error(String),
}

#[derive(Resource)]
pub struct TerrainLoader {
    rx: Mutex<Receiver<LoaderMessage>>,
    pub started: Instant,
    pub total: Option<usize>,
    pub loaded: usize,
    pub triangles: usize,
    pub finished_after: Option<f32>,
    materials: HashMap<u8, Handle<StandardMaterial>>,
}

impl TerrainLoader {
    pub fn done(&self) -> bool {
        self.total == Some(self.loaded)
    }
}

/// Starts loading the cube of batches within `radius` of `center`.
pub fn start(game: GameData, center: BatchCoord, radius: i32) -> Result<TerrainLoader, String> {
    let index = game.read_index()?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        if let Err(e) = load(&game, &index, center, radius, &tx) {
            let _ = tx.send(LoaderMessage::Error(e));
        }
    });
    Ok(TerrainLoader {
        rx: Mutex::new(rx),
        started: Instant::now(),
        total: None,
        loaded: 0,
        triangles: 0,
        finished_after: None,
        materials: HashMap::new(),
    })
}

fn cube(center: BatchCoord, radius: i32) -> Vec<BatchCoord> {
    let mut out = Vec::new();
    for dz in -radius..=radius {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                out.push(center.offset(dx, dy, dz));
            }
        }
    }
    out
}

/// Runs `work` on every item using all but one CPU core.
fn parallel<T: Sync>(items: &[T], work: impl Fn(&T) + Sync) {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().max(2) - 1);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                    work(item);
                }
            });
        }
    });
}

fn load(
    game: &GameData,
    index: &sn_world::WorldIndex,
    center: BatchCoord,
    radius: i32,
    tx: &Sender<LoaderMessage>,
) -> Result<(), String> {
    // The region plus a 1-batch margin, which feeds the aprons.
    let wanted = cube(center, radius + 1);
    let grids = Mutex::new(HashMap::new());
    let errors = Mutex::new(Vec::new());
    parallel(&wanted, |&c| match game.load_batch(index, c) {
        Ok(Some(grid)) => {
            grids.lock().unwrap().insert(c, grid);
        }
        Ok(None) => {}
        Err(e) => errors.lock().unwrap().push(e.to_string()),
    });
    if let Some(e) = errors.into_inner().unwrap().into_iter().next() {
        return Err(e);
    }
    let grids = grids.into_inner().unwrap();

    let mut targets: Vec<BatchCoord> = cube(center, radius)
        .into_iter()
        .filter(|c| grids.contains_key(c))
        .collect();
    // Nearest first, so the area around the camera appears first.
    targets
        .sort_by_key(|c| (c.x - center.x).abs() + (c.y - center.y).abs() + (c.z - center.z).abs());
    let _ = tx.send(LoaderMessage::Total(targets.len()));

    let size = batch_voxels(index);
    let tx = Mutex::new(tx.clone());
    parallel(&targets, |&coord| {
        let around = Neighbourhood::new(coord, size, |n| grids.get(&n));
        if let Some(mesh) = batch_mesh(&around) {
            let parts = split_by_material(&mesh);
            let _ = tx
                .lock()
                .unwrap()
                .send(LoaderMessage::Batch { coord, parts });
        }
    });
    Ok(())
}

/// Unity is left-handed (x right, y up, z forward); Bevy is right-handed with
/// the same x and y, so z flips and triangle winding reverses.
fn unity_to_bevy(p: [f32; 3]) -> [f32; 3] {
    [p[0], p[1], -p[2]]
}

fn split_by_material(mesh: &sn_mesh::Mesh) -> Vec<TerrainPart> {
    let mut parts: HashMap<u8, (TerrainPart, Vec<u32>)> = HashMap::new();
    for (tri, &material) in mesh.triangles.iter().zip(&mesh.triangle_materials) {
        let (part, remap) = parts.entry(material).or_insert_with(|| {
            let part = TerrainPart {
                material,
                positions: Vec::new(),
                normals: Vec::new(),
                indices: Vec::new(),
            };
            (part, vec![u32::MAX; mesh.positions.len()])
        });
        let [a, b, c] = tri.map(|v| {
            let slot = &mut remap[v as usize];
            if *slot == u32::MAX {
                *slot = part.positions.len() as u32;
                part.positions
                    .push(unity_to_bevy(voxel_to_world(mesh.positions[v as usize])));
                part.normals.push(unity_to_bevy(mesh.normals[v as usize]));
            }
            *slot
        });
        part.indices.extend([a, c, b]);
    }
    parts.into_values().map(|(part, _)| part).collect()
}

/// Spawns finished batch meshes. Runs every frame; cheap when nothing arrived.
pub fn receive(
    mut commands: Commands,
    mut loader: ResMut<TerrainLoader>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    let messages: Vec<LoaderMessage> = loader.rx.lock().unwrap().try_iter().collect();
    for message in messages {
        match message {
            LoaderMessage::Total(n) => {
                info!("terrain: meshing {n} batches");
                loader.total = Some(n);
            }
            LoaderMessage::Error(e) => {
                error!("terrain loading failed: {e}");
                exit.write(AppExit::error());
            }
            LoaderMessage::Batch { coord, parts } => {
                let mut triangles = 0;
                for part in parts {
                    triangles += part.indices.len() / 3;
                    let material = loader
                        .materials
                        .entry(part.material)
                        .or_insert_with(|| {
                            let [r, g, b] = debug_colour(part.material);
                            materials.add(StandardMaterial {
                                base_color: Color::srgb(r, g, b),
                                perceptual_roughness: 0.9,
                                ..default()
                            })
                        })
                        .clone();
                    let mut mesh = Mesh::new(
                        PrimitiveTopology::TriangleList,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions);
                    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, part.normals);
                    mesh.insert_indices(Indices::U32(part.indices));
                    commands.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material),
                        Name::new(format!("terrain {coord} type {}", part.material)),
                    ));
                }
                loader.loaded += 1;
                loader.triangles += triangles;
                if loader.done() {
                    let secs = loader.started.elapsed().as_secs_f32();
                    loader.finished_after = Some(secs);
                    info!(
                        "terrain: {} batches, {} triangles loaded in {secs:.2} s",
                        loader.loaded, loader.triangles
                    );
                }
            }
        }
    }
}
