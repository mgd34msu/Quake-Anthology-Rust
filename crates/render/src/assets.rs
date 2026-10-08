//! Registration is a load operation. Frames use numeric handles only.
use qa_core::primitives::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub position: Vec3,
    pub texcoord: [f32; 2],
    pub lightmap_coord: [f32; 2],
    pub color: [u8; 4],
}
impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: Vec3::default(),
            texcoord: [0.0; 2],
            lightmap_coord: [0.0; 2],
            color: [255; 4],
        }
    }
}
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Box<[u8]>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    #[default]
    Opaque,
    Alpha,
    Add,
    Multiply,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaTest {
    #[default]
    None,
    GreaterZero,
    AtLeastHalf,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TcGen {
    #[default]
    Texture,
    Lightmap,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DepthFunc {
    #[default]
    Lequal,
    Equal,
    Always,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub image: ImageId,
    pub blend: Blend,
    pub alpha_test: AlphaTest,
    pub texgen: TcGen,
    pub vertex_color: bool,
    pub depth_func: DepthFunc,
    pub depth_write: bool,
}
impl Default for Stage {
    fn default() -> Self {
        Self {
            image: ImageId(0),
            blend: Blend::Opaque,
            alpha_test: AlphaTest::None,
            texgen: TcGen::Texture,
            vertex_color: false,
            depth_func: DepthFunc::Lequal,
            depth_write: true,
        }
    }
}
pub struct Material {
    pub name: String,
    pub stages: Box<[Stage]>,
    pub two_sided: bool,
    pub sort: u16,
}
pub struct Model {
    pub vertices: Box<[Vertex]>,
    pub indices: Box<[u32]>,
    pub material: MaterialId,
}
pub struct Assets {
    images: Vec<Image>,
    materials: Vec<Material>,
    models: Vec<Model>,
}
impl Default for Assets {
    fn default() -> Self {
        Self::load()
    }
}
impl Assets {
    pub fn load() -> Self {
        Self {
            images: vec![Image {
                width: 1,
                height: 1,
                rgba: vec![255; 4].into_boxed_slice(),
            }],
            materials: vec![Material {
                name: "*white".into(),
                stages: vec![Stage {
                    vertex_color: true,
                    ..Stage::default()
                }]
                .into_boxed_slice(),
                two_sided: true,
                sort: 0,
            }],
            models: vec![Model {
                vertices: Box::new([]),
                indices: Box::new([]),
                material: MaterialId(0),
            }],
        }
    }
    pub fn register_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<ImageId, &'static str> {
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || rgba.len() != width as usize * height as usize * 4
        {
            return Err("invalid RGBA image dimensions");
        }
        let id = ImageId(u32::try_from(self.images.len()).map_err(|_| "image table full")?);
        self.images.push(Image {
            width,
            height,
            rgba: rgba.into(),
        });
        Ok(id)
    }
    pub fn register_material(
        &mut self,
        name: &str,
        stages: &[Stage],
        two_sided: bool,
        sort: u16,
    ) -> Result<MaterialId, &'static str> {
        if let Some(index) = self.materials.iter().position(|m|
            m.name == name && m.stages.as_ref() == stages && m.two_sided == two_sided && m.sort == sort) {
            return Ok(MaterialId(index as u32));
        }
        if stages.is_empty()
            || stages.len() > 8
            || stages.iter().any(|s| self.image(s.image).is_none())
        {
            return Err("invalid material stages");
        }
        let id =
            MaterialId(u32::try_from(self.materials.len()).map_err(|_| "material table full")?);
        self.materials.push(Material {
            name: name.into(),
            stages: stages.into(),
            two_sided,
            sort,
        });
        Ok(id)
    }
    pub fn register_model(
        &mut self,
        vertices: &[Vertex],
        indices: &[u32],
        material: MaterialId,
    ) -> Result<ModelId, &'static str> {
        if self.material(material).is_none()
            || !indices.len().is_multiple_of(3)
            || indices.iter().any(|&i| i as usize >= vertices.len())
            || vertices.iter().any(|v| {
                v.position
                    .0
                    .iter()
                    .chain(v.texcoord.iter())
                    .chain(v.lightmap_coord.iter())
                    .any(|f| !f.is_finite())
            })
        {
            return Err("invalid mesh");
        }
        let id = ModelId(u32::try_from(self.models.len()).map_err(|_| "model table full")?);
        self.models.push(Model {
            vertices: vertices.into(),
            indices: indices.into(),
            material,
        });
        Ok(id)
    }
    pub fn image(&self, id: ImageId) -> Option<&Image> {
        self.images.get(id.0 as usize)
    }
    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id.0 as usize)
    }
    pub fn model(&self, id: ModelId) -> Option<&Model> {
        self.models.get(id.0 as usize)
    }
    pub fn images(&self) -> &[Image] {
        &self.images
    }
    pub fn models(&self) -> &[Model] {
        &self.models
    }
}
