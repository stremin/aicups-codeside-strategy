use crate::*;
#[derive(Clone, Copy, Debug, trans::Trans)]
pub struct Vec2F64 {
    pub x: f64,
    pub y: f64,
}

impl Vec2F64 {
    pub fn add(&self, other: Vec2F64) -> Vec2F64 {
        Vec2F64 {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    pub fn sub(&self, other: Vec2F64) -> Vec2F64 {
        Vec2F64 {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    pub fn mul(&self, v: f64) -> Vec2F64 {
        Vec2F64 {
            x: self.x * v,
            y: self.y * v,
        }
    }

    pub fn rotate(&self, angle: f64) -> Vec2F64 {
        Vec2F64 {
            x: self.x * angle.cos() - self.y * angle.sin(),
            y: self.x * angle.sin() + self.y * angle.cos(),
        }
    }
}