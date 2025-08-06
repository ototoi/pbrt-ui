#[derive(Debug, Clone, Copy)]
pub struct Vector2 {
    pub x: f32,
    pub y: f32,
}

impl Vector2 {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0 }
    }

    pub fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn length_squared(&self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    pub fn normalize(&self) -> Self {
        let length = self.length();
        if length > 0.0 {
            Self {
                x: self.x / length,
                y: self.y / length,
            }
        } else {
            Self { x: 0.0, y: 0.0 }
        }
    }
}

impl std::ops::Add<Vector2> for Vector2 {
    type Output = Vector2;
    #[inline]
    fn add(self, rhs: Vector2) -> Vector2 {
        return Vector2 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        };
    }
}

impl std::ops::Sub<Vector2> for Vector2 {
    type Output = Vector2;
    #[inline]
    fn sub(self, rhs: Vector2) -> Vector2 {
        return Vector2 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        };
    }
}

impl std::ops::Mul<f32> for Vector2 {
    type Output = Vector2;
    #[inline]
    fn mul(self, rhs: f32) -> Vector2 {
        return Vector2 {
            x: self.x * rhs,
            y: self.y * rhs,
        };
    }
}

impl std::ops::Mul<Vector2> for f32 {
    type Output = Vector2;
    #[inline]
    fn mul(self, rhs: Vector2) -> Vector2 {
        return Vector2 {
            x: self * rhs.x,
            y: self * rhs.y,
        };
    }
}

impl std::ops::AddAssign<Vector2> for Vector2 {
    #[inline]
    fn add_assign(&mut self, rhs: Vector2) {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

impl std::ops::SubAssign<Vector2> for Vector2 {
    #[inline]
    fn sub_assign(&mut self, rhs: Vector2) {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

impl std::ops::Neg for Vector2 {
    type Output = Vector2;
    #[inline]
    fn neg(self) -> Vector2 {
        return Vector2 {
            x: -self.x,
            y: -self.y,
        };
    }
}
