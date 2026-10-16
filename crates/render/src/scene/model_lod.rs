use super::Refdef;
use qa_core::primitives::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelLodSettings {
    pub scale: f32,
    pub bias: i32,
}
impl Default for ModelLodSettings {
    fn default() -> Self {
        Self {
            scale: 5.0,
            bias: 0,
        }
    }
}
impl ModelLodSettings {
    /// Select from a nonempty captured level span using native Q3 projection.
    pub fn select(
        self,
        radius: f32,
        count: u32,
        origin: Vec3,
        view: &Refdef,
        projection_scales: [f32; 2],
    ) -> Option<u32> {
        let last = count.checked_sub(1)?;
        let mut level = 0;
        if count > 1 {
            let [_, vertical] = projection_scales;
            let distance = view.axes[0].dot(origin) - view.axes[0].dot(view.origin);
            let projected = if distance <= 0.0 {
                0.0
            } else {
                let radius = radius.abs();
                // ProjectRadius's symmetric projection, including its zero
                // products and conditional clamp rather than f32::min.
                let numerator = ((0.0 * 0.0 + radius * vertical) + -distance * 0.0) + 0.0;
                let denominator = ((0.0 * 0.0 + radius * 0.0) + -distance * -1.0) + 0.0;
                let projected = numerator / denominator;
                if projected > 1.0 { 1.0 } else { projected }
            };
            let scale = if self.scale > 20.0 { 20.0 } else { self.scale };
            let fraction = if projected != 0.0 {
                1.0 - projected * scale
            } else {
                0.0
            };
            level =
                (qa_core::math::native_integer(fraction * count as f32).max(0) as u32).min(last);
        }
        Some((i64::from(level) + i64::from(self.bias)).clamp(0, i64::from(last)) as u32)
    }
}

pub(crate) fn projection_scales(tangents: [f32; 2]) -> [f32; 2] {
    tangents.map(|value| 1.0 / value)
}
