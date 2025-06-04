use std::fmt::Debug;
use uuid::Uuid;

pub trait ResourceObject: Debug + Send + Sync {
    fn get_id(&self) -> Uuid;
    fn get_name(&self) -> String;
    fn get_type(&self) -> String;
    fn get_filename(&self) -> Option<String> {
        None
    }
    fn get_fullpath(&self) -> Option<String> {
        None
    }
}
