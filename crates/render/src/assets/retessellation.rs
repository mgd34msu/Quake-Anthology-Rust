use super::{Assets, Model, ModelId, Vertex};
use crate::{
    scene::Span,
    world::{
        WorldId, WorldPresentationId, WorldRenderSettings,
        geometry::{GeometryOptions, WorldGeometry, rebuild_geometry},
    },
};

pub(super) struct WorldMesh {
    pub(super) vertices: Vec<Vertex>,
    pub(super) indices: Vec<u32>,
    pub(super) ranges: Vec<Span>,
}

pub(super) fn world_mesh(geometry: &WorldGeometry) -> Result<WorldMesh, &'static str> {
    geometry
        .validate_patch_grids()
        .map_err(|_| "invalid world patch grid")?;
    let mut indices = Vec::new();
    let mut ranges = Vec::with_capacity(geometry.surfaces.len());
    for (source, surface) in geometry.surfaces.iter().enumerate() {
        let range = surface.indices.range();
        if source != surface.source_id as usize
            || range.end > geometry.indices.len()
            || !range.len().is_multiple_of(3)
        {
            return Err("invalid world triangle range");
        }
        let first = u32::try_from(indices.len()).map_err(|_| "world mesh full")?;
        indices.extend_from_slice(&geometry.indices[range]);
        ranges.push(Span {
            first,
            count: surface.indices.count,
        });
    }
    Ok(WorldMesh {
        vertices: geometry
            .vertices
            .iter()
            .map(|vertex| vertex.vertex)
            .collect(),
        indices,
        ranges,
    })
}

struct PreviousWorld {
    world: WorldId,
    geometry: WorldGeometry,
    mesh: ModelId,
    model: Model,
    presentations: Vec<(WorldPresentationId, Box<[Span]>)>,
    options: Option<crate::material::world_load::WorldLoadOptions>,
    light_grid: Option<crate::lighting::LightGrid>,
}

/// Cold prefix replacements remain owned until the caller's backend/frontend
/// transaction commits. Roll back this receipt before discarding asset suffixes.
#[must_use]
pub struct WorldRetessellation {
    previous: Vec<PreviousWorld>,
    model_preparation_revision: u64,
}
impl WorldRetessellation {
    pub fn worlds(&self) -> impl Iterator<Item = WorldId> + '_ {
        self.previous.iter().map(|previous| previous.world)
    }
    pub fn commit(self) {}

    /// Restore world prefixes and their mesh preparation revision. The enclosing
    /// transaction discards any other appended resources after this operation.
    pub fn rollback(self, assets: &mut Assets) -> Result<(), &'static str> {
        for previous in &self.previous {
            if assets
                .world(previous.world)
                .is_none_or(|world| world.mesh != previous.mesh)
                || assets.model(previous.mesh).is_none()
                || previous.presentations.iter().any(|&(id, ref ranges)| {
                    assets
                        .world_presentation(id)
                        .is_none_or(|presentation| presentation.bindings.len() != ranges.len())
                })
            {
                return Err("retessellation rollback prefix unavailable");
            }
        }
        for previous in self.previous.into_iter().rev() {
            let world = &mut assets.worlds[previous.world.0 as usize];
            world.geometry = previous.geometry;
            assets.models[previous.mesh.0 as usize] = previous.model;
            world.light_grid = previous.light_grid;
            if let Some(options) = previous.options
                && let Some(source) = world.material_source.as_mut()
            {
                source.restore_options(options);
            }
            for (id, ranges) in previous.presentations {
                for (binding, range) in assets.presentations[id.0 as usize]
                    .bindings
                    .iter_mut()
                    .zip(ranges)
                {
                    binding.mesh_indices = range;
                }
            }
        }
        assets.model_preparation_revision = self.model_preparation_revision;
        Ok(())
    }
}

