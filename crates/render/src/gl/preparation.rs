use super::{
    ARRAY_BUFFER, Assets, CloudGrid, ELEMENT_ARRAY_BUFFER, Gl, Mesh, RGBA, Sky, SkyBatches,
    TEXTURE_2D, TextureParams, UNSIGNED_BYTE, Vertex, size_of, vertex_layout,
};

#[derive(Default)]
pub(super) struct StaticObjects {
    pub static_vao: u32,
    pub model_vao: u32,
    pub buffers: [u32; 2],
}
impl StaticObjects {
    pub(super) fn delete(&self, gl: &Gl) {
        // SAFETY: Each guard or backend owns these names on its current context.
        // Zero names are ignored, including partially generated candidates.
        unsafe {
            (gl.delete_buffers)(2, self.buffers.as_ptr());
            (gl.delete_vertex_arrays)(1, &self.static_vao);
            (gl.delete_vertex_arrays)(1, &self.model_vao);
        }
    }
}

pub(super) struct UploadBindings<'a> {
    gl: &'a Gl,
    active: i32,
    texture: i32,
    unpack: i32,
}
impl<'a> UploadBindings<'a> {
    pub(super) fn new(gl: &'a Gl) -> Self {
        let mut saved = Self {
            gl,
            active: 0,
            texture: 0,
            unpack: 0,
        };
        // SAFETY: Upload borrows the backend's current context and changes only
        // unit zero's 2D binding. Evaluator-table unit one is never rebound.
        unsafe {
            (gl.get_integer)(0x84e0, &mut saved.active);
            (gl.get_integer)(0x0cf5, &mut saved.unpack);
            (gl.active_texture)(0x84c0);
            (gl.get_integer)(0x8069, &mut saved.texture);
        }
        saved
    }

    pub(super) fn replace_texture(&mut self, previous: u32, replacement: u32) {
        if self.texture as u32 == previous {
            self.texture = replacement as i32;
        }
    }
}
impl Drop for UploadBindings<'_> {
    fn drop(&mut self) {
        // SAFETY: The backend still owns the same current context. VAO/cache
        // invalidation avoids retaining deleted candidate or retired names.
        unsafe {
            (self.gl.bind_vertex_array)(0);
            (self.gl.bind_buffer)(ARRAY_BUFFER, 0);
            (self.gl.active_texture)(0x84c0);
            (self.gl.bind_texture)(TEXTURE_2D, self.texture as u32);
            (self.gl.pixel_store)(0x0cf5, self.unpack);
            (self.gl.active_texture)(self.active as u32);
        }
    }
}

struct MeshLayout {
    meshes: Box<[Mesh]>,
    vertices: usize,
    indices: usize,
    vertex_bytes: isize,
    index_bytes: isize,
}
impl MeshLayout {
    fn prepare(lengths: impl IntoIterator<Item = (usize, usize)>) -> Result<Self, String> {
        let mut meshes = Vec::new();
        let mut vertices = 0usize;
        let mut indices = 0usize;
        for (vertex_count, index_count) in lengths {
            let base_vertex = i32::try_from(vertices).map_err(|_| "GL vertex table full")?;
            let count = i32::try_from(index_count).map_err(|_| "GL mesh index table full")?;
            meshes.push(Mesh {
                first_index: indices,
                count,
                base_vertex,
            });
            vertices = vertices
                .checked_add(vertex_count)
                .ok_or("GL vertex table full")?;
            indices = indices
                .checked_add(index_count)
                .ok_or("GL index table full")?;
        }
        let bytes = |count: usize, element: usize| {
            count
                .checked_mul(element)
                .and_then(|size| isize::try_from(size).ok())
                .ok_or_else(|| "GL static buffer size overflow".to_owned())
        };
        Ok(Self {
            meshes: meshes.into_boxed_slice(),
            vertices,
            indices,
            vertex_bytes: bytes(vertices, size_of::<Vertex>())?,
            index_bytes: bytes(indices, size_of::<u32>())?,
        })
    }
}

