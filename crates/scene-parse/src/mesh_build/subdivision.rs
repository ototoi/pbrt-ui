use super::*;

/// Applies bounded Loop subdivision to a triangular control mesh.
pub fn loop_subdiv(d: &Directive) -> Result<(Vec<[f64; 3]>, Vec<u32>), SceneParseError> {
    use std::collections::{BTreeMap, BTreeSet};
    const MAX_OUTPUT_TRIANGLES: u64 = 1_000_000;
    const MAX_LEVELS: i64 = 5;
    let params = d.params();
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    let raw_points = params.floats("P");
    let mut points = raw_points
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| [point[0], point[1], point[2]])
        .collect::<Vec<_>>();
    let raw_indices = params.floats("indices");
    let levels = params.int("levels", params.int("nlevels", 3));
    if !(0..=MAX_LEVELS).contains(&levels)
        || raw_points.is_empty()
        || !raw_points.len().is_multiple_of(3)
        || points.is_empty()
        || points.iter().flatten().any(|value| !value.is_finite())
        || raw_indices.is_empty()
        || !raw_indices.len().is_multiple_of(3)
        || raw_indices.iter().any(|value| {
            !value.is_finite()
                || value.fract() != 0.0
                || *value < 0.0
                || *value >= points.len() as f64
        })
    {
        return Err(fail(
            "loopsubdiv requires finite points, triangle indices and 0..=5 levels",
        ));
    }
    let final_triangles = (raw_indices.len() as u64 / 3)
        .checked_mul(4_u64.pow(levels as u32))
        .ok_or_else(|| fail("loopsubdiv output size overflow"))?;
    if final_triangles > MAX_OUTPUT_TRIANGLES {
        return Err(fail("loopsubdiv exceeds the preview triangle limit"));
    }
    if params.has("edgessharpness") || params.has("cornerweights") || params.has("cornerindices") {
        return Err(fail(
            "crease and corner weights are not supported by the viewport tessellator",
        ));
    }
    let mut triangles = raw_indices
        .into_iter()
        .map(|value| value as u32)
        .collect::<Vec<_>>();
    for face in triangles.as_chunks::<3>().0.iter() {
        if face[0] == face[1] || face[1] == face[2] || face[2] == face[0] {
            return Err(fail("loopsubdiv contains a degenerate triangle"));
        }
    }
    for _ in 0..levels {
        let mut edges = BTreeMap::<(u32, u32), Vec<u32>>::new();
        let mut neighbors = vec![BTreeSet::<u32>::new(); points.len()];
        let mut boundary_neighbors = vec![BTreeSet::<u32>::new(); points.len()];
        for face in triangles.as_chunks::<3>().0.iter() {
            for (a, b, opposite) in [
                (face[0], face[1], face[2]),
                (face[1], face[2], face[0]),
                (face[2], face[0], face[1]),
            ] {
                neighbors[a as usize].insert(b);
                neighbors[b as usize].insert(a);
                edges
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push(opposite);
            }
        }
        for (&(a, b), adjacent) in &edges {
            if adjacent.len() > 2 {
                return Err(fail("loopsubdiv contains a non-manifold edge"));
            }
            if adjacent.len() == 1 {
                boundary_neighbors[a as usize].insert(b);
                boundary_neighbors[b as usize].insert(a);
            }
        }
        let old_points = points;
        let mut refined = Vec::with_capacity(old_points.len() + edges.len());
        for (index, point) in old_points.iter().copied().enumerate() {
            let boundary = &boundary_neighbors[index];
            if !boundary.is_empty() && boundary.len() != 2 {
                // A bowtie vertex (two boundary fans touching) has no single
                // boundary curve to follow: keep it where it is, as a corner.
                refined.push(point);
            } else if !boundary.is_empty() {
                let mut boundary = boundary.iter();
                let a = old_points[*boundary.next().unwrap() as usize];
                let b = old_points[*boundary.next().unwrap() as usize];
                refined.push(std::array::from_fn(|axis| {
                    0.75 * point[axis] + 0.125 * (a[axis] + b[axis])
                }));
            } else {
                let ring = &neighbors[index];
                if ring.is_empty() {
                    return Err(fail("loopsubdiv contains an unused control point"));
                }
                let valence = ring.len() as f64;
                let beta = if ring.len() == 3 {
                    3.0 / 16.0
                } else {
                    3.0 / (8.0 * valence)
                };
                let sum = ring.iter().fold([0.0; 3], |mut sum, neighbor| {
                    for axis in 0..3 {
                        sum[axis] += old_points[*neighbor as usize][axis];
                    }
                    sum
                });
                refined.push(std::array::from_fn(|axis| {
                    (1.0 - valence * beta) * point[axis] + beta * sum[axis]
                }));
            }
        }
        let mut edge_vertices = BTreeMap::new();
        for (&edge, opposite) in &edges {
            let a = old_points[edge.0 as usize];
            let b = old_points[edge.1 as usize];
            let point = if opposite.len() == 1 {
                std::array::from_fn(|axis| 0.5 * (a[axis] + b[axis]))
            } else {
                let c = old_points[opposite[0] as usize];
                let d = old_points[opposite[1] as usize];
                std::array::from_fn(|axis| {
                    0.375 * (a[axis] + b[axis]) + 0.125 * (c[axis] + d[axis])
                })
            };
            let id = u32::try_from(refined.len())
                .map_err(|_| fail("loopsubdiv exceeds the vertex index range"))?;
            refined.push(point);
            edge_vertices.insert(edge, id);
        }
        let mut output = Vec::with_capacity(triangles.len() * 4);
        for face in triangles.as_chunks::<3>().0.iter() {
            let (a, b, c) = (face[0], face[1], face[2]);
            let (ab, bc, ca) = (
                edge_vertices[&(a.min(b), a.max(b))],
                edge_vertices[&(b.min(c), b.max(c))],
                edge_vertices[&(c.min(a), c.max(a))],
            );
            output.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
        }
        triangles = output;
        points = refined;
    }
    Ok((points, triangles))
}
