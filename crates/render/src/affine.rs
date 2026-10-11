pub(crate) fn fan(count: usize) -> impl Iterator<Item = [usize; 3]> {
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

#[cfg(test)]
#[path = "../tests/affine/oracle.rs"]
mod tests;
