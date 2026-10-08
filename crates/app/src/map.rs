//! Cold BSP and entity boundary conversion for the shared runtime.
//! Spawn anchors describe the map; movement bounds are chosen by the player.
use qa_console::views::Source;
use qa_content::vfs::{MountKind, Vfs, normalize};
use qa_core::primitives::{ClipNode, MovementRules, SurfaceFlags, Vec3};
use qa_formats::{
    archive::ArchiveReader,
    bsp::{Bsp, BspFormat, Map},
    entities::{EntityLump, EntitySyntax},
};
use qa_render::{
    Assets,
    material::world_load::{LoadedWorld, SkyEnvironment, WorldLoadOptions, load_world},
};
use qa_world::collision::{
    CollisionWorld, Contents,
    brushes::{Brush, BrushMap},
    hulls::{HullModel, Q1Hulls},
};

#[derive(Clone, Copy, Debug)]
pub struct SpawnAnchor {
    pub position: Vec3,
    pub angles: Vec3,
    pub entity: usize,
    /// Q3's random/telefrag-aware module fallback is not loaded at this gate.
    pub fixture_fallback: bool,
}

pub struct LoadedMap {
    pub collision: CollisionWorld,
    pub render: LoadedWorld,
    pub spawn: SpawnAnchor,
    pub native_source: Source,
    pub virtual_path: String,
    pub profile_product: String,
    pub entity_count: usize,
    pub collision_brushes: usize,
}

/// Owned cold input retains one VFS read while settings are imported. The
/// validated lump directory selects its source; records are decoded at load.
pub struct MapInput {
    bytes: Vec<u8>,
    pub native_source: Source,
    pub virtual_path: String,
    pub profile_product: String,
}

pub fn movement(name: &str) -> Result<MovementRules, &'static str> {
    match name {
        "q1" => Ok(MovementRules::Quake),
        "qw" => Ok(MovementRules::QuakeWorld),
        "q2" => Ok(MovementRules::Quake2),
        "q2rr" => Ok(MovementRules::Quake2Rerelease),
        "q3" => Ok(MovementRules::Quake3),
        _ => Err("movement must be q1, qw, q2, q2rr or q3"),
    }
}

pub fn movement_name(rules: MovementRules) -> &'static str {
    match rules {
        MovementRules::Quake => "q1",
        MovementRules::QuakeWorld => "qw",
        MovementRules::Quake2 => "q2",
        MovementRules::Quake2Rerelease => "q2rr",
        MovementRules::Quake3 => "q3",
    }
}

pub fn native_movement(source: Source) -> MovementRules {
    match source {
        Source::Quake => MovementRules::Quake,
        Source::QuakeWorld => MovementRules::QuakeWorld,
        Source::Quake2 => MovementRules::Quake2,
        Source::Quake2Rerelease => MovementRules::Quake2Rerelease,
        Source::Quake3 => MovementRules::Quake3,
    }
}

/// Read once and establish the settings source before asset preparation.
pub fn read(vfs: &Vfs, name: &str) -> Result<MapInput, String> {
    let path = virtual_path(name)?;
    let file = vfs
        .open(path.as_bytes())
        .ok_or_else(|| format!("map not found in VFS: {path}"))?;
    let length = usize::try_from(vfs.length(file).map_err(|e| format!("map size: {e:?}"))?)
        .map_err(|_| "map too large for this platform")?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "map allocation failed")?;
    bytes.resize(length, 0);
    let read = vfs
        .read_into_reusing(file, &mut bytes, &mut ArchiveReader::default())
        .map_err(|e| format!("map read: {e:?}"))?;
    if read != length {
        return Err("incomplete map read".into());
    }
    let format = Bsp::parse(&bytes)
        .map_err(|e| format!("BSP directory: {e:?}"))?
        .format;
    let source = match format.family() {
        1 => Source::Quake,
        2 => Source::Quake2,
        _ => Source::Quake3,
    };
    let origin = vfs.origin(file).ok_or("missing map origin")?;
    let directory = if origin.kind == MountKind::Directory {
        origin.path
    } else {
        origin
            .path
            .parent()
            .ok_or("map archive has no product directory")?
    };
    let product = directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid map product directory")?;
    let profile_product = format!(
        "{}/{product}",
        match source {
            Source::Quake | Source::QuakeWorld => "q1",
            Source::Quake2 | Source::Quake2Rerelease => "q2",
            Source::Quake3 => "q3a",
        }
    );
    Ok(MapInput {
        bytes,
        native_source: source,
        virtual_path: path,
        profile_product,
    })
}

