use super::mesh_data::MeshData;
use crate::model::scene::Shape;

use std::io::*;

use ply_rs::parser;
use ply_rs::ply;

const VERTEX_P: u32 = 1;
const VERTEX_N: u32 = 2;
const VERTEX_UV: u32 = 8;

#[derive(Debug)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    nx: f32,
    ny: f32,
    nz: f32,
    u: f32,
    v: f32,
    flags: u32,
}

#[derive(Debug)]
struct Face {
    vertex_index: Vec<i32>,
    n: [f32; 3],
}

impl ply::PropertyAccess for Vertex {
    fn new() -> Self {
        Vertex {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            nx: 0.0,
            ny: 0.0,
            nz: 0.0,
            u: 0.0,
            v: 0.0,
            flags: 0,
        }
    }
    fn set_property(&mut self, key: String, property: ply::Property) {
        match property {
            ply::Property::Float(v) => match key.as_str() {
                "x" => {
                    self.x = v;
                    self.flags |= VERTEX_P;
                }
                "y" => {
                    self.y = v;
                    self.flags |= VERTEX_P;
                }
                "z" => {
                    self.z = v;
                    self.flags |= VERTEX_P;
                }
                "nx" => {
                    self.nx = v;
                    self.flags |= VERTEX_N;
                }
                "ny" => {
                    self.ny = v;
                    self.flags |= VERTEX_N;
                }
                "nz" => {
                    self.nz = v;
                    self.flags |= VERTEX_N;
                }
                "u" => {
                    self.u = v;
                    self.flags |= VERTEX_UV;
                }
                "v" => {
                    self.v = v;
                    self.flags |= VERTEX_UV;
                }
                "s" => {
                    self.u = v;
                    self.flags |= VERTEX_UV;
                }
                "t" => {
                    self.v = v;
                    self.flags |= VERTEX_UV;
                }
                "texture_u" => {
                    self.u = v;
                    self.flags |= VERTEX_UV;
                }
                "texture_v" => {
                    self.v = v;
                    self.flags |= VERTEX_UV;
                }
                "texture_s" => {
                    self.u = v;
                    self.flags |= VERTEX_UV;
                }
                "texture_t" => {
                    self.v = v;
                    self.flags |= VERTEX_UV;
                }
                k => panic!("Vertex: Unexpected key/value combination: key: {}", k),
            },
            _ => {
                panic!(
                    "Vertex: Unexpected key/value combination: key: {}, type: {:?}",
                    &key, property
                );
            }
        }
    }
}

// same thing for Face
impl ply::PropertyAccess for Face {
    fn new() -> Self {
        Face {
            vertex_index: Vec::new(),
            n: [0.0; 3],
        }
    }
    fn set_property(&mut self, key: String, property: ply::Property) {
        if key == "vertex_indices" || key == "vertex_index" {
            match property {
                ply::Property::ListInt(vec) => {
                    for i in 0..vec.len() {
                        self.vertex_index.push(vec[i]);
                    }
                }
                ply::Property::ListUInt(vec) => {
                    for i in 0..vec.len() {
                        self.vertex_index.push(vec[i] as i32);
                    }
                }
                _ => {
                    panic!(
                        "Face: Unexpected key/value combination: key: {}, type: {:?}",
                        &key, property
                    );
                }
            }
        } else if key == "nx" || key == "ny" || key == "nz" {
            match property {
                ply::Property::Float(v) => match key.as_str() {
                    "nx" => self.n[0] = v,
                    "ny" => self.n[1] = v,
                    "nz" => self.n[2] = v,
                    _ => {}
                },
                _ => {
                    panic!(
                        "Face: Unexpected key/value combination: key: {}, type: {:?}",
                        &key, property
                    );
                }
            }
        }
    }
}

