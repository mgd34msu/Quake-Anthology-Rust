//! Cold BSP and entity boundary conversion for the shared runtime.
//! Spawn anchors describe the map; movement bounds are chosen by the player.
use qa_content::{
    products::{self, Edition},
    vfs::{MountKind, Vfs, normalize},
};
use qa_core::primitives::{Bounds, ClipNode, GeometryId, RuleSetId, SurfaceFlags, Vec3};
use qa_formats::{
    archive::ArchiveReader,
    bsp::{Bsp, BspFormat, Lump, Map},
    entities::{EntityLump, EntitySyntax},
};
use qa_render::{
    Assets,
    material::world_load::{LoadedWorld, SkyEnvironment, WorldLoadOptions, load_world},
};
use qa_world::collision::{
    CollisionStore, Contents,
    brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    hulls::HullModel,
};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug)]
pub struct SpawnAnchor {
    pub position: Vec3,
    pub angles: Vec3,
    pub entity: usize,
    /// An extra authored anchor is a fixture until native module selection runs.
    pub fixture_fallback: bool,
}

pub struct LoadedMap {
    pub collision: GeometryId,
    pub collision_bounds: Bounds,
    pub render: LoadedWorld,
    pub spawns: Box<[SpawnAnchor]>,
    pub native_source: RuleSetId,
    pub virtual_path: String,
    pub entity_count: usize,
    pub collision_brushes: usize,
    pub entity_source: NativeEntityText,
}

/// Exact source bytes stay separate from folded runtime identities. Guest
/// fields, native spelling and protocol strings must not be rebuilt from IDs.
pub struct NativeEntityText {
    pub syntax: EntitySyntax,
    pub bytes: Box<[u8]>,
}

#[derive(Clone, Copy)]
enum NameString {
    LevelString,
    RawToken,
}

struct NameField {
    key: &'static [u8],
    string: NameString,
}
impl NameField {
    const fn level(key: &'static [u8]) -> Self {
        Self {
            key,
            string: NameString::LevelString,
        }
    }
    const fn raw(key: &'static [u8]) -> Self {
        Self {
            key,
            string: NameString::RawToken,
        }
    }
}

// Q1 progs106/defs.qc string fields; Q2 g_save.c fields[]; Q3 g_spawn.c
// fields[] and g_target.c SP_target_speaker. Numeric `sounds` is not a name.
const Q1_NAME_FIELDS: &[NameField] = &[
    NameField::level(b"classname"),
    NameField::level(b"model"),
    NameField::level(b"targetname"),
    NameField::level(b"target"),
    NameField::level(b"killtarget"),
    NameField::level(b"noise"),
    NameField::level(b"noise1"),
    NameField::level(b"noise2"),
    NameField::level(b"noise3"),
    NameField::level(b"noise4"),
    NameField::level(b"map"),
];
const Q2_NAME_FIELDS: &[NameField] = &[
    NameField::level(b"classname"),
    NameField::level(b"model"),
    NameField::level(b"map"),
    NameField::level(b"targetname"),
    NameField::level(b"target"),
    NameField::level(b"pathtarget"),
    NameField::level(b"deathtarget"),
    NameField::level(b"killtarget"),
    NameField::level(b"combattarget"),
    NameField::level(b"team"),
    NameField::level(b"noise"),
    NameField::level(b"item"),
    NameField::level(b"sky"),
    NameField::level(b"nextmap"),
];
const Q3_NAME_FIELDS: &[NameField] = &[
    NameField::level(b"classname"),
    NameField::level(b"model"),
    NameField::level(b"model2"),
    NameField::level(b"targetname"),
    NameField::level(b"target"),
    NameField::level(b"team"),
    NameField::level(b"targetShaderName"),
    NameField::level(b"targetShaderNewName"),
    NameField::raw(b"noise"),
];

fn entity_syntax(source: RuleSetId) -> EntitySyntax {
    match source {
        RuleSetId::Quake | RuleSetId::QuakeWorld => EntitySyntax::Quake,
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => EntitySyntax::Quake2,
        RuleSetId::Quake3 => EntitySyntax::Quake3,
    }
}

