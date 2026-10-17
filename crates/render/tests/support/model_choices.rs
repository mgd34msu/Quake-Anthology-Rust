use qa_render::{
    Frame, ModelId, ModelPart, SceneEntity,
    scene::{ModelChoice, ModelSelector},
    shader::reference::ShaderRef,
};

pub fn lods(
    frame: &mut Frame,
    pose: SceneEntity,
    radius: Option<f32>,
    parts: impl IntoIterator<Item = impl IntoIterator<Item = Option<ModelPart>>>,
) -> bool {
    frame.add_model_choices(pose, |capture| {
        parts.into_iter().all(|levels| {
            let children = levels
                .into_iter()
                .map(|part| part.map_or_else(ModelChoice::missing, ModelChoice::part));
            let Some(choice) = capture.select(ModelSelector::Lod { radius }, children) else {
                return false;
            };
            capture.submit(choice)
        })
    })
}

pub fn skin(
    frame: &mut Frame,
    pose: SceneEntity,
    model: ModelId,
    group: u32,
    shaders: impl IntoIterator<Item = ShaderRef>,
) -> bool {
    members(
        frame,
        pose,
        model,
        ModelSelector::SkinGroup {
            timing_source: model,
            group,
        },
        shaders,
    )
}

pub fn sprite(
    frame: &mut Frame,
    pose: SceneEntity,
    model: ModelId,
    shaders: impl IntoIterator<Item = ShaderRef>,
) -> bool {
    members(
        frame,
        pose,
        model,
        ModelSelector::SpriteFrame { model },
        shaders,
    )
}

fn members(
    frame: &mut Frame,
    pose: SceneEntity,
    model: ModelId,
    selector: ModelSelector,
    shaders: impl IntoIterator<Item = ShaderRef>,
) -> bool {
    frame.add_model_choices(pose, |capture| {
        let children = shaders
            .into_iter()
            .map(|shader| ModelChoice::part(ModelPart { model, shader }));
        let Some(choice) = capture.select(selector, children) else {
            return false;
        };
        capture.submit(choice)
    })
}
