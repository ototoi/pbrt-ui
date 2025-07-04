use super::mesh_data::MeshData;
use crate::model::scene::Shape;

use crate::model::base::Vector3;

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::Weak;

const TRI: [usize; 4] = [0, 1, 2, 0];
const NEXT: [usize; 4] = [1, 2, 0, 1];
const PREV: [usize; 4] = [2, 0, 1, 2];

// LoopSubdiv Local Structures
#[derive(Clone, Debug)]
struct SDVertex {
    p: Vector3,
    start_face: Option<Weak<RefCell<SDFace>>>,
    child: Option<Weak<RefCell<SDVertex>>>,
    regular: bool,
    boundary: bool,
}

#[derive(Clone, Debug)]
struct SDFace {
    v: [Option<Weak<RefCell<SDVertex>>>; 3],
    f: [Option<Weak<RefCell<SDFace>>>; 3],
    children: [Option<Weak<RefCell<SDFace>>>; 4],
}

#[derive(Clone, Debug)]
struct SDEdge {
    v: [Option<Weak<RefCell<SDVertex>>>; 2],
    f: [Option<Weak<RefCell<SDFace>>>; 2],
    f0edge_num: usize,
}

impl SDVertex {
    pub fn new(p: &Vector3) -> Self {
        SDVertex {
            p: *p,
            start_face: None,
            child: None,
            regular: false,
            boundary: false,
        }
    }

    pub fn get_start_face(&self) -> Option<Arc<RefCell<SDFace>>> {
        if let Some(f) = self.start_face.as_ref() {
            return f.upgrade();
        }
        return None;
    }

    pub fn valence(&self) -> u32 {
        let start_face = self.get_start_face().unwrap();
        if !self.boundary {
            // Compute valence of interior vertex
            let mut nf = 1;
            let mut f = start_face.clone();
            loop {
                let f2 = f.as_ref().borrow().next_face(self).unwrap();
                if f2.as_ptr() == start_face.as_ptr() {
                    break;
                }
                f = f2;
                nf += 1;
            }
            return nf;
        } else {
            // Compute valence of boundary vertex
            let mut nf = 1;
            let mut f = start_face.clone();
            loop {
                let f2 = f.as_ref().borrow().next_face(self);
                if let Some(ff) = f2 {
                    f = ff;
                } else {
                    break;
                }
                nf += 1;
            }
            let mut f = start_face.clone();
            loop {
                let f2 = f.as_ref().borrow().prev_face(self);
                if let Some(ff) = f2 {
                    f = ff;
                } else {
                    break;
                }
                nf += 1;
            }
            return nf + 1;
        }
    }

    pub fn one_ring(&self) -> Vec<Vector3> {
        let valence = self.valence();
        let mut points = Vec::with_capacity(valence as usize);
        let start_face = self.get_start_face().unwrap();

        if !self.boundary {
            // Get one-ring vertices for interior vertex
            let mut face = start_face.clone();
            loop {
                let p = face
                    .as_ref()
                    .borrow()
                    .next_vert(self)
                    .unwrap()
                    .as_ref()
                    .borrow()
                    .p;
                points.push(p);
                let f2 = face.as_ref().borrow().next_face(self).unwrap();
                if f2.as_ptr() == start_face.as_ptr() {
                    break;
                }
                face = f2;
            }
        } else {
            // Get one-ring vertices for boundary vertex
            let mut face = start_face.clone();
            loop {
                let f2 = face.as_ref().borrow().next_face(self);
                if let Some(ff) = f2 {
                    face = ff;
                } else {
                    break;
                }
            }
            {
                let p = face
                    .as_ref()
                    .borrow()
                    .next_vert(self)
                    .unwrap()
                    .as_ref()
                    .borrow()
                    .p;
                points.push(p);
            }
            loop {
                let p = face
                    .as_ref()
                    .borrow()
                    .prev_vert(self)
                    .unwrap()
                    .as_ref()
                    .borrow()
                    .p;
                points.push(p);

                let f2 = face.as_ref().borrow().prev_face(self);
                if let Some(ff) = f2 {
                    face = ff;
                } else {
                    break;
                }
            }
        }
        return points;
    }
}