impl Assets {
    /// Rebuild each selected world's retained source controls and lighting once.
    /// Source IDs, visibility and collision ownership remain unchanged.
    pub fn rebuild_worlds(
        &mut self,
        worlds: &[WorldId],
        settings: WorldRenderSettings,
    ) -> Result<WorldRetessellation, &'static str> {
        if !settings.patch_subdivisions.is_finite() {
            return Err("non-finite patch subdivision threshold");
        }
        let retained = self.counts();
        let mut changes = WorldRetessellation {
            previous: Vec::new(),
            model_preparation_revision: self.model_preparation_revision,
        };
        let result = (|| {
            for (index, &id) in worlds.iter().enumerate() {
                if worlds[..index].contains(&id) {
                    continue;
                }
                let world = self.world(id).ok_or("retessellation world unavailable")?;
                let mut options = world
                    .material_source
                    .as_ref()
                    .map_or_else(GeometryOptions::default, |source| source.geometry_options());
                let previous_options = world
                    .material_source
                    .as_ref()
                    .map(|source| source.load_options().clone());
                let previous_settings = world.render_settings();
                if previous_settings.is_none()
                    && settings.lighting != crate::world::WorldLighting::default()
                {
                    return Err("world lighting source unavailable");
                }
                let lighting_changed = previous_settings
                    .is_some_and(|previous| previous.lighting != settings.lighting);
                let patches_changed = (previous_options.is_none()
                    || previous_settings.is_some_and(|previous| {
                        previous.patch_subdivisions.to_bits()
                            != settings.patch_subdivisions.to_bits()
                    }))
                    && world
                        .geometry
                        .surfaces
                        .iter()
                        .any(|surface| surface.patch.is_some());
                if !lighting_changed && !patches_changed {
                    continue;
                }
                options.patch_subdivisions = settings.patch_subdivisions;
                let source_colors = world
                    .material_source
                    .as_ref()
                    .and_then(|source| source.vertex_colors());
                if source_colors.is_some() {
                    options.vertex_color_shift = settings.lighting.shift();
                }
                let geometry = rebuild_geometry(&world.geometry, options, source_colors)
                    .map_err(|_| "world geometry rebuild failed")?;
                let light_grid = world
                    .light_grid
                    .as_ref()
                    .map(|grid| grid.recolored(settings.lighting.shift()))
                    .transpose()?;
                let mesh = world_mesh(&geometry)?;
                let mesh_id = world.mesh;
                let shader = self.model(mesh_id).ok_or("world mesh unavailable")?.shader;
                let presentations = world
                    .presentations
                    .iter()
                    .map(|&presentation| {
                        let row = self
                            .world_presentation(presentation)
                            .filter(|row| {
                                row.world == id && row.bindings.len() == mesh.ranges.len()
                            })
                            .ok_or("retessellation presentation unavailable")?;
                        Ok((
                            presentation,
                            row.bindings
                                .iter()
                                .map(|binding| binding.mesh_indices)
                                .collect(),
                        ))
                    })
                    .collect::<Result<Vec<_>, &'static str>>()?;
                let model =
                    self.prepare_model(&mesh.vertices, mesh.vertices.len(), &mesh.indices, shader)?;
                let revision = self
                    .model_preparation_revision
                    .checked_add(1)
                    .ok_or("world model preparation revision exhausted")?;
                let world = &mut self.worlds[id.0 as usize];
                let previous = PreviousWorld {
                    world: id,
                    geometry: std::mem::replace(&mut world.geometry, geometry),
                    mesh: mesh_id,
                    model: std::mem::replace(&mut self.models[mesh_id.0 as usize], model),
                    presentations,
                    options: previous_options,
                    light_grid: std::mem::replace(&mut world.light_grid, light_grid),
                };
                if let Some(source) = world.material_source.as_mut() {
                    source.set_render_settings(settings);
                }
                for &(presentation, _) in &previous.presentations {
                    for (binding, &range) in self.presentations[presentation.0 as usize]
                        .bindings
                        .iter_mut()
                        .zip(&mesh.ranges)
                    {
                        binding.mesh_indices = range;
                    }
                }
                self.model_preparation_revision = revision;
                changes.previous.push(previous);
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(changes),
            Err(error) => {
                changes.rollback(self)?;
                self.discard_registration_suffix(retained)?;
                Err(error)
            }
        }
    }
}
