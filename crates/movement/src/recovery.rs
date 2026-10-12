//! Bounded native position recovery through the same caller-owned trace service.
use crate::physics::Step;
use qa_core::primitives::Vec3;
use qa_world::collision::TraceError;

impl Step<'_> {
    /// Native initial-snap order: Z, then Y, then X with offsets 0,-1,+1.
    /// It changes the rollback origin only after finding a valid full hull.
    pub(crate) fn initial_snap(&mut self, origin: Vec3, scale: f32) -> Result<Vec3, TraceError> {
        for z in [0.0, -1.0, 1.0] {
            for y in [0.0, -1.0, 1.0] {
                for x in [0.0, -1.0, 1.0] {
                    let mut candidate = Vec3([
                        origin.0[0] + x * scale,
                        origin.0[1] + y * scale,
                        origin.0[2] + z * scale,
                    ]);
                    if scale == 0.125 {
                        candidate.0 = candidate.0.map(qa_core::math::narrow_eighth);
                    }
                    if !self.trace(candidate, candidate)?.all_solid {
                        self.player.body.position = candidate;
                        return Ok(candidate);
                    }
                }
            }
        }
        Ok(origin)
    }
    pub(crate) fn recover_position(&mut self, origin: Vec3) -> Result<Option<Vec3>, TraceError> {
        let mins = self.player.body.mins;
        let maxs = self.player.body.maxs;
        if !self
            .trace_mask(origin, origin, mins, maxs, self.mask)?
            .start_solid
        {
            return Ok(Some(origin));
        }
        let mut candidates = [(0.0f32, Vec3::default()); 6];
        let mut count = 0;
        // Native face order is +Z,-Z,+X,-X,+Y,-Y.
        for (axis, positive) in [
            (2, true),
            (2, false),
            (0, true),
            (0, false),
            (1, true),
            (1, false),
        ] {
            let mut normal = Vec3::default();
            normal.0[axis] = if positive { 1.0 } else { -1.0 };
            let mut start = origin;
            start.0[axis] += if positive { maxs.0[axis] } else { mins.0[axis] };
            let mut face_mins = mins;
            let mut face_maxs = maxs;
            face_mins.0[axis] = 0.0;
            face_maxs.0[axis] = 0.0;
            let mut trace = self.trace_mask(start, start, face_mins, face_maxs, self.mask)?;
            let mut epsilon = None;
            if trace.start_solid {
                for tangent in 0..3 {
                    if tangent == axis {
                        continue;
                    }
                    let mut shifted = start;
                    shifted.0[tangent] += 1.0;
                    trace = self.trace_mask(shifted, shifted, face_mins, face_maxs, self.mask)?;
                    if !trace.start_solid {
                        start = shifted;
                        epsilon = Some((tangent, 1.0));
                        break;
                    }
                    shifted.0[tangent] -= 2.0;
                    trace = self.trace_mask(shifted, shifted, face_mins, face_maxs, self.mask)?;
                    if !trace.start_solid {
                        start = shifted;
                        epsilon = Some((tangent, -1.0));
                        break;
                    }
                }
            }
            if trace.start_solid {
                continue;
            }
            let mut opposite = origin;
            opposite.0[axis] += if positive { mins.0[axis] } else { maxs.0[axis] };
            if let Some((tangent, direction)) = epsilon {
                opposite.0[tangent] += direction;
            }
            trace = self.trace_mask(start, opposite, face_mins, face_maxs, self.mask)?;
            if trace.start_solid {
                continue;
            }
            let end = trace.end + normal * 0.125;
            let delta = end - opposite;
            let mut candidate = origin + delta;
            if let Some((tangent, direction)) = epsilon {
                candidate.0[tangent] += direction;
            }
            if self
                .trace_mask(candidate, candidate, mins, maxs, self.mask)?
                .start_solid
            {
                continue;
            }
            candidates[count] = (delta.dot(delta), candidate);
            count += 1;
        }
        if count == 0 {
            return Ok(None);
        }
        let mut best = candidates[0];
        // qsrc's sort endpoint excludes its last recorded candidate. Selecting
        // the first strict minimum of that prefix preserves the same result.
        for &candidate in &candidates[1..count.saturating_sub(1).max(1)] {
            if candidate.0 < best.0 {
                best = candidate;
            }
        }
        Ok(Some(best.1))
    }
}
