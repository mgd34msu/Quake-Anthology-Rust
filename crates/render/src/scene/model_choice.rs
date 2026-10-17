use super::{Frame, ModelPart, Refdef, SceneEntity, SceneModel, Span};
use crate::{Assets, ModelId};
use qa_core::primitives::RuleSetId;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelReplacementSettings {
    pub enabled: bool,
    pub distance: f32,
}
impl Default for ModelReplacementSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            distance: 0.0,
        }
    }
}
impl ModelReplacementSettings {
    fn select(self, entity: &SceneEntity, view: &Refdef) -> u32 {
        if !self.enabled {
            return 0;
        }
        let within = entity.no_model_lod
            || self.distance <= 0.0
            || qa_core::math::length(entity.origin - view.origin) <= self.distance;
        u32::from(within)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelSelector {
    Lod { radius: Option<f32> },
    Replacement { rules: RuleSetId },
    SkinGroup { timing_source: ModelId, group: u32 },
    SpriteFrame { model: ModelId },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkeletalFrameMap {
    Wrap,
    Elapsed { rate: f32 },
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ModelFramePose {
    pub current: u32,
    pub previous: u32,
    pub back_lerp: f32,
}
impl PartialEq for ModelFramePose {
    fn eq(&self, other: &Self) -> bool {
        self.current == other.current
            && self.previous == other.previous
            && self.back_lerp.to_bits() == other.back_lerp.to_bits()
    }
}
impl ModelFramePose {
    fn submitted(entity: &SceneEntity) -> Self {
        Self {
            current: entity.frame,
            previous: entity.old_frame,
            back_lerp: entity.back_lerp,
        }
    }
    fn single(frame: u32) -> Self {
        Self {
            current: frame,
            previous: frame,
            back_lerp: 0.0,
        }
    }
    fn mapped(
        self,
        map: SkeletalFrameMap,
        count: u32,
        entity: &SceneEntity,
        view: &Refdef,
    ) -> Option<Self> {
        if count == 0 {
            return None;
        }
        Some(match map {
            SkeletalFrameMap::Wrap => Self {
                current: self.current % count,
                previous: self.previous % count,
                ..self
            },
            SkeletalFrameMap::Elapsed { rate } => {
                let elapsed = ((view.model_time().seconds() + f64::from(entity.sync_base))
                    * f64::from(rate))
                .floor();
                if !elapsed.is_finite() {
                    return None;
                }
                Self::single(elapsed.rem_euclid(f64::from(count)) as u32)
            }
        })
    }
    pub(super) fn valid(self, count: u32) -> bool {
        self.current < count
            && self.previous < count
            && self.back_lerp.is_finite()
            && (0.0..=1.0).contains(&self.back_lerp)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum ChoiceNode {
    #[default]
    Missing,
    Part(ModelPart),
    Link(u32),
    Select {
        selector: ModelSelector,
        children: Span,
    },
    MapFrames {
        mapping: SkeletalFrameMap,
        child: u32,
    },
}

/// A stack leaf or an opaque node scoped to one atomic capture callback.
#[derive(Clone, Copy)]
pub struct ModelChoice<'capture> {
    node: ChoiceNode,
    invariant: PhantomData<&'capture mut &'capture ()>,
}
impl ModelChoice<'_> {
    pub fn part(part: ModelPart) -> Self {
        Self::new(ChoiceNode::Part(part))
    }
    pub fn missing() -> Self {
        Self::new(ChoiceNode::Missing)
    }
    fn new(node: ChoiceNode) -> Self {
        Self {
            node,
            invariant: PhantomData,
        }
    }
}

/// All children precede their parent in the existing packet-owned choice arena.
/// Opaque invariant handles cannot escape the generative capture callback.
pub struct ModelCapture<'capture> {
    frame: &'capture mut Frame,
    pose: SceneEntity,
    first: usize,
    failed: bool,
    invariant: PhantomData<&'capture mut &'capture ()>,
}
impl<'capture> ModelCapture<'capture> {
    fn append(&mut self, node: ChoiceNode) -> Option<u32> {
        let index = self.frame.list.model_choice_count;
        let Some(target) = self.frame.list.model_choices.get_mut(index) else {
            self.failed = true;
            return None;
        };
        if let ChoiceNode::Link(child) = node
            && ((child as usize) < self.first || child as usize >= index)
        {
            self.failed = true;
            return None;
        }
        *target = node;
        self.frame.list.model_choice_count += 1;
        Some(index as u32)
    }
    fn materialize(&mut self, choice: ModelChoice<'capture>) -> Option<u32> {
        if let ChoiceNode::Link(index) = choice.node {
            if (index as usize) < self.first || index as usize >= self.frame.list.model_choice_count
            {
                self.failed = true;
                return None;
            }
            Some(index)
        } else {
            self.append(choice.node)
        }
    }
    pub fn select(
        &mut self,
        selector: ModelSelector,
        children: impl IntoIterator<Item = ModelChoice<'capture>>,
    ) -> Option<ModelChoice<'capture>> {
        let mut children = children.into_iter();
        let Some(first) = children.next() else {
            self.failed = true;
            return None;
        };
        let Some(second) = children.next() else {
            if matches!(selector, ModelSelector::Replacement { .. }) {
                self.failed = true;
                return None;
            }
            return Some(first);
        };
        let first_index = self.frame.list.model_choice_count;
        self.append(first.node)?;
        self.append(second.node)?;
        for child in children {
            self.append(child.node)?;
        }
        let count = self.frame.list.model_choice_count - first_index;
        if matches!(selector, ModelSelector::Replacement { .. }) && count != 2 {
            self.failed = true;
            return None;
        }
        let index = self.append(ChoiceNode::Select {
            selector,
            children: Span {
                first: first_index as u32,
                count: count as u32,
            },
        })?;
        Some(ModelChoice::new(ChoiceNode::Link(index)))
    }
    pub fn map_frames(
        &mut self,
        mapping: SkeletalFrameMap,
        child: ModelChoice<'capture>,
    ) -> Option<ModelChoice<'capture>> {
        if matches!(mapping, SkeletalFrameMap::Elapsed { rate } if !rate.is_finite() || rate <= 0.0)
        {
            self.failed = true;
            return None;
        }
        let child = self.materialize(child)?;
        let index = self.append(ChoiceNode::MapFrames { mapping, child })?;
        Some(ModelChoice::new(ChoiceNode::Link(index)))
    }
    pub fn submit(&mut self, root: ModelChoice<'capture>) -> bool {
        let mut entity = self.pose;
        match root.node {
            ChoiceNode::Missing => return true,
            ChoiceNode::Part(part) => {
                entity.model = SceneModel::Mesh(part.model);
                entity.shader = entity.shader.or(Some(part.shader));
            }
            ChoiceNode::Link(_) => {
                let Some(root) = self.materialize(root) else {
                    return false;
                };
                entity.model = SceneModel::Choice { root };
            }
            ChoiceNode::Select { .. } | ChoiceNode::MapFrames { .. } => {
                self.failed = true;
                return false;
            }
        }
        if self.frame.add_entity_part(entity) {
            true
        } else {
            self.failed = true;
            false
        }
    }
}
impl Frame {
    pub fn add_model_choices(
        &mut self,
        pose: SceneEntity,
        build: impl for<'capture> FnOnce(&mut ModelCapture<'capture>) -> bool,
    ) -> bool {
        let checkpoint = self.checkpoint();
        let rejected = self.list.rejected;
        let first = self.list.model_choice_count;
        let success = {
            let mut capture = ModelCapture {
                frame: self,
                pose,
                first,
                failed: false,
                invariant: PhantomData,
            };
            build(&mut capture) && !capture.failed
        };
        if success {
            return true;
        }
        self.restore(checkpoint);
        if self.list.rejected == rejected {
            self.reject_submission();
        }
        false
    }
}

