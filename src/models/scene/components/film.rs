use super::component::Component;
use crate::models::base::*;

#[derive(Debug, Clone)]
pub struct FilmComponent {
    pub props: PropertyMap,
}

impl FilmComponent {
    pub fn new(film_type: &str, props: &PropertyMap) -> Self {
        let mut props = props.clone();
        props.insert("string type", Property::from(film_type));
        FilmComponent { props: props }
    }
    pub fn get_keys(&self) -> Vec<(String, String)> {
        let keys = self.props.get_keys();
        return keys;
    }
}

impl Component for FilmComponent {}
