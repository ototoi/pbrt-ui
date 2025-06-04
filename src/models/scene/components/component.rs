use std::{any::Any, fmt::Debug};

pub trait Component: Debug + Any {
    fn update(&mut self) {
        // Default implementation does nothing
    }
}
