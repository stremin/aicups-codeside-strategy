use crate::*;
#[derive(Clone, Debug, trans::Trans)]
pub struct Vec2F32 {
    pub x: f32,
    pub y: f32,
}

impl Vec2F32 {
    pub fn from64(pos: Vec2F64) -> Vec2F32 {
        Vec2F32 {
            x: pos.x as f32,
            y: pos.y as f32
        }
    }
}
