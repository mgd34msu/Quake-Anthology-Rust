//! Native parent groups select already loaded frame-major poses and skins.
use qa_core::primitives::{Bounds, IndexSpan, ThinkTime};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedGroup {
    pub members: IndexSpan,
    /// Empty for a single member; otherwise cumulative positive endpoints.
    pub endpoints: IndexSpan,
    pub bounds: Bounds,
}

/// View-selected timing capability; neither geometry format nor backend chooses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameTiming {
    #[default]
    Cumulative,
    UniformFirst,
}

/// Native skin timing is independent of frame-group timing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkinTiming {
    #[default]
    Cumulative,
    LegacyFour,
}

/// One immutable source recipe, shared by cold registrations of the same mesh.
#[derive(Debug, PartialEq)]
pub struct ModelAnimation {
    frames: Box<[TimedGroup]>,
    skins: Box<[TimedGroup]>,
    endpoints: Box<[f32]>,
    sync: i32,
}
impl ModelAnimation {
    pub fn new(
        frame_groups: Box<[TimedGroup]>,
        skin_groups: Box<[TimedGroup]>,
        endpoints: Box<[f32]>,
        sync: i32,
        physical_frames: u32,
        physical_skins: u32,
    ) -> Result<Self, &'static str> {
        if physical_frames == 0
            || endpoints.len() > u32::MAX as usize
            || endpoints
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            || !groups_valid(&frame_groups, &endpoints, physical_frames)
            || !groups_valid(&skin_groups, &endpoints, physical_skins)
        {
            return Err("invalid model animation groups");
        }
        Ok(Self {
            frames: frame_groups,
            skins: skin_groups,
            endpoints,
            sync,
        })
    }

    pub fn frame_groups(&self) -> &[TimedGroup] {
        &self.frames
    }
    pub fn skin_groups(&self) -> &[TimedGroup] {
        &self.skins
    }
    pub fn endpoints(&self) -> &[f32] {
        &self.endpoints
    }
    pub fn sync(&self) -> i32 {
        self.sync
    }
    pub(super) fn physical_frames(&self) -> u32 {
        self.frames
            .last()
            .map_or(0, |group| group.members.first + group.members.count)
    }

    pub(crate) fn frame_pose(
        &self,
        frame: u32,
        time: ThinkTime,
        sync_base: f32,
        timing: FrameTiming,
    ) -> Option<u32> {
        let group = self.frames.get(frame as usize)?;
        if group.endpoints.count == 0
            || timing == FrameTiming::UniformFirst && group.members.count == 1
        {
            return Some(group.members.first);
        }
        let endpoints = &self.endpoints[group.endpoints.range()];
        let selected = match timing {
            FrameTiming::Cumulative => cumulative_member(endpoints, time, sync_base)?,
            FrameTiming::UniformFirst => {
                // GLQuake ignores syncbase and the remaining cumulative endpoints.
                let pose = checked_native_integer(time.seconds() / f64::from(endpoints[0]))?;
                usize::try_from(pose % i32::try_from(group.members.count).ok()?).ok()?
            }
        };
        u32::try_from(group.members.get(selected)?).ok()
    }

    /// Relative authored member; the packet owns this group's exact ShaderRefs.
    pub(crate) fn skin_member(
        &self,
        skin: u32,
        time: ThinkTime,
        sync_base: f32,
        timing: SkinTiming,
    ) -> Option<u32> {
        let group = self.skins.get(skin as usize)?;
        if group.members.count == 1 {
            return Some(0);
        }
        match timing {
            SkinTiming::Cumulative => u32::try_from(cumulative_member(
                &self.endpoints[group.endpoints.range()],
                time,
                sync_base,
            )?)
            .ok(),
            SkinTiming::LegacyFour => {
                // GLQuake overwrites each of four slots with successive j&3
                // members, then repeats short groups into unfilled slots.
                let slot = checked_native_integer(time.seconds() * 10.0)? as u32 & 3;
                let count = group.members.count;
                Some(if count < 4 {
                    slot % count
                } else {
                    count - 1 - ((count - 1 - slot) % 4)
                })
            }
        }
    }
}

fn cumulative_member(endpoints: &[f32], time: ThinkTime, sync_base: f32) -> Option<usize> {
    // r_alias: cl.time is double, but the sum is stored in float before
    // the quotient, product and subtraction are evaluated.
    if !sync_base.is_finite() {
        return None;
    }
    let time = (time.seconds() + f64::from(sync_base)) as f32;
    let full = *endpoints.last()?;
    let cycles = checked_native_integer(f64::from(time / full))?;
    let target = time - cycles as f32 * full;
    if !target.is_finite() {
        return None;
    }
    Some(
        endpoints[..endpoints.len() - 1]
            .iter()
            .position(|&endpoint| endpoint > target)
            .unwrap_or(endpoints.len() - 1),
    )
}

fn groups_valid(groups: &[TimedGroup], endpoints: &[f32], members: u32) -> bool {
    if groups.len() > u32::MAX as usize {
        return false;
    }
    let mut next = 0;
    for group in groups {
        if group.members.first != next
            || group.members.count == 0
            || !group.members.fits(members as usize)
            || !group.endpoints.fits(endpoints.len())
            || (group.endpoints.count == 0 && group.members.count != 1)
            || (group.endpoints.count != 0 && group.endpoints.count != group.members.count)
            || !group.bounds.is_valid()
        {
            return false;
        }
        next = group.members.first + group.members.count;
    }
    next == members
}

/// Native signed-int conversion is undefined outside this range; reject rather
/// than saturating and indexing a different pose.
fn checked_native_integer(value: f64) -> Option<i32> {
    let value = value.trunc();
    qa_core::math::fits_native_integer(value).then(|| qa_core::math::native_integer(value))
}
