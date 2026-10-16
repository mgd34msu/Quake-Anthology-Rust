//! Cold BSP and entity boundary conversion for the shared runtime.
//! Spawn anchors describe the map; movement bounds are chosen by the player.
use qa_content::{
    products::{self, Edition},
    vfs::{MountKind, Vfs, normalize},
};
use qa_core::primitives::{Bounds, ClipNode, GeometryId, RuleSetId, SurfaceFlags, Vec3};
use qa_formats::{
    archive::ArchiveReader,
    bsp::{Bsp, BspFormat, Lump, Map, SurfaceKind},
    entities::{EntityLump, EntitySyntax, level_string},
};
use qa_render::{
    Assets,
    assets::PaletteId,
    material::world_load::{
        LoadedWorld, SkyEnvironment, WorldLoadOptions, WorldPresentationRequest, load_world,
    },
};
use qa_world::collision::{
    CollisionStore, Contents,
    brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    hulls::HullModel,
    patches::PatchInput,
    surfaces::{SurfaceInput, SurfaceTable},
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
    pub portals: Option<qa_world::portals::AreaPortals>,
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
    pub bytes: std::sync::Arc<[u8]>,
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
    let bytes = vfs
        .read_to_vec(file, usize::MAX, &mut ArchiveReader::default())
        .map_err(|error| format!("map {path}: {error}"))?;
    let bsp = Bsp::parse(&bytes).map_err(|e| format!("BSP directory: {e:?}"))?;
    let source = bsp.format.rule_set();
    let entity_source = NativeEntityText {
        syntax: source.lineage(),
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
        source_palette: Option<PaletteId>,
        presentations: &[WorldPresentationRequest],
        mut options: WorldLoadOptions,
        ui_fullscreen: bool,
    ) -> Result<LoadedMap, String> {
        let map = Map::parse(&self.bytes).map_err(|e| format!("BSP: {e:?}"))?;
        let collision_bounds = map.models.first().ok_or("missing world model")?.bounds;
        let portals = load_portals(&map)?;
        let (spawns, entity_count, sky_environment) = spawns(&map, self.native_source)?;
        let (collision, collision_brushes) = collision(&map, geometry)?;
        options.sky_environment = sky_environment;
        let render = match load_world(
            vfs,
            &map,
            assets,
            source_palette,
            presentations,
            options,
            ui_fullscreen,
        ) {
            Ok(render) => render,
            Err(error) => {
                geometry.remove(collision);
                return Err(format!("world assets: {error:?}"));
            }
        };
        Ok(LoadedMap {
            collision,
            portals,
            collision_bounds,
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

pub fn load_portals(map: &Map<'_>) -> Result<Option<qa_world::portals::AreaPortals>, String> {
    use qa_world::portals::{AreaPortals, Portal};
    let mut links = Vec::new();
    for (first, area) in map.areas.iter().enumerate() {
        for portal in &map.area_portals[area.first as usize..(area.first + area.count) as usize] {
            links.push(Portal {
                number: portal.portal,
                first: first as u32,
                second: portal.other_area,
            });
        }
    }
    let count = if map.areas.is_empty() {
        map.leaves
            .iter()
            .filter_map(|leaf| usize::try_from(leaf.area).ok())
            .max()
            .map_or(0, |area| area + 1)
    } else {
        map.areas.len()
    };
    if count == 0 {
        return Ok(None);
    }
    let numbers = map
        .area_portals
        .iter()
        .map(|p| p.portal as usize + 1)
        .max()
        .unwrap_or(0);
    AreaPortals::load(count, numbers, links)
        .map(Some)
        .map_err(|e| format!("area portals: {e:?}"))
}

pub(crate) fn virtual_path(name: &str) -> Result<String, String> {
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
    let syntax = source.lineage();
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
        .or_else(|| candidates.first().copied());
    let mut anchors: Vec<_> = primary.into_iter().collect();
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

fn patch_membership(
    output: &mut Vec<u32>,
    surfaces: impl IntoIterator<Item = u32>,
    patch_for_surface: &[Option<u32>],
) -> Result<qa_core::primitives::IndexSpan, String> {
    let first = u32::try_from(output.len()).map_err(|_| "leaf patch count")?;
    for surface in surfaces {
        if let Some(patch) = patch_for_surface
            .get(surface as usize)
            .ok_or("patch surface reference")?
        {
            output.push(*patch);
        }
    }
    let end = u32::try_from(output.len()).map_err(|_| "leaf patch count")?;
    Ok(qa_core::primitives::IndexSpan {
        first,
        count: end - first,
    })
}

/// Dedicated servers use the same conversion without registering visual assets.
pub fn collision(map: &Map<'_>, store: &mut CollisionStore) -> Result<(GeometryId, usize), String> {
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
                map.leaves[qa_core::primitives::leaf_from_child(child) as usize].contents
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
        let authored = map
            .authored_brushes
            .then(|| {
                map.brushes
                    .iter()
                    .map(|brush| {
                        let sides = map
                            .brush_sides
                            .get(brush.sides.range())
                            .ok_or("authored brush side range")?;
                        let first = sides.first().ok_or("authored brush has no sides")?.plane;
                        for (offset, side) in sides.iter().enumerate() {
                            if Some(side.plane)
                                != first.checked_add(
                                    u32::try_from(offset).map_err(|_| "authored side count")?,
                                )
                            {
                                return Err("authored brush plane order");
                            }
                        }
                        let mut contents = Contents::from_q1(brush.contents, Contents::SOLID);
                        if contents.intersects(Contents::SKY) {
                            contents |= Contents::VOLUME_CLIP;
                        }
                        Ok(Brush {
                            sides: qa_core::primitives::IndexSpan {
                                first,
                                count: brush.sides.count,
                            },
                            contents,
                        })
                    })
                    .collect::<Result<Vec<_>, &str>>()
            })
            .transpose()?;
        let models = map
            .models
            .iter()
            .map(|model| HullModel {
                roots: [
                    drawing_child(model.headnodes[0]),
                    model.headnodes[1],
                    model.headnodes[2],
                ],
                brushes: model.brushes,
            })
            .collect();
        let geometry = store
            .load_hulls(
                map.planes.clone(),
                drawing,
                map.clipnodes.clone(),
                models,
                bounds,
                authored,
                qa_world::collision::HullPositionLimits::default(),
            )
            .map_err(|e| format!("Q1 collision: {e:?}"))?;
        return Ok((
            geometry,
            if map.authored_brushes {
                map.brushes.len()
            } else {
                0
            },
        ));
    }
    // Brush numbers and ordered leaf references belong to the loaded map.
    // Model roots select membership; unplaced inline models are never part
    // of a world trace merely because their brushes share this storage.
    let mut planes = Vec::new();
    let q2 = matches!(map.bsp.format.rule_set(), RuleSetId::Quake2);
    let early = map.bsp.format == BspFormat::Quake3Test;
    let mut surfaces = SurfaceTable {
        records: vec![SurfaceInput::unnamed(SurfaceFlags::default())],
        sides: Vec::new(),
    };
    if q2 {
        surfaces
            .records
            .extend(
                map.texture_info
                    .iter()
                    .enumerate()
                    .map(|(index, row)| SurfaceInput {
                        name: row.name,
                        material: b"",
                        flags: SurfaceFlags::from_q2(row.flags as u32),
                        value: row.value,
                        source_index: index as u32 + 1,
                    }),
            );
    } else if !early {
        surfaces.records.extend(
            map.shaders
                .iter()
                .enumerate()
                .map(|(index, row)| SurfaceInput {
                    name: row.name,
                    material: b"",
                    flags: SurfaceFlags::from_q3(row.surface_flags as u32),
                    value: 0,
                    source_index: index as u32 + 1,
                }),
        );
    }
    let mut brushes = Vec::new();
    for source in &map.brushes {
        let first_plane = u32::try_from(planes.len()).map_err(|_| "collision plane count")?;
        for side in &map.brush_sides[source.sides.range()] {
            planes.push(map.planes[side.plane as usize]);
            surfaces.sides.push(if q2 {
                side.texture_info.map_or(0, |id| id + 1)
            } else if early {
                let index =
                    u32::try_from(surfaces.records.len()).map_err(|_| "collision surface count")?;
                surfaces.records.push(SurfaceInput {
                    name: side
                        .shader
                        .map_or(&[][..], |id| map.shaders[id as usize].name),
                    material: b"",
                    flags: SurfaceFlags::from_q3(side.flags as u32),
                    value: 0,
                    source_index: side.shader.map_or(0, |id| id + 1),
                });
                index
            } else {
                side.shader.map_or(0, |id| id + 1)
            });
        }
        brushes.push(Brush {
            sides: qa_core::primitives::IndexSpan {
                first: first_plane,
                count: source.sides.count,
            },
            contents: if matches!(map.bsp.format.rule_set(), RuleSetId::Quake2) {
                Contents::from_q2(source.contents as u32)
            } else {
                Contents::from_q3(source.contents as u32)
            },
        });
    }
    let mut patches = Vec::new();
    let mut patch_for_surface = vec![None; map.surfaces.len()];
    for (index, source) in map.surfaces.iter().enumerate() {
        if source.kind != SurfaceKind::Patch {
            continue;
        }
        let (surface, contents) = if let Some(shader) = source.shader {
            let row = map.shaders.get(shader as usize).ok_or("patch shader")?;
            (
                shader.checked_add(1).ok_or("collision surface count")?,
                Contents::from_q3(row.content_flags as u32),
            )
        } else if early {
            let side = u32::try_from(source.brush_side).ok();
            let flags = side
                .and_then(|side| map.brush_sides.get(side as usize))
                .map_or(0, |side| side.flags);
            // IBSP44 carries flags on the referenced side and contents on
            // its containing brush. An unassociated patch has no contents.
            let contents = side
                .and_then(|side| {
                    map.brushes
                        .iter()
                        .rev()
                        .find(|brush| brush.sides.contains(side as usize))
                })
                .map_or(Contents::EMPTY, |brush| {
                    Contents::from_q3(brush.contents as u32)
                });
            let surface =
                u32::try_from(surfaces.records.len()).map_err(|_| "collision surface count")?;
            surfaces.records.push(SurfaceInput {
                name: source.shader_name,
                material: b"",
                flags: SurfaceFlags::from_q3(flags as u32),
                value: 0,
                source_index: 0,
            });
            (surface, contents)
        } else {
            return Err("patch shader".into());
        };
        let dimensions = source.patch.map(|value| u32::try_from(value));
        let dimensions = [
            dimensions[0].map_err(|_| "patch width")?,
            dimensions[1].map_err(|_| "patch height")?,
        ];
        let control_points = map
            .vertices
            .get(source.vertices.range())
            .ok_or("patch vertices")?
            .iter()
            .map(|vertex| vertex.position)
            .collect();
        patch_for_surface[index] =
            Some(u32::try_from(patches.len()).map_err(|_| "collision patch count")?);
        patches.push(PatchInput {
            dimensions,
            control_points,
            contents,
            surface,
        });
    }
    let mut leaf_patches = Vec::new();
    let mut leaves: Vec<_> = map
        .leaves
        .iter()
        .map(|leaf| {
            let patches = if q2 {
                qa_core::primitives::IndexSpan::default()
            } else {
                patch_membership(
                    &mut leaf_patches,
                    map.leaf_faces
                        .get(leaf.faces.range())
                        .ok_or("leaf surface range")?
                        .iter()
                        .copied(),
                    &patch_for_surface,
                )?
            };
            Ok(CollisionLeaf {
                stored_contents: q2.then(|| Contents::from_q2(leaf.contents as u32)),
                brushes: leaf.brushes,
                patches,
            })
        })
        .collect::<Result<_, String>>()?;
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
                leaf_brushes.extend(source.brushes.range().map(|index| index as u32));
                leaves.push(CollisionLeaf {
                    stored_contents: None,
                    brushes: qa_core::primitives::IndexSpan {
                        first: first_brush,
                        count: source.brushes.count,
                    },
                    patches: patch_membership(
                        &mut leaf_patches,
                        source.faces.range().map(|surface| surface as u32),
                        &patch_for_surface,
                    )?,
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
        leaf_patches,
        models,
    };
    let count = brushes.len();
    let geometry = store
        .load_brushes(planes, brushes, patches, surfaces, tree, bounds)
        .map_err(|e| format!("brush collision: {e:?}"))?;
    Ok((geometry, count))
}

#[cfg(test)]
#[path = "../tests/fixtures/map_collision.rs"]
mod collision_tests;
