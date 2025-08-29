use super::plane_data::PlaneMesh;
use super::plane_data::PlaneOutline;

#[derive(Clone, Copy, Debug)]
struct Edge {
    start: usize,
    end: usize,
    twin: Option<usize>,
}

pub fn create_plane_outline_from_plane_mesh(mesh: &PlaneMesh) -> Option<PlaneOutline> {
    let mut edges: Vec<Edge> = Vec::new();
    let num_faces = mesh.indices.len() / 3;
    for i in 0..num_faces {
        let idx0 = mesh.indices[i * 3] as usize;
        let idx1 = mesh.indices[i * 3 + 1] as usize;
        let idx2 = mesh.indices[i * 3 + 2] as usize;

        edges.push(Edge {
            start: idx0,
            end: idx1,
            twin: None,
        });
        edges.push(Edge {
            start: idx1,
            end: idx2,
            twin: None,
        });
        edges.push(Edge {
            start: idx2,
            end: idx0,
            twin: None,
        });
    }
    for i in 0..edges.len() {
        if edges[i].twin.is_some() {
            continue;
        }
        for j in i + 1..edges.len() {
            if edges[j].twin.is_some() {
                continue;
            }
            if i != j && edges[i].start == edges[j].end && edges[i].end == edges[j].start {
                edges[i].twin = Some(j);
                edges[j].twin = Some(i);
                break;
            }
        }
    }
    let mut outline_edges: Vec<Edge> = Vec::new();
    for edge in edges.iter() {
        if edge.twin.is_none() {
            outline_edges.push(edge.clone());
        }
    }
    let mut outline_loops: Vec<Edge> = Vec::new();
    if !outline_edges.is_empty() {
        let mut current_edge = outline_edges[0];
        loop {
            if let Some(next) = outline_edges.iter().find(|e| e.start == current_edge.end) {
                if !outline_loops.is_empty()
                    && next.start == outline_loops[0].start
                    && next.end == outline_loops[0].end
                {
                    break;
                } else {
                    outline_loops.push(next.clone());
                    current_edge = next.clone();
                }
            } else {
                return None;
            }
        }
    }

    if !outline_loops.is_empty() {
        let mut outline_positions: Vec<f32> = Vec::new();
        for edge in outline_loops.iter() {
            let start_idx = edge.start * 3;
            outline_positions.push(mesh.positions[start_idx]);
            outline_positions.push(mesh.positions[start_idx + 1]);
            outline_positions.push(mesh.positions[start_idx + 2]);
        }
        //println!("outline_positions.len(): {}", outline_positions.len());
        return Some(PlaneOutline {
            positions: outline_positions,
        });
    }

    return None;
}
