use super::*;

impl<'directive> Params<'directive> {
    pub fn has(&self, name: &str) -> bool {
        self.directive.parameters.iter().any(|p| p.matches(name))
    }

    pub fn values(&self, name: &str) -> Vec<Value> {
        self.directive
            .parameters
            .iter()
            .filter(|p| p.matches(name))
            .flat_map(|p| p.values.clone())
            .collect()
    }

    pub fn float(&self, name: &str, default: f64) -> f64 {
        self.values(name)
            .into_iter()
            .filter_map(|value| value.as_float())
            .next()
            .unwrap_or(default)
    }

    pub fn floats(&self, name: &str) -> Vec<f64> {
        self.values(name)
            .into_iter()
            .filter_map(|value| value.as_float())
            .collect()
    }

    pub fn ints(&self, name: &str) -> Vec<i64> {
        self.floats(name)
            .into_iter()
            .map(|value| value.round() as i64)
            .collect()
    }

    pub fn int(&self, name: &str, default: i64) -> i64 {
        self.float(name, default as f64).round() as i64
    }

    pub fn string(&self, name: &str, default: &str) -> String {
        self.values(name)
            .into_iter()
            .filter_map(|value| value.as_text().map(str::to_string))
            .next()
            .unwrap_or_else(|| default.to_string())
    }

    /// Triples of numbers, e.g. `"point P"`. Values that are not numeric are
    /// skipped so a malformed list degrades instead of aborting the scene.
    pub fn points(&self, name: &str) -> Vec<[f64; 3]> {
        let numbers = self.floats(name);
        numbers
            .chunks(3)
            .map(|chunk| {
                let mut point = [0.0; 3];
                point[..chunk.len().min(3)].copy_from_slice(&chunk[..chunk.len().min(3)]);
                point
            })
            .filter(|_| numbers.len() >= 3)
            .collect()
    }

    pub fn uvs(&self, name: &str) -> Vec<[f64; 2]> {
        let numbers = self.floats(name);
        numbers
            .chunks(2)
            .map(|chunk| {
                let mut uv = [0.0; 2];
                uv[..chunk.len().min(2)].copy_from_slice(&chunk[..chunk.len().min(2)]);
                uv
            })
            .filter(|_| numbers.len() >= 2)
            .collect()
    }
}