impl MapInput {
    /// Decode map/entities/collision once, then register resources using the
    /// caller's already-selected saved settings and command-line overrides.
    pub fn load(
        self,
        vfs: &Vfs,
        assets: &mut Assets,
        mut options: WorldLoadOptions,
    ) -> Result<LoadedMap, String> {
        let map = Map::parse(&self.bytes).map_err(|e| format!("BSP: {e:?}"))?;
        let (spawn, entity_count, sky_environment) = spawn(&map)?;
        let (collision, collision_brushes) = collision(&map)?;
        options.sky_environment = sky_environment;
        let render =
            load_world(vfs, &map, assets, options).map_err(|e| format!("world assets: {e:?}"))?;
        Ok(LoadedMap {
            collision,
            render,
            spawn,
            native_source: self.native_source,
            virtual_path: self.virtual_path,
            profile_product: self.profile_product,
            entity_count,
            collision_brushes,
        })
    }
}

fn virtual_path(name: &str) -> Result<String, String> {
    if name.starts_with('/') || name.starts_with('\\') {
        return Err("map must be a virtual VFS name".into());
    }
    let path = if name.contains('/') || name.contains('\\') {
        name.to_owned()
    } else {
        format!("maps/{name}")
    };
    let path = if path.to_ascii_lowercase().ends_with(".bsp") {
        path
    } else {
        path + ".bsp"
    };
    let normalized = normalize(path.as_bytes()).map_err(|e| format!("map path: {e:?}"))?;
    String::from_utf8(normalized).map_err(|_| "map path must be UTF-8".into())
}

fn vector(bytes: &[u8]) -> Result<Vec3, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "non-UTF-8 spawn vector")?;
    let mut values = text.split_ascii_whitespace();
    let mut result = [0.0; 3];
    for value in &mut result {
        *value = values
            .next()
            .ok_or("incomplete spawn vector")?
            .parse::<f32>()
            .map_err(|_| "invalid spawn vector")?;
        if !value.is_finite() {
            return Err("non-finite spawn vector".into());
        }
    }
    if values.next().is_some() {
        return Err("too many spawn vector coordinates".into());
    }
    Ok(Vec3(result))
}

fn spawn(map: &Map<'_>) -> Result<(SpawnAnchor, usize, SkyEnvironment), String> {
    let family = map.bsp.format.family();
    let syntax = match family {
        1 => EntitySyntax::Quake,
        2 => EntitySyntax::Quake2,
        _ => EntitySyntax::Quake3,
    };
    let entities =
        EntityLump::parse(map.entity_text(), syntax).map_err(|e| format!("entity lump: {e:?}"))?;
    let mut preferred = None;
    let mut fallback = None;
    let mut sky_environment = SkyEnvironment::default();
    for (index, range) in entities.records.iter().enumerate() {
        let fields = &entities.fields[range.clone()];
        let field = |name: &[u8]| {
            let id = entities.names.find(name)?;
            fields.iter().find(|entry| entry.key == id).map(|e| e.value)
        };
        let Some(classname) = field(b"classname") else {
            continue;
        };
        if family == 2 && classname == b"worldspawn" {
            let name = field(b"sky")
                .filter(|name| !name.is_empty())
                .unwrap_or(b"unit1_");
            let rate = field(b"skyrotate")
                .map(|value| {
                    std::str::from_utf8(value)
                        .map_err(|_| "invalid sky rotation")?
                        .trim()
                        .parse::<f32>()
                        .map_err(|_| "invalid sky rotation")
                })
                .transpose()?
                .unwrap_or(0.0);
            let axis = field(b"skyaxis")
                .map(vector)
                .transpose()?
                .unwrap_or_default();
            sky_environment = SkyEnvironment::new(name, rate, axis)?;
        }
        let desired: &[u8] = if family == 3 {
            b"info_player_deathmatch"
        } else {
            b"info_player_start"
        };
        if classname != desired {
            continue;
        }
        let mut position = vector(field(b"origin").unwrap_or(b"0 0 0"))?;
        let angles = if let Some(angles) = field(b"angles") {
            vector(angles)?
        } else if let Some(angle) = field(b"angle") {
            let yaw = std::str::from_utf8(angle)
                .map_err(|_| "invalid spawn angle")?
                .parse::<f32>()
                .map_err(|_| "invalid spawn angle")?;
            if !yaw.is_finite() {
                return Err("non-finite spawn angle".into());
            }
            Vec3([0.0, yaw, 0.0])
        } else {
            Vec3::default()
        };
        // Q1 client.qc PutClientInServer; Q2 p_client.c SelectSpawnPoint
        // (the pmove origin, not its temporary entity +1); Q3 g_client.c.
        position.0[2] += if family == 1 { 1.0 } else { 9.0 };
        let anchor = SpawnAnchor {
            position,
            angles: if family == 2 {
                Vec3([0.0, angles.0[1], 0.0])
            } else {
                angles
            },
            entity: index,
            fixture_fallback: family == 3,
        };
        fallback.get_or_insert(anchor);
        let initial = if family == 2 {
            field(b"targetname").is_none_or(|v| v.is_empty())
        } else if family == 3 {
            field(b"spawnflags")
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|flags| flags & 1 != 0)
        } else {
            true
        };
        if initial && preferred.is_none() {
            preferred = Some(SpawnAnchor {
                fixture_fallback: false,
                ..anchor
            });
        }
    }
    preferred
        .or(fallback)
        .map(|spawn| (spawn, entities.records.len(), sky_environment))
        .ok_or_else(|| "no supported player spawn in the map".into())
}