fn create_reader(filanme: &str) -> Result<Box<dyn BufRead>> {
    let filanme = std::path::PathBuf::from(filanme);
    let extent = filanme
        .extension()
        .ok_or(Error::from(ErrorKind::InvalidData))?;
    let extent = extent.to_string_lossy().into_owned();
    if extent == "gz" {
        let f = std::fs::File::open(filanme)?;
        let reader = std::io::BufReader::new(f);
        let reader = flate2::read::GzDecoder::new(reader);
        let reader = std::io::BufReader::new(reader);
        return Ok(Box::new(reader));
    } else {
        let f = std::fs::File::open(filanme)?;
        let reader = std::io::BufReader::new(f);
        return Ok(Box::new(reader));
    }
}

pub fn load_from_ply(filename: &str) -> Result<MeshData> {
    let mut reader = create_reader(&filename)?;
    let vertex_parser = parser::Parser::<Vertex>::new();
    let face_parser = parser::Parser::<Face>::new();
    let header = vertex_parser.read_header(&mut reader).unwrap();

    let mut p = Vec::new();
    let mut vertex_indices: Vec<i32> = Vec::new();
    //let mut face_list = Vec::new();
    let mut n = Vec::new();
    let s = Vec::new();
    let mut uv = Vec::new();
    for (_name, element) in header.elements.iter() {
        //println!("{:?}", name);
        // we could also just parse them in sequence, but the file format might change
        match element.name.as_ref() {
            "vertex" => {
                let vertex_list =
                    vertex_parser.read_payload_for_element(&mut reader, element, &header)?;
                if !vertex_list.is_empty() {
                    let flags = vertex_list[0].flags;
                    if (flags & VERTEX_P) != 0 {
                        p.reserve(vertex_list.len());
                        for v in vertex_list.iter() {
                            p.push(v.x);
                            p.push(v.y);
                            p.push(v.z);
                        }
                    }
                    if (flags & VERTEX_N) != 0 {
                        n.reserve(vertex_list.len());
                        for v in vertex_list.iter() {
                            n.push(v.nx);
                            n.push(v.ny);
                            n.push(v.nz);
                        }
                    }
                    if (flags & VERTEX_UV) != 0 {
                        uv.reserve(vertex_list.len());
                        for v in vertex_list.iter() {
                            uv.push(v.u);
                            uv.push(v.v);
                        }
                    }
                }
            }
            "face" => {
                let face_list =
                    face_parser.read_payload_for_element(&mut reader, element, &header)?;

                vertex_indices.reserve(face_list.len() * 3);
                for face in face_list {
                    let n_vert = face.vertex_index.len();
                    match n_vert {
                        3 => {
                            for idx in face.vertex_index {
                                vertex_indices.push(idx);
                            }
                        }
                        4 => {
                            let i0 = face.vertex_index[0];
                            let i1 = face.vertex_index[1];
                            let i2 = face.vertex_index[2];
                            let i3 = face.vertex_index[3];
                            vertex_indices.push(i0);
                            vertex_indices.push(i1);
                            vertex_indices.push(i2);
                            vertex_indices.push(i3);
                            vertex_indices.push(i0);
                            vertex_indices.push(i2);
                        }
                        _ => {
                            return Err(Error::from(ErrorKind::InvalidData));
                        }
                    }
                }
            }
            _ => {
                //println!("Ignoring element: {}", name);
            }
        }
    }
    return Ok(MeshData {
        indices: vertex_indices,
        positions: p,
        tangents: s,
        normals: n,
        uvs: uv,
    });
}

pub fn create_mesh_data_from_plymesh(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    assert!(mesh_type == "plymesh", "Mesh type is not ply");
    if let Some(fullpath) = shape.as_property_map().find_one_string("string fullpath") {
        match load_from_ply(&fullpath) {
            Ok(mesh_data) => {
                return Some(mesh_data);
            }
            Err(e) => {
                log::error!("Error loading ply file: {}", e);
                return None;
            }
        }
    } else {
        return None;
    }
}
