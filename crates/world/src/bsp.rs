//! Cold topology validation shared by collision and visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeError {
    Cycle,
    Depth,
}

/// Child ranges are checked at their file boundaries. Leaves count as zero;
/// node depth is one plus the largest child's depth, including disconnected DAGs.
pub(crate) fn node_depths(
    count: usize,
    children: impl Fn(usize) -> [i32; 2],
) -> Result<Box<[usize]>, NodeError> {
    let mut marks = vec![0u8; count];
    let mut depths = vec![0usize; count];
    let mut stack = Vec::with_capacity(count);
    for root in 0..count {
        if marks[root] == 2 {
            continue;
        }
        marks[root] = 1;
        stack.push((root, 0usize));
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if *next < 2 {
                let child = children(node)[*next];
                *next += 1;
                if child < 0 {
                    continue;
                }
                let child = child as usize;
                match marks[child] {
                    1 => return Err(NodeError::Cycle),
                    0 => {
                        marks[child] = 1;
                        stack.push((child, 0));
                    }
                    _ => {}
                }
            } else {
                let depth = children(node)
                    .into_iter()
                    .filter(|&child| child >= 0)
                    .map(|child| depths[child as usize])
                    .max()
                    .unwrap_or(0);
                depths[node] = depth.checked_add(1).ok_or(NodeError::Depth)?;
                marks[node] = 2;
                stack.pop();
            }
        }
    }
    Ok(depths.into_boxed_slice())
}

#[cfg(test)]
#[path = "../tests/bsp/oracle.rs"]
mod tests;