fn collision(map: &Map<'_>) -> Result<(CollisionWorld, usize), String> {
    let model = map.models.first().ok_or("missing world model")?;
    if map.bsp.format.family() == 1 {
        let drawing_child = |child: i32| {
            if child >= 0 {
                child
            } else {
                map.leaves[(-1i64 - i64::from(child)) as usize].contents
            }
        };
        let drawing = map
            .nodes
            .iter()
            .map(|node| ClipNode {
                plane: node.plane,
                children: node.children.map(drawing_child),
            })
            .collect();
        let models = map
            .models
            .iter()
            .map(|model| HullModel {
                roots: [
                    drawing_child(model.headnodes[0]),
                    model.headnodes[1],
                    model.headnodes[2],
                ],
            })
            .collect();
        let hulls = Q1Hulls::load(map.planes.clone(), drawing, map.clipnodes.clone(), models)
            .map_err(|e| format!("Q1 collision: {e:?}"))?;
        return Ok((CollisionWorld::Hulls(hulls), 0));
    }
    let mut included = vec![false; map.brushes.len()];
    if matches!(map.bsp.format, BspFormat::Quake3 | BspFormat::QuakeLive) {
        included[model.brushes.indices()].fill(true);
    } else {
        // Q2 models name a BSP root, not a brush range. Only reachable world
        // leaves contribute; inline model brushes remain unplaced.
        let mut visited = vec![false; map.nodes.len()];
        let mut nodes = vec![model.headnodes[0]];
        while let Some(node) = nodes.pop() {
            if node >= 0 {
                let index = node as usize;
                if visited[index] {
                    continue;
                }
                visited[index] = true;
                nodes.extend_from_slice(&map.nodes[index].children);
            } else {
                let leaf = &map.leaves[(-1i64 - i64::from(node)) as usize];
                for &brush in &map.leaf_brushes[leaf.brushes.indices()] {
                    included[brush as usize] = true;
                }
            }
        }
    }
    let mut planes = Vec::new();
    let mut surfaces = Vec::new();
    let mut brushes = Vec::new();
    for (index, source) in map.brushes.iter().enumerate() {
        if !included[index] {
            continue;
        }
        let first_plane = u32::try_from(planes.len()).map_err(|_| "collision plane count")?;
        for side in &map.brush_sides[source.sides.indices()] {
            planes.push(map.planes[side.plane as usize]);
            surfaces.push(if map.bsp.format.family() == 2 {
                SurfaceFlags::from_q2(
                    side.texture_info
                        .map_or(0, |id| map.texture_info[id as usize].flags as u32),
                )
            } else {
                SurfaceFlags::from_q3(
                    side.shader
                        .map_or(0, |id| map.shaders[id as usize].surface_flags as u32),
                )
            });
        }
        brushes.push(Brush {
            first_plane,
            plane_count: source.sides.count,
            contents: if map.bsp.format.family() == 2 {
                Contents::from_q2(source.contents as u32)
            } else {
                Contents::from_q3(source.contents as u32)
            },
        });
    }
    let count = brushes.len();
    let brushes = BrushMap::load_surfaces(planes, brushes, surfaces)
        .map_err(|e| format!("brush collision: {e:?}"))?;
    Ok((CollisionWorld::Brushes(brushes), count))
}
