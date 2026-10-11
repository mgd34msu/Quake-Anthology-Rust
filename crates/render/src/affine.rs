pub(crate) fn fan(count: usize) -> impl Iterator<Item = [usize; 3]> + Clone {
    (1..count.saturating_sub(1)).map(|at| [0, at, at + 1])
}

pub(crate) fn determinant(points: [[f64; 2]; 3]) -> f64 {
    (points[1][0] - points[0][0]) * (points[2][1] - points[0][1])
        - (points[2][0] - points[0][0]) * (points[1][1] - points[0][1])
}

/// Callers prepare their own coordinate offsets before widening to f64.
pub(crate) fn widest_fan_basis(
    count: usize,
    mut point: impl FnMut(usize) -> [f64; 2],
) -> Option<([usize; 3], f64)> {
    if count < 3 {
        return None;
    }
    let origin = point(0);
    let mut largest = 0.0_f64;
    let mut basis = None;
    for basis_indices in fan(count) {
        let area = determinant([origin, point(basis_indices[1]), point(basis_indices[2])]);
        if area.abs() > largest {
            largest = area.abs();
            basis = Some((basis_indices, area));
        }
    }
    basis
}

pub(crate) fn fit3(points: [[f64; 2]; 3], values: [f64; 3], area: f64) -> [f64; 3] {
    let ab = [points[1][0] - points[0][0], points[1][1] - points[0][1]];
    let ac = [points[2][0] - points[0][0], points[2][1] - points[0][1]];
    let vb = values[1] - values[0];
    let vc = values[2] - values[0];
    let x = (vb * ac[1] - vc * ab[1]) / area;
    let y = (ab[0] * vc - ac[0] * vb) / area;
    [x, y, values[0] - x * points[0][0] - y * points[0][1]]
}

pub(crate) fn fit(points: [[f64; 2]; 3], values: [f64; 3]) -> [f64; 3] {
    fit3(points, values, determinant(points))
}

/// Split a polygon in source order. Callers own storage and intersection validation.
pub(crate) fn split_polygon<T: Copy>(
    points: &[T],
    distance: impl Fn(usize, T) -> f32,
    epsilon: f32,
    mut front: impl FnMut(T) -> bool,
    mut back: impl FnMut(T) -> bool,
    intersect: impl Fn(T, T, f32) -> Option<T>,
) -> Option<()> {
    let side = |distance: f32| {
        if distance > epsilon {
            1
        } else if distance < -epsilon {
            -1
        } else {
            0
        }
    };
    for (index, &point) in points.iter().enumerate() {
        let next_index = (index + 1) % points.len();
        let next = points[next_index];
        let d = distance(index, point);
        let next_d = distance(next_index, next);
        let current_side = side(d);
        let next_side = side(next_d);
        if current_side >= 0 && !front(point) {
            return None;
        }
        if current_side <= 0 && !back(point) {
            return None;
        }
        if current_side == 0 || next_side == 0 || current_side == next_side {
            continue;
        }
        let crossing = intersect(point, next, d / (d - next_d))?;
        if !front(crossing) || !back(crossing) {
            return None;
        }
    }
    Some(())
}

#[cfg(test)]
#[path = "../tests/affine/oracle.rs"]
mod tests;
