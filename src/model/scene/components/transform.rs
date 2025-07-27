use super::component::Component;
use crate::model::base::*;

#[derive(Debug, Clone)]
pub struct TransformComponent {
    pub props: PropertyMap,
}

impl Default for TransformComponent {
    fn default() -> Self {
        let mut props = PropertyMap::new();
        props.insert("float position", Property::Floats(vec![0.0, 0.0, 0.0]));
        props.insert("float rotation", Property::Floats(vec![0.0, 0.0, 0.0]));
        props.insert("float scale", Property::Floats(vec![1.0, 1.0, 1.0]));
        TransformComponent { props }
    }
}

impl TransformComponent {
    pub fn new() -> Self {
        TransformComponent::default()
    }

    pub fn set_local_matrix(&mut self, m: Matrix4x4) {
        if let Some((position, rotation, scale)) = m.decompose(1e-6) {
            self.set_local_trs(position, rotation, scale);
        } else {
            panic!("Failed to decompose matrix");
        }
    }

    pub fn set_local_trs(&mut self, position: Vector3, rotation: Quaternion, scale: Vector3) {
        let (rx, ry, rz) = rotation.to_euler_angles(); //radians
        let (rx, ry, rz) = (rx.to_degrees(), ry.to_degrees(), rz.to_degrees()); //degree

        self.props.insert(
            "float position",
            Property::Floats(vec![position.x, position.y, position.z]),
        );
        self.props
            .insert("float rotation", Property::Floats(vec![rx, ry, rz]));
        self.props.insert(
            "float scale",
            Property::Floats(vec![scale.x, scale.y, scale.z]),
        );
    }

    pub fn get_local_trs(&self) -> (Vector3, Quaternion, Vector3) {
        let position = self.props.get_floats("float position");
        let rotation = self.props.get_floats("float rotation");
        let scale = self.props.get_floats("float scale");

        let position = Vector3::new(position[0], position[1], position[2]);
        let rotation = Quaternion::from_euler_angles(
            rotation[0].to_radians(),
            rotation[1].to_radians(),
            rotation[2].to_radians(),
        );
        let scale = Vector3::new(scale[0], scale[1], scale[2]);
        return (position, rotation, scale);
    }

    pub fn get_local_matrix(&self) -> Matrix4x4 {
        let (position, rotation, scale) = self.get_local_trs();

        let t = Matrix4x4::translate(position.x, position.y, position.z);
        let r = rotation.to_matrix();
        let s = Matrix4x4::scale(scale.x, scale.y, scale.z);
        return t * r * s;
    }

    pub fn get_keys(&self) -> Vec<(String, String)> {
        return self.props.get_keys();
    }
}

impl Component for TransformComponent {}
