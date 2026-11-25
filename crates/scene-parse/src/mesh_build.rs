//! Checked conversion from parser values to preview triangle arrays.
use crate::{error::SceneParseError, params::Directive};

mod triangles;
pub use triangles::{triangle_mesh, vertex_normals};
mod grids;
pub use grids::{bilinear_mesh, heightfield_mesh};
mod curves;
pub use curves::curve_mesh;
mod subdivision;
pub use subdivision::loop_subdiv;
mod quadrics;
pub use quadrics::{hyperboloid_mesh, paraboloid_mesh};
mod nurbs;
pub use nurbs::nurbs_mesh;

#[cfg(test)]
pub(crate) fn test_directive(source: &str) -> Directive {
    let mut lexer = crate::lexer::Lexer::new("test.pbrt", source);
    let mut reader = crate::params::DirectiveReader::new(&mut lexer);
    reader.read_directive().unwrap().unwrap()
}
