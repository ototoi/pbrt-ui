#[derive(Debug, Clone)]
pub enum Property {
    Strings(Vec<String>),
    Floats(Vec<f32>),
    Ints(Vec<i32>),
    Bools(Vec<bool>),
}

impl From<f32> for Property {
    fn from(value: f32) -> Self {
        Property::Floats(vec![value])
    }
}
impl From<[f32; 2]> for Property {
    fn from(value: [f32; 2]) -> Self {
        Property::Floats(vec![value[0], value[1]])
    }
}
impl From<[f32; 3]> for Property {
    fn from(value: [f32; 3]) -> Self {
        Property::Floats(vec![value[0], value[1], value[2]])
    }
}
impl From<&[f32]> for Property {
    fn from(value: &[f32]) -> Self {
        Property::Floats(value.to_vec())
    }
}
impl From<Vec<f32>> for Property {
    fn from(value: Vec<f32>) -> Self {
        Property::Floats(value)
    }
}
impl From<i32> for Property {
    fn from(value: i32) -> Self {
        Property::Ints(vec![value])
    }
}
impl From<Vec<i32>> for Property {
    fn from(value: Vec<i32>) -> Self {
        Property::Ints(value)
    }
}
impl From<bool> for Property {
    fn from(value: bool) -> Self {
        Property::Bools(vec![value])
    }
}
impl From<Vec<bool>> for Property {
    fn from(value: Vec<bool>) -> Self {
        Property::Bools(value)
    }
}
impl From<String> for Property {
    fn from(value: String) -> Self {
        Property::Strings(vec![value])
    }
}
impl From<Vec<String>> for Property {
    fn from(value: Vec<String>) -> Self {
        Property::Strings(value)
    }
}
impl From<&str> for Property {
    fn from(value: &str) -> Self {
        Property::Strings(vec![value.to_string()])
    }
}

impl Property {
    pub fn len(&self) -> usize {
        match self {
            Property::Strings(v) => v.len(),
            Property::Floats(v) => v.len(),
            Property::Ints(v) => v.len(),
            Property::Bools(v) => v.len(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PropertyMap(pub Vec<(String, String, Property)>);

impl PropertyMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: &str, value: Property) {
        let (key_type, key_name) = Self::get_param_type(key);
        if let Some(item) = self.0.iter_mut().find(|(_, k, _)| k == key_name) {
            item.0 = key_type.to_string();
            //item.1 = key_name.to_string();
            item.2 = value;
        } else {
            self.0
                .push((key_type.to_string(), key_name.to_string(), value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Property> {
        let (_key_type, key_name) = Self::get_param_type(key);
        self.0
            .iter()
            .find(|(_, k, _)| k == key_name)
            .map(|(_, _, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Property> {
        let (_key_type, key_name) = Self::get_param_type(key);
        self.0
            .iter_mut()
            .find(|(_, k, _)| k == key_name)
            .map(|(_, _, v)| v)
    }

    pub fn get_keys(&self) -> Vec<(String, String)> {
        self.0
            .iter()
            .map(|(t, k, _)| (t.clone(), k.clone()))
            .collect()
    }

    pub fn entry(&self, key: &str) -> Option<(&String, &String, &Property)> {
        let (_key_type, key_name) = Self::get_param_type(key);
        let entry = self
            .0
            .iter()
            .find(|(_, k, _)| k == key_name)
            .map(|(a, b, c)| (a, b, c));
        return entry;
    }

    pub fn entry_mut(&mut self, key: &str) -> Option<(&mut String, &mut String, &mut Property)> {
        let (_key_type, key_name) = Self::get_param_type(key);
        let entry = self
            .0
            .iter_mut()
            .find(|(_, k, _)| k == key_name)
            .map(|(a, b, c)| (a, b, c));
        return entry;
    }

    //--------------------------------------------------//

    pub fn get_param_type(s: &str) -> (&str, &str) {
        let ss: Vec<&str> = s.split_ascii_whitespace().collect();
        if ss.len() == 2 {
            return (ss[0], ss[1]);
        } else if ss.len() == 1 {
            return ("", ss[0]);
        } else {
            return ("", s);
        }
    }

    pub fn get_key(key_type: &str, key_name: &str) -> String {
        if key_type.is_empty() {
            return key_name.to_string();
        }
        return format!("{} {}", key_type, key_name);
    }
}

pub type ParamSet = PropertyMap;

impl ParamSet {
    pub fn add_strings(&mut self, key: &str, values: &[String]) {
        self.insert(key, Property::Strings(values.to_vec()));
    }

    pub fn add_ints(&mut self, key: &str, values: &[i32]) {
        self.insert(key, Property::Ints(values.to_vec()));
    }

    pub fn add_floats(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }

    pub fn add_bools(&mut self, key: &str, values: &[bool]) {
        self.insert(key, Property::Bools(values.to_vec()));
    }

    pub fn add_string(&mut self, key: &str, value: &str) {
        self.insert(key, Property::Strings(vec![value.to_string()]));
    }

    //--------------------------------------------------//
    pub fn add_color(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }
    pub fn add_rgb(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }
    pub fn add_xyz(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }
    pub fn add_blackbody(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }
    pub fn add_point(&mut self, key: &str, values: &[f32]) {
        self.insert(key, Property::Floats(values.to_vec()));
    }
    //--------------------------------------------------//
    pub fn get_floats(&self, key: &str) -> Vec<f32> {
        if let Some(Property::Floats(v)) = self.get(key) {
            return v.clone();
        }
        return vec![];
    }
    pub fn get_ints(&self, key: &str) -> Vec<i32> {
        if let Some(Property::Ints(v)) = self.get(key) {
            return v.clone();
        }
        return vec![];
    }
    pub fn get_bools(&self, key: &str) -> Vec<bool> {
        if let Some(Property::Bools(v)) = self.get(key) {
            return v.clone();
        }
        return vec![];
    }
    pub fn get_strings(&self, key: &str) -> Vec<String> {
        if let Some(Property::Strings(v)) = self.get(key) {
            return v.clone();
        }
        return vec![];
    }

    pub fn get_points(&self, key: &str) -> Vec<f32> {
        if let Some(Property::Floats(v)) = self.get(key) {
            return v.clone();
        }
        return vec![];
    }
    //--------------------------------------------------//
    pub fn find_one_float(&self, key: &str) -> Option<f32> {
        if let Some(Property::Floats(v)) = self.get(key) {
            if v.len() > 0 {
                return Some(v[0]);
            }
        }
        return None;
    }
    pub fn find_one_int(&self, key: &str) -> Option<i32> {
        if let Some(Property::Ints(v)) = self.get(key) {
            if v.len() > 0 {
                return Some(v[0]);
            }
        }
        return None;
    }
    pub fn find_one_string(&self, key: &str) -> Option<String> {
        if let Some(Property::Strings(v)) = self.get(key) {
            if v.len() > 0 {
                return Some(v[0].clone());
            }
        }
        return None;
    }
    //--------------------------------------------------//
    pub fn remove(&mut self, key: &str) {
        let (_key_type, key_name) = Self::get_param_type(key);
        self.0.retain(|(_, k, _)| k != key_name);
    }
}