fn level_string(bytes: &[u8]) -> Cow<'_, [u8]> {
    if !bytes.contains(&b'\\') {
        return Cow::Borrowed(bytes);
    }
    // ED_NewString/G_NewString: \n becomes LF; every other escaped byte is
    // consumed and leaves one backslash. A final backslash remains a backslash.
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' {
            at += 1;
            decoded.push(if bytes.get(at) == Some(&b'n') {
                b'\n'
            } else {
                b'\\'
            });
        } else {
            decoded.push(bytes[at]);
        }
        at += 1;
    }
    Cow::Owned(decoded)
}

impl NativeEntityText {
    /// Collect native identity values before the common catalog binds IDs.
    /// This is not a module spawn or a complete module precache inventory.
    pub fn catalog_names(&self) -> Result<Vec<Cow<'_, [u8]>>, String> {
        let entities = EntityLump::parse(&self.bytes, self.syntax)
            .map_err(|e| format!("entity names: {e:?}"))?;
        let descriptors = match self.syntax {
            EntitySyntax::Quake => Q1_NAME_FIELDS,
            EntitySyntax::Quake2 => Q2_NAME_FIELDS,
            EntitySyntax::Quake3 => Q3_NAME_FIELDS,
        };
        let rules = (0..entities.names.len())
            .map(|index| {
                let mut key = entities
                    .names
                    .get(qa_core::primitives::NameId(index as u32))
                    .ok_or_else(|| "invalid native entity key id".to_owned())?;
                if self.syntax == EntitySyntax::Quake {
                    // ED_ParseEdict trims trailing spaces before exact field lookup.
                    while let Some(trimmed) = key.strip_suffix(b" ") {
                        key = trimmed;
                    }
                }
                Ok(descriptors
                    .iter()
                    .find(|field| {
                        if self.syntax == EntitySyntax::Quake {
                            key == field.key
                        } else {
                            key.eq_ignore_ascii_case(field.key)
                        }
                    })
                    .map(|field| field.string))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(entities
            .fields
            .iter()
            .filter_map(|field| {
                rules[field.key.0 as usize].map(|rule| match rule {
                    NameString::LevelString => level_string(field.value),
                    NameString::RawToken => Cow::Borrowed(field.value),
                })
            })
            .collect())
    }
}

/// Owned cold input retains one VFS read while settings are imported. The
/// validated lump directory selects its source; records are decoded at load.
pub struct MapInput {
    /// Stock startup metadata from the winning mount, never the BSP format.
    pub client_rules: Option<RuleSetId>,
    bytes: Vec<u8>,
    pub native_source: RuleSetId,
    pub virtual_path: String,
    profile_product: ProfileProduct,
    entity_source: NativeEntityText,
}

enum ProfileProduct {
    Stock(String),
    Unselected(String),
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
    let bsp = Bsp::parse(&bytes).map_err(|e| format!("BSP directory: {e:?}"))?;
    let source = bsp.format.rule_set();
    let entity_source = NativeEntityText {
        syntax: entity_syntax(source),
        bytes: bsp.bytes(Lump::Entities).into(),
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
    let product_metadata = products::root_metadata(directory);
    let profile_product = if let Some(metadata) = product_metadata {
        let profile_root =
            std::str::from_utf8(metadata.root_hint).map_err(|_| "invalid product root hint")?;
        let edition = if metadata.edition == Edition::Rerelease {
            "rerelease/"
        } else {
            ""
        };
        ProfileProduct::Stock(format!("{profile_root}/{edition}{product}"))
    } else {
        ProfileProduct::Unselected(product.to_owned())
    };
    Ok(MapInput {
        client_rules: product_metadata.map(|metadata| metadata.client_rules),
        bytes,
        native_source: source,
        virtual_path: path,
        profile_product,
        entity_source,
    })
}

impl MapInput {
    pub fn profile_product(&self, client: RuleSetId) -> Cow<'_, str> {
        match &self.profile_product {
            ProfileProduct::Stock(path) => Cow::Borrowed(path),
            ProfileProduct::Unselected(directory) => {
                let root = match client {
                    RuleSetId::Quake | RuleSetId::QuakeWorld => "q1",
                    RuleSetId::Quake2 => "q2",
                    RuleSetId::Quake2Rerelease => "q2/rerelease",
                    RuleSetId::Quake3 => "q3a",
                };
                Cow::Owned(format!("{root}/{directory}"))
            }
        }
    }

