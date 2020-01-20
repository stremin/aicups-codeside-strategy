use crate::*;
#[derive(Clone, Debug, trans::Trans)]
pub struct Level {
    pub tiles: Vec<Vec<Tile>>,
}

impl Level {
    pub fn width(&self) -> usize {
        self.tiles.len()
    }

    pub fn height(&self, ) -> usize {
        self.tiles[0].len()
    }
}