impl SDFace {
    pub fn new() -> Self {
        SDFace {
            v: [None, None, None],
            f: [None, None, None],
            children: [None, None, None, None],
        }
    }

    fn vnum(&self, v: &SDVertex) -> i32 {
        let p_v = v as *const SDVertex;
        for i in 0..3 {
            let vi = self.v[i].as_ref().unwrap().upgrade().unwrap();
            let p_vi: *const SDVertex = vi.as_ptr();
            if p_v == p_vi {
                return i as i32;
            }
        }
        return -1;
    }

    pub fn next_face(&self, v: &SDVertex) -> Option<Arc<RefCell<SDFace>>> {
        let i = self.vnum(v);
        if i >= 0 {
            if let Some(f) = self.f[i as usize].as_ref() {
                return f.upgrade();
            }
        }
        return None;
    }

    pub fn prev_face(&self, v: &SDVertex) -> Option<Arc<RefCell<SDFace>>> {
        let i = self.vnum(v);
        if i >= 0 {
            if let Some(f) = self.f[PREV[i as usize]].as_ref() {
                return f.upgrade();
            }
        }
        return None;
    }

    pub fn next_vert(&self, v: &SDVertex) -> Option<Arc<RefCell<SDVertex>>> {
        let i = self.vnum(v);
        if i >= 0 {
            if let Some(f) = self.v[NEXT[i as usize]].as_ref() {
                return f.upgrade();
            }
        }
        return None;
    }

    pub fn prev_vert(&self, v: &SDVertex) -> Option<Arc<RefCell<SDVertex>>> {
        let i = self.vnum(v);
        if i >= 0 {
            if let Some(f) = self.v[PREV[i as usize]].as_ref() {
                return f.upgrade();
            }
        }
        return None;
    }

    pub fn other_vert(&self, v0: &SDVertex, v1: &SDVertex) -> Option<Arc<RefCell<SDVertex>>> {
        let p_v0: *const SDVertex = v0 as *const SDVertex;
        let p_v1: *const SDVertex = v1 as *const SDVertex;
        for i in 0..3 {
            let vi = self.v[i].as_ref().unwrap().upgrade().unwrap();
            let p_vi: *const SDVertex = vi.as_ref().as_ptr();
            if p_vi != p_v0 && p_vi != p_v1 {
                return Some(vi);
            }
        }
        return None;
    }

    pub fn get_child(&self, i: usize) -> Option<Arc<RefCell<SDFace>>> {
        if let Some(child) = self.children[i].as_ref() {
            return child.upgrade();
        }
        return None;
    }
}

impl SDEdge {
    pub fn new(v0: &Option<Weak<RefCell<SDVertex>>>, v1: &Option<Weak<RefCell<SDVertex>>>) -> Self {
        let mut v = [None, None];
        let p_v0 = v0.as_ref().unwrap().upgrade().unwrap().as_ptr();
        let p_v1 = v1.as_ref().unwrap().upgrade().unwrap().as_ptr();
        if p_v0 < p_v1 {
            v[0] = v0.clone();
            v[1] = v1.clone();
        } else {
            v[0] = v1.clone();
            v[1] = v0.clone();
        }

        SDEdge {
            v,
            f: [None, None],
            f0edge_num: 0,
        }
    }

    pub fn get_key(&self) -> String {
        let v0 = self.v[0].as_ref().unwrap();
        let v0 = v0.upgrade().unwrap();
        let v0 = v0.as_ptr();

        let v1 = self.v[1].as_ref().unwrap();
        let v1 = v1.upgrade().unwrap();
        let v1 = v1.as_ptr();

        //println!("{:?}_{:?}", v0, v1);
        assert!(v0 < v1);

        return format!("{:?}_{:?}", v0, v1);
    }

    pub fn get_position(&self, i: usize) -> Vector3 {
        let v = self.v[i].as_ref().unwrap().upgrade().unwrap();
        let p = v.as_ref().borrow().p;
        return p;
    }
}