pub(super) struct PreparedAssets<'a> {
    gl: &'a Gl,
    pub objects: StaticObjects,
    pub meshes: Box<[Mesh]>,
    pub textures: Box<[u32]>,
    pub(super) changed_images: Box<[crate::ImageId]>,
    pub texture_params: Box<[TextureParams]>,
    pub sky_clouds: Box<[Option<CloudGrid>]>,
    pub skies: SkyBatches,
}
impl<'a> PreparedAssets<'a> {
    pub(super) fn prepare(
        gl: &'a Gl,
        assets: &Assets,
        textures: &[u32],
        texture_params: &[TextureParams],
        sky_capacity: usize,
        changed_images: Box<[crate::ImageId]>,
    ) -> Result<Self, String> {
        let layout = MeshLayout::prepare(
            assets
                .models()
                .iter()
                .map(|model| (model.vertices.len(), model.indices.len())),
        )?;
        let sky_clouds = assets
            .materials()
            .iter()
            .map(|material| match material.settings.sky {
                Some(Sky::Cube { clouds, .. }) if clouds.valid() && !material.stages.is_empty() => {
                    CloudGrid::generate(clouds).map(Some).map_err(|error| {
                        format!(
                            "{}: {error}",
                            assets.name(material.name).unwrap_or("<unknown material>")
                        )
                    })
                }
                _ => Ok(None),
            })
            .collect::<Result<Box<[_]>, String>>()?;
        i32::try_from(assets.images().len()).map_err(|_| "GL texture table full")?;
        let appended = assets
            .images()
            .get(textures.len()..)
            .ok_or("GL texture prefix shrank")?;
        let mut vertices = Vec::with_capacity(layout.vertices);
        let mut indices = Vec::with_capacity(layout.indices);
        for model in assets.models() {
            vertices.extend_from_slice(&model.vertices);
            indices.extend_from_slice(&model.indices);
        }
        let mut candidate = Self {
            gl,
            objects: StaticObjects::default(),
            meshes: layout.meshes,
            textures: textures
                .iter()
                .copied()
                .chain(appended.iter().map(|_| 0))
                .collect(),
            changed_images,
            texture_params: texture_params
                .iter()
                .copied()
                .chain(appended.iter().map(|image| TextureParams {
                    inverse_intensity: image.prepared.as_ref().map_or(1.0, |p| p.inverse_intensity),
                    sampler: image.native_sampler,
                }))
                .collect(),
            sky_clouds,
            skies: SkyBatches::new(assets.materials().len(), sky_capacity),
        };
        for &image in &candidate.changed_images {
            let index = image.0 as usize;
            candidate.textures[index] = 0;
            let image = &assets.images()[index];
            candidate.texture_params[index] = TextureParams {
                inverse_intensity: image.prepared.as_ref().map_or(1.0, |p| p.inverse_intensity),
                sampler: image.native_sampler,
            };
        }
        check_error(gl, "before asset upload")?;
        // SAFETY: All slices are owned for this synchronous upload; checked mesh
        // extents fit GLsizeiptr, and Assets bounds image/mip dimensions at load.
        // The guard owns only new objects until its tables are committed.
        unsafe {
            (gl.gen_vertex_arrays)(1, &mut candidate.objects.static_vao);
            (gl.gen_buffers)(2, candidate.objects.buffers.as_mut_ptr());
            (gl.bind_vertex_array)(candidate.objects.static_vao);
            (gl.bind_buffer)(ARRAY_BUFFER, candidate.objects.buffers[0]);
            (gl.buffer_data)(
                ARRAY_BUFFER,
                layout.vertex_bytes,
                vertices.as_ptr().cast(),
                0x88e4,
            );
            vertex_layout(gl, 0, None);
            (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, candidate.objects.buffers[1]);
            (gl.buffer_data)(
                ELEMENT_ARRAY_BUFFER,
                layout.index_bytes,
                indices.as_ptr().cast(),
                0x88e4,
            );
            (gl.gen_vertex_arrays)(1, &mut candidate.objects.model_vao);
            (gl.bind_vertex_array)(candidate.objects.model_vao);
            vertex_layout(gl, 0, Some(0));
            (gl.bind_buffer)(ELEMENT_ARRAY_BUFFER, candidate.objects.buffers[1]);
        }
        check_error(gl, "static mesh upload")?;
        if !candidate.changed_images.is_empty() {
            // SAFETY: UploadBindings restores this context's previous alignment.
            unsafe { (gl.pixel_store)(0x0cf5, 1) };
        }
        for &id in &candidate.changed_images {
            let image = &assets.images()[id.0 as usize];
            let texture = &mut candidate.textures[id.0 as usize];
            // SAFETY: Assets validates the RGBA/mip spans and native dimensions.
            // Each new name occupies only its candidate slot until commit.
            unsafe {
                (gl.gen_textures)(1, texture);
                (gl.bind_texture)(TEXTURE_2D, *texture);
                if let Some(prepared) = &image.prepared {
                    for (level, pixels) in prepared.levels.iter().enumerate() {
                        (gl.texture_image)(
                            TEXTURE_2D,
                            level as i32,
                            0x8058,
                            pixels.width as i32,
                            pixels.height as i32,
                            0,
                            RGBA,
                            UNSIGNED_BYTE,
                            pixels.rgba.as_ptr().cast(),
                        );
                    }
                    (gl.texture_parameter)(
                        TEXTURE_2D,
                        0x813d,
                        prepared.levels.len().saturating_sub(1) as i32,
                    );
                } else {
                    (gl.texture_image)(
                        TEXTURE_2D,
                        0,
                        0x8058,
                        image.width as i32,
                        image.height as i32,
                        0,
                        RGBA,
                        UNSIGNED_BYTE,
                        image.rgba.as_ptr().cast(),
                    );
                    (gl.generate_mipmap)(TEXTURE_2D);
                }
                (gl.texture_parameter)(TEXTURE_2D, 0x2801, 0x2701);
                (gl.texture_parameter)(TEXTURE_2D, 0x2800, 0x2601);
                (gl.texture_parameter)(TEXTURE_2D, 0x2802, 0x2901);
                (gl.texture_parameter)(TEXTURE_2D, 0x2803, 0x2901);
            }
            check_error(gl, "image upload")?;
        }
        check_error(gl, "asset upload")?;
        Ok(candidate)
    }
}
impl Drop for PreparedAssets<'_> {
    fn drop(&mut self) {
        self.objects.delete(self.gl);
        for id in &self.changed_images {
            if let Some(texture) = self.textures.get(id.0 as usize) {
                // SAFETY: Before commit these slots own candidate names. After
                // the table swap they own only replaced old names; appended
                // slots are absent. Unchanged borrowed names are never deleted.
                unsafe { (self.gl.delete_textures)(1, texture) };
            }
        }
    }
}

fn check_error(gl: &Gl, phase: &str) -> Result<(), String> {
    // SAFETY: The backend's context is current for this whole admission scope.
    let error = unsafe { (gl.get_error)() };
    if error == 0 {
        Ok(())
    } else {
        Err(format!("GL {phase} failed with error 0x{error:04x}"))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/gl_preparation.rs"]
mod tests;