pub(super) struct SelectedPart {
    pub part: ModelPart,
    pub frames: ModelFramePose,
}
pub(super) fn choose(
    list: &super::CommandList,
    assets: &Assets,
    mut root: u32,
    entity: &SceneEntity,
    view: &Refdef,
    projection: Option<[f32; 2]>,
) -> Result<Option<SelectedPart>, ()> {
    let mut mapping = None;
    let mut sprite = None;
    loop {
        match list.model_choices[root as usize] {
            ChoiceNode::Missing => return Ok(None),
            ChoiceNode::Link(child) => root = child,
            ChoiceNode::MapFrames {
                mapping: next,
                child,
            } => {
                mapping = Some(next);
                root = child;
            }
            ChoiceNode::Select { selector, children } => {
                let selected = match selector {
                    ModelSelector::Lod { radius } => view
                        .model_lod
                        .select(
                            radius.ok_or(())?,
                            children.count,
                            entity.origin,
                            view,
                            projection.ok_or(())?,
                        )
                        .ok_or(())?,
                    ModelSelector::Replacement { rules } => {
                        view.model_replacements[rules as usize].select(entity, view)
                    }
                    ModelSelector::SkinGroup {
                        timing_source,
                        group,
                    } => {
                        let animation = assets
                            .model(timing_source)
                            .and_then(|model| model.animation())
                            .ok_or(())?;
                        if animation
                            .skin_groups()
                            .get(group as usize)
                            .ok_or(())?
                            .members
                            .count
                            != children.count
                        {
                            return Err(());
                        }
                        animation
                            .skin_member(
                                group,
                                view.model_time(),
                                entity.sync_base,
                                view.skin_timing,
                            )
                            .ok_or(())?
                    }
                    ModelSelector::SpriteFrame { model } => {
                        let loaded = assets.model(model).ok_or(())?;
                        if loaded.sprite.is_none() || loaded.frame_count() != children.count {
                            return Err(());
                        }
                        let frames = resolve_frames(loaded, entity, view).ok_or(())?;
                        sprite = Some((model, frames));
                        frames.current
                    }
                };
                root = children.first + selected;
            }
            ChoiceNode::Part(part) => {
                let model = assets.model(part.model).ok_or(())?;
                let frames = if let Some(mapping) = mapping {
                    if model.skeleton().is_none() || sprite.is_some() {
                        return Err(());
                    }
                    ModelFramePose::submitted(entity)
                        .mapped(mapping, model.frame_count(), entity, view)
                        .ok_or(())?
                } else if let Some((sprite_model, frames)) = sprite {
                    if part.model != sprite_model {
                        return Err(());
                    }
                    frames
                } else {
                    resolve_frames(model, entity, view).ok_or(())?
                };
                return Ok(Some(SelectedPart { part, frames }));
            }
        }
    }
}

pub(super) fn resolve_frames(
    model: &crate::assets::Model,
    entity: &SceneEntity,
    view: &Refdef,
) -> Option<ModelFramePose> {
    let frames = if let Some(sprite) = &model.sprite {
        let current = if let Some(animation) = model.animation() {
            animation.frame_pose(
                if entity.frame < model.native_frame_count() {
                    entity.frame
                } else {
                    0
                },
                view.model_time(),
                entity.sync_base,
                super::FrameTiming::Cumulative,
            )?
        } else if sprite.wrap_frames {
            entity.frame % model.frame_count()
        } else if entity.frame < model.frame_count() {
            entity.frame
        } else {
            0
        };
        ModelFramePose::single(current)
    } else if let Some(animation) = model.animation() {
        ModelFramePose {
            current: animation.frame_pose(
                entity.frame,
                view.model_time(),
                entity.sync_base,
                view.frame_timing,
            )?,
            previous: animation.frame_pose(
                entity.old_frame,
                view.model_time(),
                entity.sync_base,
                view.frame_timing,
            )?,
            back_lerp: entity.back_lerp,
        }
    } else {
        ModelFramePose::submitted(entity)
    };
    frames.valid(model.frame_count()).then_some(frames)
}