fn weight_one_ring(vert: &SDVertex, beta: f32) -> Vector3 {
    // Put _vert_ one-ring in _p_ring_
    let p_ring = vert.one_ring();
    let valence = p_ring.len() as f32;
    let mut p = (1.0 - valence * beta) * vert.p;
    for pp in p_ring.iter() {
        p += beta * *pp;
    }
    return p;
}

fn weight_boundary(vert: &SDVertex, beta: f32) -> Vector3 {
    // Put _vert_ one-ring in _p_ring_
    let p_ring = vert.one_ring();
    let mut p = (1.0 - 2.0 * beta) * vert.p;
    p += beta * p_ring[0];
    p += beta * p_ring[p_ring.len() - 1];
    return p;
}

fn beta(valence: u32) -> f32 {
    if valence == 3 {
        return 3.0 / 16.0;
    } else {
        return 3.0 / (8.0 * valence as f32);
    }
}

fn loop_gamma(valence: u32) -> f32 {
    return 1.0 / (valence as f32 + 3.0 / (8.0 * beta(valence)));
}

fn loop_subdiv(levels: i32, indices: Vec<i32>, p: Vec<Vector3>) -> Option<MeshData> {
    let mut vertices = Vec::new();
    let mut faces = Vec::new();

    let n_vertices = p.len();
    for i in 0..n_vertices {
        let v = Arc::new(RefCell::new(SDVertex::new(&p[i])));
        vertices.push(v);
    }
    let n_indices = indices.len();
    let n_faces = n_indices / 3;
    for _ in 0..n_faces {
        let f = Arc::new(RefCell::new(SDFace::new()));
        faces.push(f);
    }

    // Set face to vertex pointers
    for i in 0..n_faces {
        for j in 0..3 {
            let v = vertices[indices[3 * i + j] as usize].clone();
            {
                let mut f = faces[i].as_ref().borrow_mut();
                let w = Arc::downgrade(&v);
                f.v[j] = Some(w);
            }
            {
                let mut v = v.as_ref().borrow_mut();
                v.start_face = Some(Arc::downgrade(&faces[i]));
            }
        }
    }

    // Set neighbor pointers in _faces_
    let mut edges = HashMap::new();
    for i in 0..n_faces {
        //let f = faces[i].borrow();
        for en in 0..3 {
            let v0 = TRI[en + 0];
            let v1 = TRI[en + 1];

            //let ff = faces[i].borrow();
            let e = Arc::new(RefCell::new(SDEdge::new(
                &faces[i].borrow().v[v0],
                &faces[i].borrow().v[v1],
            )));
            let key = e.borrow().get_key();
            if !edges.contains_key(&key) {
                //if let std::collections::hash_map::Entry::Vacant(e) = edges.entry(key) {
                // Handle new edge
                {
                    let mut e = e.as_ref().borrow_mut();
                    e.f[0] = Some(Arc::downgrade(&faces[i]));
                    e.f0edge_num = v0;
                }
                edges.insert(key, e);
            } else {
                // Handle previously seen edge
                {
                    let e = edges.get(&key).unwrap();
                    let e = e.as_ref().borrow_mut();
                    {
                        let f0 = e.f[0].as_ref().unwrap().upgrade().unwrap();
                        let mut f0 = f0.as_ref().borrow_mut();
                        f0.f[e.f0edge_num] = Some(Arc::downgrade(&faces[i]));
                    }
                    {
                        let mut f = faces[i].as_ref().borrow_mut();
                        let ef = e.f[0].as_ref().unwrap().clone();
                        f.f[en] = Some(ef);
                    }
                }
                edges.remove(&key);
            }
        }
    }

    // Finish vertex initialization
    for i in 0..n_vertices {
        let mut v = vertices[i].as_ref().borrow_mut();
        let start_face = v.get_start_face().unwrap();
        let mut face = start_face.clone();
        loop {
            let f2 = face.borrow().next_face(v.deref());
            if let Some(ff) = f2 {
                if start_face.as_ptr() == ff.as_ptr() {
                    break;
                }
                face = ff;
            } else {
                v.boundary = true;
                break;
            }
        }
        if !v.boundary && v.valence() == 6 {
            v.regular = true;
        } else if v.boundary && v.valence() == 4 {
            v.regular = true;
        } else {
            v.regular = false;
        }
    }

    // Refine _LoopSubdiv_ into triangles
    let mut f = faces.clone();
    let mut v = vertices.clone();
    for _ in 0..levels {
        // Update _f_ and _v_ for next level of subdivision
        let mut new_faces = Vec::new();
        let mut new_vertices = Vec::new();

        // Allocate next level of children in shape tree
        for vertex in v.iter() {
            let mut vv = vertex.as_ref().borrow_mut();
            let new_vertex = Arc::new(RefCell::new(SDVertex::new(&vv.p)));
            {
                let mut nv = new_vertex.as_ref().borrow_mut();
                nv.regular = vv.regular;
                nv.boundary = vv.boundary;
            }
            vv.child = Some(Arc::downgrade(&new_vertex));

            new_vertices.push(new_vertex);
        }

        for face in f.iter() {
            let mut ff = face.as_ref().borrow_mut();
            for k in 0..4 {
                let new_face = Arc::new(RefCell::new(SDFace::new()));
                ff.children[k] = Some(Arc::downgrade(&new_face));
                new_faces.push(new_face);
            }
        }

        // Update vertex positions and create new edge vertices

        // Update vertex positions for even vertices
        for vertex in v.iter() {
            let vv = vertex.as_ref().borrow_mut();
            if !vv.boundary {
                // Apply one-ring rule for even vertex
                if vv.regular {
                    let child = vv.child.as_ref().unwrap().upgrade().unwrap();
                    let mut child = child.as_ref().borrow_mut();
                    child.p = weight_one_ring(vv.deref(), 1.0 / 16.0);
                } else {
                    let child = vv.child.as_ref().unwrap().upgrade().unwrap();
                    let mut child = child.as_ref().borrow_mut();
                    child.p = weight_one_ring(vv.deref(), beta(vv.valence()));
                }
            } else {
                // Apply boundary rule for even vertex
                let child = vv.child.as_ref().unwrap().upgrade().unwrap();
                let mut child = child.as_ref().borrow_mut();
                child.p = weight_boundary(vv.deref(), 1.0 / 8.0);
            }
        }

        // Compute new odd edge vertices
        let mut edge_verts: HashMap<String, Arc<RefCell<SDVertex>>> = HashMap::new();
        for face in f.iter() {
            let face = face.as_ref().borrow();
            for k in 0..3 {
                // Compute odd vertex on _k_th edge
                let edge = SDEdge::new(&face.v[k], &face.v[NEXT[k]]);
                let key = edge.get_key();
                if !edge_verts.contains_key(&key) {
                    let p = Vector3::zero();
                    let vert = Arc::new(RefCell::new(SDVertex::new(&p)));
                    new_vertices.push(vert.clone());
                    let boundary = face.f[k].is_none();
                    let mut vv = vert.as_ref().borrow_mut();
                    vv.regular = true;
                    vv.boundary = boundary;
                    vv.start_face = face.children[3].clone();
                    if boundary {
                        let p0 = edge.get_position(0);
                        let p1 = edge.get_position(1);
                        vv.p = 0.5 * p0 + 0.5 * p1;
                    } else {
                        let p0 = edge.get_position(0);
                        let p1 = edge.get_position(1);
                        let v0 = edge.v[0].as_ref().unwrap().upgrade().unwrap();
                        let v1 = edge.v[1].as_ref().unwrap().upgrade().unwrap();
                        let po0 = face
                            .other_vert(v0.borrow().deref(), v1.borrow().deref())
                            .unwrap()
                            .borrow()
                            .p;
                        let po1 = face.f[k]
                            .as_ref()
                            .unwrap()
                            .upgrade()
                            .unwrap()
                            .borrow()
                            .other_vert(v0.borrow().deref(), v1.borrow().deref())
                            .unwrap()
                            .borrow()
                            .p;
                        vv.p = (3.0 / 8.0) * p0
                            + (3.0 / 8.0) * p1
                            + (1.0 / 8.0) * po0
                            + (1.0 / 8.0) * po1;
                    }
                    edge_verts.insert(key.clone(), vert.clone());
                }
            }
        }

        // Update new shape topology

        // Update even vertex face pointers
        for vertex in v.iter() {
            let vv = vertex.as_ref().borrow();
            let start_face = vv.get_start_face().unwrap();
            let vert_num = start_face.as_ref().borrow().vnum(vv.deref());
            let child = vv.child.as_ref().unwrap().upgrade().unwrap();
            let mut child = child.as_ref().borrow_mut();
            child.start_face = start_face.as_ref().borrow().children[vert_num as usize].clone();
        }

        // Update face neighbor pointers
        for face in f.iter() {
            let face = face.as_ref().borrow();
            for j in 0..3 {
                // Update children _f_ pointers for siblings
                {
                    let f3 = face.get_child(3).unwrap();
                    let mut f3 = f3.as_ref().borrow_mut();
                    f3.f[j] = face.children[NEXT[j]].clone();
                }
                {
                    let fj = face.get_child(j).unwrap();
                    let mut fj = fj.as_ref().borrow_mut();
                    fj.f[NEXT[j]] = face.children[3].clone();
                }
                // Update children _f_ pointers for neighbor children
                {
                    if let Some(f2) = face.f[j].as_ref() {
                        let fj = face.get_child(j).unwrap();
                        let mut fj = fj.as_ref().borrow_mut();
                        let f2 = f2.upgrade().unwrap();
                        let f2 = f2.as_ref().borrow();
                        let face_vj = face.v[j].as_ref().unwrap().upgrade().unwrap();
                        let face_vj = face_vj.borrow(); //face->v[j]
                        fj.f[j] = f2.children[f2.vnum(face_vj.deref()) as usize].clone();
                    }
                    if let Some(f2) = face.f[PREV[j]].as_ref() {
                        let fj = face.get_child(j).unwrap();
                        let mut fj = fj.as_ref().borrow_mut();
                        let f2 = f2.upgrade().unwrap();
                        let f2 = f2.as_ref().borrow();
                        let face_vj = face.v[j].as_ref().unwrap().upgrade().unwrap();
                        let face_vj = face_vj.borrow(); //face->v[j]
                        fj.f[PREV[j]] = f2.children[f2.vnum(face_vj.deref()) as usize].clone();
                    }
                }
            }
        }

        // Update face vertex pointers
        for face in f.iter() {
            let face = face.as_ref().borrow();
            for j in 0..3 {
                // Update child vertex pointer to new even vertex
                {
                    let fj = face.get_child(j).unwrap();
                    let mut fj = fj.as_ref().borrow_mut();
                    let face_vj = face.v[j].as_ref().unwrap().upgrade().unwrap();
                    let face_vj = face_vj.borrow();
                    fj.v[j] = face_vj.child.clone();
                }

                // Update child vertex pointer to new odd vertex
                {
                    let edge = SDEdge::new(&face.v[j], &face.v[NEXT[j]]);
                    let key = edge.get_key();
                    if let Some(vert) = edge_verts.get(&key) {
                        {
                            let f_child_j = face.get_child(j).unwrap();
                            let mut f_child_j = f_child_j.as_ref().borrow_mut();
                            f_child_j.v[NEXT[j]] = Some(Arc::downgrade(vert));
                        }
                        {
                            let f_child_j = face.get_child(NEXT[j]).unwrap();
                            let mut f_child_j = f_child_j.as_ref().borrow_mut();
                            f_child_j.v[j] = Some(Arc::downgrade(vert));
                        }
                        {
                            let f_child_j = face.get_child(3).unwrap();
                            let mut f_child_j = f_child_j.as_ref().borrow_mut();
                            f_child_j.v[j] = Some(Arc::downgrade(vert));
                        }
                    }
                }
            }
        }
        // Prepare for next level of subdivision
        f = new_faces;
        v = new_vertices;
    }

    // Push vertices to limit surface
    let mut p_limit = Vec::with_capacity(v.len());
    for vert in v.iter() {
        let mut vert = vert.as_ref().borrow_mut();
        if vert.boundary {
            vert.p = weight_boundary(vert.deref(), 1.0 / 5.0);
        } else {
            vert.p = weight_one_ring(vert.deref(), loop_gamma(vert.valence()));
        }
        p_limit.push(vert.p);
    }

    // Compute vertex tangents on limit surface
    let mut ns = Vec::with_capacity(v.len());
    for vertex in v.iter() {
        let vertex = vertex.as_ref().borrow();

        let mut s = Vector3::zero();
        let mut t = Vector3::zero();

        let p_ring = vertex.one_ring();
        let valence = p_ring.len();
        if !vertex.boundary {
            for j in 0..valence {
                s += f32::cos(2.0 * PI * j as f32 / valence as f32) * p_ring[j];
                t += f32::sin(2.0 * PI * j as f32 / valence as f32) * p_ring[j];
            }
        } else {
            // Compute tangents of boundary face
            s = p_ring[valence - 1] - p_ring[0];
            if valence == 2 {
                t = p_ring[0] + p_ring[1] - 2.0 * vertex.p;
            } else if valence == 3 {
                t = p_ring[1] - vertex.p;
            } else if valence == 4 {
                // regular
                t = -1.0 * p_ring[0]
                    + 2.0 * p_ring[1]
                    + 2.0 * p_ring[2]
                    + -1.0 * p_ring[3]
                    + -2.0 * vertex.p;
            } else {
                let theta = PI / (valence - 1) as f32;
                t = f32::sin(theta) * (p_ring[0] + p_ring[valence - 1]);
                for k in 1..(valence - 1) {
                    let wt = (2.0 * f32::cos(theta) - 2.0) * f32::sin(k as f32 * theta);
                    t += wt * p_ring[k];
                }
                t = -t;
            }
        }
        let n = Vector3::cross(&s, &t).normalize();
        ns.push(n);
    }

    // Create triangle shape from subdivision shape
    let ntris = f.len();
    let mut verts: Vec<i32> = Vec::with_capacity(ntris * 3);
    {
        let mut used_verts = HashMap::new();
        for i in 0..v.len() {
            let p_vi = v[i].as_ref().as_ptr();
            used_verts.insert(p_vi, i as u32);
        }
        for i in 0..ntris {
            let face = f[i].as_ref().borrow();
            for j in 0..3 {
                let vj = face.v[j].as_ref().unwrap().upgrade().unwrap();
                let p_vj = vj.as_ptr();
                let index = *used_verts.get(&p_vj).unwrap() as i32;
                verts.push(index);
            }
        }
    }
    let mut p = Vec::new();
    for v in p_limit.iter() {
        p.push(v.x);
        p.push(v.y);
        p.push(v.z);
    }

    let mut n = Vec::new();
    for v in ns.iter() {
        n.push(v.x);
        n.push(v.y);
        n.push(v.z);
    }

    let _uv = Vec::new();
    let _s = Vec::new();
    let mesh_data = MeshData {
        indices: verts,
        positions: p,
        normals: n,
        uvs: _uv,
        tangents: _s,
    };

    return Some(mesh_data);
}

pub fn create_mesh_data_from_loopsubdiv(shape: &Shape) -> Option<MeshData> {
    //println!("create_mesh_data_from_loopsubdiv");
    let mesh_type = shape.get_type();
    assert!(mesh_type == "loopsubdiv", "Mesh type is not loopsubdiv");

    let mut levels = 3;
    if let Some(l) = shape.as_property_map().find_one_int("nlevels") {
        levels = l;
    } else if let Some(l) = shape.as_property_map().find_one_int("levels") {
        levels = l;
    }

    let indices = shape.as_property_map().get_ints("indices");
    let p = shape.as_property_map().get_floats("P");
    if indices.len() == 0 || p.len() == 0 {
        return None;
    }
    let p = p
        .chunks(3)
        .map(|v| Vector3::new(v[0], v[1], v[2]))
        .collect::<Vec<_>>();
    return loop_subdiv(levels, indices, p);
}
