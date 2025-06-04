use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Weak;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Small {
    e: u8,
}

impl Small {
    fn size(&self) -> usize {
        std::mem::size_of::<Small>()
    }
}

impl Default for Small {
    fn default() -> Self {
        Small { e: 0 }
    }
}

#[derive(Debug, Clone)]
struct Big {
    v: Vec<u8>,
}
impl Big {
    fn size(&self) -> usize {
        std::mem::size_of::<Big>()
    }
}

impl Default for Big {
    fn default() -> Self {
        Big { v: vec![0; 256] }
    }
}

enum C {
    Small(Small),
    Big(Big),
}

impl C {
    fn size(&self) -> usize {
        match self {
            C::Small(_) => std::mem::size_of::<Small>(),
            C::Big(_) => std::mem::size_of::<Big>(),
        }
    }
}

trait IObject: Debug {}

#[derive(Debug, Clone)]
struct AObject {
    pub c: u8,
}
impl Default for AObject {
    fn default() -> Self {
        AObject { c: 0 }
    }
}

impl IObject for AObject {}

#[derive(Debug, Clone)]
struct BObject {
    pub c: u8,
}

impl Default for BObject {
    fn default() -> Self {
        BObject { c: 0 }
    }
}

impl IObject for BObject {}

trait Component: Any {}

#[derive(Debug, Clone, Default)]
struct Mesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

struct MeshComponent {
    pub mesh: Arc<Mesh>,
}

impl Component for MeshComponent {}

#[derive(Debug, Clone, Default)]
struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

struct TransformComponent {
    pub transform: Transform,
}

impl Component for TransformComponent {}

struct Node {
    pub id: Uuid,
    pub name: String,
    pub components: Vec<Box<dyn Any>>,
    pub parent: Option<Weak<Node>>,
    pub children: Vec<Arc<Node>>,
}

impl Node {
    fn new(name: &str) -> Self {
        let t = TransformComponent {
            transform: Transform::default(),
        };
        let components: Vec<Box<dyn Any>> = vec![Box::new(t)];
        Node {
            name: name.to_string(),
            components,
            id: Uuid::new_v4(),
            parent: None,
            children: Vec::new(),
        }
    }

    pub fn get_id(&self) -> Uuid {
        self.id
    }

    fn add_component<T: Component>(&mut self, component: T) {
        self.components.push(Box::new(component));
    }

    fn get_component<T: Component>(&self) -> Option<&T> {
        for component in self.components.iter() {
            if let Some(c) = component.downcast_ref::<T>() {
                return Some(c);
            }
        }
        None
    }

    fn get_component_mut<T: Component>(&mut self) -> Option<&mut T> {
        for component in self.components.iter_mut() {
            if let Some(c) = component.downcast_mut::<T>() {
                return Some(c);
            }
        }
        None
    }
}

#[test]
fn test_enum_size() {
    let small = Small::default();
    let big = Big::default();
    println!("size of small: {}", std::mem::size_of::<Small>());
    println!("size of big: {}", std::mem::size_of::<Big>());
    println!("size of C: {}", std::mem::size_of::<C>());
    assert!(std::mem::size_of::<C>() > std::mem::size_of::<Small>());
    assert!(std::mem::size_of::<C>() == std::mem::size_of::<Big>());
}

#[test]
fn test_trait_object() {
    let a = AObject::default();
    let b = BObject::default();
    let mut objects: Vec<Arc<dyn Any>> = Vec::new();
    objects.push(Arc::new(a));
    objects.push(Arc::new(b));
    for obj in objects.iter_mut() {
        if let Some(a) = obj.downcast_ref::<AObject>() {
            println!("AObject: {:?}", a);
        } else if let Some(b) = obj.downcast_ref::<BObject>() {
            println!("BObject: {:?}", b);
        } else {
            println!("Unknown");
        }
    }
}

#[test]
fn test_node_object() {
    let mesh = Mesh {
        vertices: vec![0.0, 0.0, 0.0],
        indices: vec![0, 1, 2],
    };
    let mesh = MeshComponent {
        mesh: Arc::new(mesh),
    };
    let mut a = Node::new("anode");
    a.add_component(mesh);

    let b = Node::new("bnode");
    let nodes = vec![a, b];
    for node in nodes.iter() {
        println!("Node: {:?}, {:?}", node.name, node.id);
        if let Some(c) = node.get_component::<TransformComponent>() {
            println!("TransformComponent: {:?}", c.transform);
        }
        if let Some(c) = node.get_component::<MeshComponent>() {
            println!("MeshComponent: {:?}", c.mesh);
        }
    }
}