    pub fn catalog_names(&self) -> Result<Vec<Cow<'_, [u8]>>, String> {
        self.entity_source.catalog_names()
    }

    /// Decode map/entities/collision once, then register resources using the
    /// caller's already-selected saved settings and command-line overrides.
    pub fn load(
        self,
        vfs: &Vfs,
        assets: &mut Assets,
        geometry: &mut CollisionStore,
        mut options: WorldLoadOptions,
    ) -> Result<LoadedMap, String> {
        let map = Map::parse(&self.bytes).map_err(|e| format!("BSP: {e:?}"))?;
        let (spawns, entity_count, sky_environment) = spawns(&map, self.native_source)?;
        let (collision, collision_brushes) = collision(&map, geometry)?;
        options.sky_environment = sky_environment;
        let render = match load_world(vfs, &map, assets, options) {
            Ok(render) => render,
            Err(error) => {
                geometry.remove(collision);
                return Err(format!("world assets: {error:?}"));
            }
        };
        Ok(LoadedMap {
            collision,
            collision_bounds: map.models.first().ok_or("missing world model")?.bounds,
            render,
            spawns,
            native_source: self.native_source,
            virtual_path: self.virtual_path,
            entity_count,
            collision_brushes,
            entity_source: self.entity_source,
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

fn spawns(
    map: &Map<'_>,
    source: RuleSetId,
) -> Result<(Box<[SpawnAnchor]>, usize, SkyEnvironment), String> {
    let rules = map.bsp.format.rule_set();
    let syntax = entity_syntax(source);
    let entities =
        EntityLump::parse(map.entity_text(), syntax).map_err(|e| format!("entity lump: {e:?}"))?;
    let mut preferred = None;
    let mut fallback = None;
    let mut candidates = Vec::new();
    let mut sky_environment = SkyEnvironment::default();
    for (index, range) in entities.records.iter().enumerate() {
        let fields = &entities.fields[range.clone()];
        let field = |name: &[u8]| {
            let id = if syntax == EntitySyntax::Quake {
                entities.names.find(name)
            } else {
                entities.names.find_folded(name)
            }?;
            fields.iter().find(|entry| entry.key == id).map(|e| e.value)
        };
        let Some(classname) = field(b"classname") else {
            continue;
        };
        if matches!(rules, RuleSetId::Quake2) && classname == b"worldspawn" {
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
        let desired: &[u8] = if matches!(rules, RuleSetId::Quake3) {
            b"info_player_deathmatch"
        } else {
            b"info_player_start"
        };
        if classname != desired
            && classname != b"info_player_deathmatch"
            && classname != b"info_player_coop"
        {
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
        position.0[2] += if matches!(rules, RuleSetId::Quake) {
            1.0
        } else {
            9.0
        };
        let anchor = SpawnAnchor {
            position,
            angles: if matches!(rules, RuleSetId::Quake2) {
                Vec3([0.0, angles.0[1], 0.0])
            } else {
                angles
            },
            entity: index,
            fixture_fallback: matches!(rules, RuleSetId::Quake3),
        };
        candidates.push(SpawnAnchor {
            fixture_fallback: true,
            ..anchor
        });
        if classname != desired {
            continue;
        }
        fallback.get_or_insert(anchor);
        let initial = if matches!(rules, RuleSetId::Quake2) {
            field(b"targetname").is_none_or(|v| v.is_empty())
        } else if matches!(rules, RuleSetId::Quake3) {
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
    let primary = preferred
        .or(fallback)
        .or_else(|| candidates.first().copied())
        .ok_or("no supported player spawn in the map")?;
    let mut anchors = vec![primary];
    for anchor in candidates {
        if !anchors
            .iter()
            .any(|other| other.position == anchor.position)
        {
            anchors.push(anchor);
        }
    }
    Ok((
        anchors.into_boxed_slice(),
        entities.records.len(),
        sky_environment,
    ))
}

fn collision(map: &Map<'_>, store: &mut CollisionStore) -> Result<(GeometryId, usize), String> {
    map.models.first().ok_or("missing world model")?;
    // Mod_LoadSubmodels/CMod_LoadSubmodels expands collision model bounds at
    // load. Entity linking applies its own independent one-unit expansion.
    let bounds = map
        .models
        .iter()
        .map(|model| Bounds {
            mins: model.bounds.mins - Vec3([1.0; 3]),
            maxs: model.bounds.maxs + Vec3([1.0; 3]),
        })
        .collect();
    if matches!(map.bsp.format.rule_set(), RuleSetId::Quake) {
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
        let geometry = store
            .load_hulls(
                map.planes.clone(),
                drawing,
                map.clipnodes.clone(),
                models,
                bounds,
            )
            .map_err(|e| format!("Q1 collision: {e:?}"))?;
        return Ok((geometry, 0));
    }
    // Brush numbers and ordered leaf references belong to the loaded map.
    // Model roots select membership; unplaced inline models are never part
    // of a world trace merely because their brushes share this storage.
    let mut planes = Vec::new();
    let mut surfaces = Vec::new();
    let mut brushes = Vec::new();
    for source in &map.brushes {
        let first_plane = u32::try_from(planes.len()).map_err(|_| "collision plane count")?;
        for side in &map.brush_sides[source.sides.indices()] {
            planes.push(map.planes[side.plane as usize]);
            surfaces.push(if matches!(map.bsp.format.rule_set(), RuleSetId::Quake2) {
                SurfaceFlags::from_q2(
                    side.texture_info
                        .map_or(0, |id| map.texture_info[id as usize].flags as u32),
                )
            } else {
                let flags = if map.bsp.format == BspFormat::Quake3Test {
                    side.flags as u32
                } else {
                    side.shader
                        .map_or(0, |id| map.shaders[id as usize].surface_flags as u32)
                };
                SurfaceFlags::from_q3(flags)
            });
        }
        brushes.push(Brush {
            first_plane,
            plane_count: source.sides.count,
            contents: if matches!(map.bsp.format.rule_set(), RuleSetId::Quake2) {
                Contents::from_q2(source.contents as u32)
            } else {
                Contents::from_q3(source.contents as u32)
            },
        });
    }
    let mut leaves: Vec<_> = map
        .leaves
        .iter()
        .map(|leaf| CollisionLeaf {
            stored_contents: matches!(map.bsp.format.rule_set(), RuleSetId::Quake2)
                .then(|| Contents::from_q2(leaf.contents as u32)),
            first_brush: leaf.brushes.first,
            brush_count: leaf.brushes.count,
        })
        .collect();
    let mut leaf_brushes = map.leaf_brushes.clone();
    let mut models = Vec::with_capacity(map.models.len());
    for (index, source) in map.models.iter().enumerate() {
        if matches!(map.bsp.format, BspFormat::Quake3 | BspFormat::QuakeLive) {
            if index == 0 {
                models.push(ModelRoot::Tree(0));
            } else {
                // qsrc CMod_LoadSubmodels creates a direct leaf for each
                // Q3 inline model. Keep its contiguous native brush order.
                let first_brush =
                    u32::try_from(leaf_brushes.len()).map_err(|_| "leaf brush count")?;
                let leaf = u32::try_from(leaves.len()).map_err(|_| "collision leaf count")?;
                leaf_brushes
                    .extend(source.brushes.first..source.brushes.first + source.brushes.count);
                leaves.push(CollisionLeaf {
                    stored_contents: None,
                    first_brush,
                    brush_count: source.brushes.count,
                });
                models.push(ModelRoot::Leaf(leaf));
            }
        } else {
            // Q2 and the early Q3 test format retain their own BSP roots.
            models.push(ModelRoot::Tree(source.headnodes[0]));
        }
    }
    let tree = BrushTree {
        planes: map.planes.clone(),
        nodes: map
            .nodes
            .iter()
            .map(|node| ClipNode {
                plane: node.plane,
                children: node.children,
            })
            .collect(),
        leaves,
        leaf_brushes,
        models,
    };
    let count = brushes.len();
    let geometry = store
        .load_brushes(planes, brushes, surfaces, tree, bounds)
        .map_err(|e| format!("brush collision: {e:?}"))?;
    Ok((geometry, count))
}

#[cfg(test)]
#[path = "../tests/fixtures/map_collision.rs"]
mod collision_tests;
