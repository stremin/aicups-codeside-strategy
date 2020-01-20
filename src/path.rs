use std::rc::Rc;

use model::Level;
use model::Properties;
use model::Tile;
use model::Vec2F64;
use std::hash::{Hash, Hasher};
use std::fmt::{Debug, Formatter, Error};

// константы для упрощения кода
const MAX_HORIZONTAL_SPEED: f64 = 10.0;
const FALL_SPEED: f64 = 10.0;
// properties.unit_max_horizontal_speed
const TICKS_PER_SECOND: f64 = 60.0; // properties.ticks_per_second

const HORIZONTAL_EPSILON: f64 = 0.049;
// точность достижения точки, должна гарантировать, что игрок целиком внутри квадрата по ширине
const VERTICAL_EPSILON: f64 = 0.2; // точность достижения точки, любое вертикальное движение должно попадать в этот диапазон (PadJump отдельно)

pub type TilePos = (isize, isize);

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum VerticalState {
    // walk, fall
    Default,
    // сколько квадратов еще можно лететь
    Jump(usize),
    // сколько квадратов еще можно лететь
    PadJump(usize),
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum MoveType {
    Start,
    Recover,
    MineSuicide,
    WalkLeft,
    WalkRight,
    LadderUp,
    LadderDown,
    Fall,
    FallLeft,
    FallRight,
    Fall2Left,
    Fall2Right,
    FallEdgeLeft,
    FallEdgeRight,
    Jump,
    JumpLeft,
    JumpRight,
    Jump2Left,
    Jump2Right,
    JumpStop,
    PadJumpLeft,
    PadJumpRight,
    PadJump2Left,
    PadJump2Right,
    PadJumpUp,
    PadJumpStop,
}

pub enum ControlResult {
    TargetReached,
    Recover,
    MoveAction(MoveAction),
}

#[derive(Clone)]
pub struct Move {
    pub typ: MoveType,
    pub pos1: TilePos,
    pub vertical_state1: VerticalState,
    pub pos2: TilePos,
    pub vertical_state2: VerticalState,
    pub ticks: i32,
    pub control: Rc<dyn Fn(Vec2F64, VerticalState) -> ControlResult>,
}

impl PartialEq for Move {
    fn eq(&self, other: &Self) -> bool {
        self.typ == other.typ && self.pos1 == other.pos1 && self.pos2 == other.pos2 &&
            self.vertical_state1 == other.vertical_state1 && self.vertical_state2 == other.vertical_state2 &&
            self.ticks == other.ticks
    }
}

impl Eq for Move {}

impl Hash for Move {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.typ, self.pos1, self.pos2, self.vertical_state1, self.vertical_state2, self.ticks).hash(state);
    }
}

impl Debug for Move {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "{:?} {{ pos1: {:?}, v_state1: {:?}, pos2: {:?}, v_state2: {:?}, ticks: {} }}",
               self.typ, self.pos1, self.vertical_state1, self.pos2, self.vertical_state2, self.ticks)
    }
}

#[derive(Debug)]
pub struct MoveAction {
    pub typ: MoveType,
    pub velocity: f64,
    pub jump: bool,
    pub jump_down: bool,
}

pub trait TileMovement {
    /// Проверка, что этот тип движения возможен из tile_pos.
    fn can_move(&self, tile_pos: TilePos, vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move>;
}

struct WalkSideMovement {
    delta: isize
}

impl TileMovement for WalkSideMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => {}
            VerticalState::Jump(_) => {} // можно прервать и пойти по платформе (запрыгнув снизу)
            VerticalState::PadJump(_) => return None
        };
        match level.tiles[tile_pos.0 as usize][(tile_pos.1 - 1) as usize] {
            Tile::Empty | Tile::JumpPad => if !unit_is_on_ladder(tile_pos, level) { return None; },
            _ => {}
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1);
        // падения с края будут обработаны в отдельном движении
        match level.tiles[new_pos.0 as usize][(new_pos.1 - 1) as usize] {
            Tile::Empty | Tile::JumpPad => if !unit_is_on_ladder(tile_pos, level) { return None; },
            _ => {}
        };
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Empty | Tile::Platform | Tile::Ladder => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::WalkLeft } else { MoveType::WalkRight };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_max_horizontal_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 != new_pos.1 || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("wsm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: false,
                    jump_down: false,
                })
            }),
        })
    }
}

fn target_reached(position: Vec2F64, target: TilePos, vertical_state: VerticalState) -> bool {
    let vertical_epsilon = match vertical_state {
        VerticalState::Default => VERTICAL_EPSILON,
        VerticalState::Jump(_) => VERTICAL_EPSILON,
        VerticalState::PadJump(_) => 2.0 * VERTICAL_EPSILON,
    };
    (position.x - (target.0 as f64 + 0.5)).abs() < HORIZONTAL_EPSILON && position.y >= target.1 as f64 &&
        (position.y - target.1 as f64) < vertical_epsilon
}

fn choose_horizontal_speed(pos: f64, target: f64) -> f64 {
    let delta = (pos - target).abs();
    let speed = if delta < MAX_HORIZONTAL_SPEED / TICKS_PER_SECOND {
        delta * TICKS_PER_SECOND
    } else {
        MAX_HORIZONTAL_SPEED
    };
    if pos > target {
        -speed
    } else {
        speed
    }
}

struct LadderMovement {
    vdelta: isize
}

impl TileMovement for LadderMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => {}
            VerticalState::Jump(_) => {} // можно прервать
            VerticalState::PadJump(_) => return None
        };
        // ход только для лестниц
        if !unit_is_on_ladder(tile_pos, level) {
            return None;
        }
        let new_pos = (tile_pos.0, tile_pos.1 + self.vdelta);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Empty | Tile::Platform | Tile::Ladder => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let speed = if self.vdelta > 0 { properties.unit_jump_speed } else { properties.unit_fall_speed };
        let delta = self.vdelta;
        let move_type = if delta < 0 { MoveType::LadderDown } else { MoveType::LadderUp };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || pos.0 != new_pos.0 {
                    println!("lm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: delta > 0,
                    jump_down: delta < 0,
                })
            }),
        })
    }
}

struct FallMovement {
    delta: isize
}

impl TileMovement for FallMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => {}
            VerticalState::Jump(_) => {} // можно прервать и упасть
            VerticalState::PadJump(_) => return None
        }
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 - 1);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        if self.delta != 0 && !check_possible_location((new_pos.0, new_pos.1 + 2), level, false, true) {
            return None;
        }
        // под игроком должен быть свободный квадрат, даже при падении со сдвигом
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 - 1), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Empty | Tile::Platform | Tile::Ladder => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::FallLeft } else { if self.delta > 0 { MoveType::FallRight } else { MoveType::Fall } };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_fall_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("fm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: false,
                    jump_down: true,
                })
            }),
        })
    }
}

// падение с боковым кубиком
// P##
// P.#
// ..#
// .##
struct Fall2Movement {
    delta: isize
}

impl TileMovement for Fall2Movement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => {}
            VerticalState::Jump(_) => {} // можно прервать и упасть
            VerticalState::PadJump(_) => return None
        }
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 - 1);
        // должен быть кубик сбоку
        match level.tiles[new_pos.0 as usize][(new_pos.1 + 2) as usize] {
            Tile::Wall => {}
            _ => return None,
        }
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        // под игроком должен быть свободный квадрат, даже при падении со сдвигом
        if !check_possible_location((tile_pos.0, tile_pos.1 - 1), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Empty | Tile::Platform | Tile::Ladder => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::Fall2Left } else { MoveType::Fall2Right };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_max_horizontal_speed + properties.ticks_per_second / properties.unit_fall_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("f2m position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: false,
                    jump_down: true,
                })
            }),
        })
    }
}

// падение с края
struct FallEdgeMovement {
    delta: isize
}

impl TileMovement for FallEdgeMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => {}
            VerticalState::Jump(_) => {} // можно прервать и упасть
            VerticalState::PadJump(_) => return None
        }
        // под нами должен быть пол
        match level.tiles[tile_pos.0 as usize][(tile_pos.1 - 1) as usize] {
            Tile::Wall | Tile::Platform => {}
            Tile::Empty | Tile::JumpPad | Tile::Ladder /* с лестницы слишком рано начинаем падать */ => return None,
        };
        // а рядом - пустая клетка, чтобы упасть
        let delta1 = if self.delta > 0 { 1 } else { -1 };
        match level.tiles[(tile_pos.0 + delta1) as usize][(tile_pos.1 - 1) as usize] {
            Tile::Empty => {}
            _ => return None,
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 - 1);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        if !check_possible_location((new_pos.0, new_pos.1 + 2), level, false, true) {
            return None;
        }
        if delta1 != self.delta {
            if !check_possible_location((tile_pos.0 + delta1, tile_pos.1 - 1), level, false, true) {
                return None;
            }
            if !check_possible_location((tile_pos.0 + delta1, tile_pos.1), level, false, true) {
                return None;
            }
            if !check_possible_location((tile_pos.0 + delta1, tile_pos.1 + 1), level, false, true) {
                return None;
            }
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Empty | Tile::Platform | Tile::Ladder => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::FallEdgeLeft } else { MoveType::FallEdgeRight };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_fall_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || (pos.0 != tile_pos.0 && pos.0 != tile_pos.0 + delta1 && pos.0 != new_pos.0) {
                    println!("fem position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: false,
                    jump_down: pos.0 == new_pos.0,
                })
            }),
        })
    }
}

struct JumpMovement {
    delta: isize
}

impl TileMovement for JumpMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => {
                match level.tiles[tile_pos.0 as usize][(tile_pos.1 - 1) as usize] {
                    Tile::Wall | Tile::Ladder | Tile::Platform => {}
                    Tile::Empty | Tile::JumpPad => return None,
                }
            }
            VerticalState::Jump(tiles) => if tiles < 1 { return None; },
            VerticalState::PadJump(_) => return None
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 + 1);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        // стена сбоку
        if self.delta != 0 && !check_possible_location((new_pos.0, new_pos.1 - 1), level, false, true) {
            return None;
        }
        // стена над нами
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 + 2), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Ladder => VerticalState::Default,
            Tile::Empty | Tile::Platform => {
                if level.tiles[new_pos.0 as usize][(new_pos.1 + 1) as usize] == Tile::Ladder {
                    // клетка выше тоже может быть лестницей и остановить прыжок
                    VerticalState::Default
                } else {
                    match vertical_state {
                        VerticalState::Default => VerticalState::Jump(jump_max_tiles(properties) - 1),
                        VerticalState::Jump(tiles) =>
                            if tiles > 1 { VerticalState::Jump(tiles - 1) } else { VerticalState::Default }
                        _ => return None
                    }
                }
            }
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::JumpLeft } else { if self.delta > 0 { MoveType::JumpRight } else { MoveType::Jump } };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_jump_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("jm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: true,
                    jump_down: false,
                })
            }),
        })
    }
}

// запрыгивание на кубик с ограниченным простанством сверху
// .#.
// ...
// P..
// P#.
struct Jump2Movement {
    delta: isize
}

impl TileMovement for Jump2Movement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => {
                match level.tiles[tile_pos.0 as usize][(tile_pos.1 - 1) as usize] {
                    Tile::Wall | Tile::Ladder | Tile::Platform => {}
                    Tile::Empty | Tile::JumpPad => return None,
                }
            }
            VerticalState::Jump(tiles) => if tiles < 1 { return None; },
            VerticalState::PadJump(_) => return None
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 + 1);
        // должен быть кубик сбоку
        match level.tiles[new_pos.0 as usize][(new_pos.1 - 1) as usize] {
            Tile::Wall => {}
            Tile::Platform => {}
            _ => return None,
        }
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        // стена над нами
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 + 2), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Ladder => VerticalState::Default,
            Tile::Empty | Tile::Platform => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::Jump2Left } else { MoveType::Jump2Right };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_max_horizontal_speed + properties.ticks_per_second / properties.unit_jump_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if (pos.1 != tile_pos.1 && pos.1 != new_pos.1) || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("j2m position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: pos.1 == tile_pos.1,
                    jump_down: false,
                })
            }),
        })
    }
}

// нужен, чтобы эффективно запрыгивать по платформам вверх, без него появляются лишние движения в сторону
struct JumpStopMovement {}

impl TileMovement for JumpStopMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, _level: &Level, _properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => return None,
            VerticalState::Jump(_) => {}
            VerticalState::PadJump(_) => return None
        };
        let new_pos = tile_pos;
        let new_vertical_state = VerticalState::Default;
        let move_type = MoveType::JumpStop;
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: 2, // ?
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if vertical_state == VerticalState::Default && target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 < tile_pos.1 || pos.1 > new_pos.1 || pos.0 != new_pos.0 {
                    println!("jsm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: position.y - tile_pos.1 as f64 <= FALL_SPEED / TICKS_PER_SECOND, // если сразу падать, то выпадаем из квадрата
                    jump_down: false,
                })
            }),
        })
    }
}

struct PadJumpMovement {
    delta: isize
}

impl TileMovement for PadJumpMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => return None,
            VerticalState::Jump(_) => return None,
            VerticalState::PadJump(tiles) => if tiles < 2 { return None; },
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 + 2);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        // стена сбоку
        if !check_possible_location((new_pos.0, new_pos.1 - 2), level, true, true) {
            return None;
        }
        if !check_possible_location((new_pos.0, new_pos.1 - 1), level, true, true) {
            return None;
        }
        // стена над нами
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 + 2), level, true, true) {
            return None;
        }
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 + 3), level, true, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Ladder => VerticalState::Default,
            Tile::Empty | Tile::Platform => {
                if level.tiles[new_pos.0 as usize][(new_pos.1 + 1) as usize] == Tile::Ladder {
                    // клетка выше тоже может быть лестницей и остановить прыжок
                    VerticalState::Default
                } else {
                    match vertical_state {
                        VerticalState::PadJump(tiles) =>
                            if tiles > 2 { VerticalState::PadJump(tiles - 2) } else { VerticalState::Default }
                        _ => return None
                    }
                }
            }
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::PadJumpLeft } else { MoveType::PadJumpRight };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (2.0 * properties.ticks_per_second / properties.jump_pad_jump_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 < tile_pos.1 || pos.1 > new_pos.1 || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("pjsm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: true,
                    jump_down: false,
                })
            }),
        })
    }
}


// запрыгивание на боковой кубик с батута
// .#
// ..
// P.
// P#
struct PadJump2Movement {
    delta: isize
}

impl TileMovement for PadJump2Movement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        assert_ne!(self.delta, 0);
        match vertical_state {
            VerticalState::Default => return None,
            VerticalState::Jump(_) => return None,
            VerticalState::PadJump(tiles) => if tiles < 1 { return None; },
        };
        let new_pos = (tile_pos.0 + self.delta, tile_pos.1 + 1);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        // должен быть кубик сбоку
        match level.tiles[new_pos.0 as usize][(new_pos.1 - 1) as usize] {
            Tile::Wall => {}
            _ => return None,
        }
        // должен быть кубик и сбоку сверху
        match level.tiles[new_pos.0 as usize][(new_pos.1 + 2) as usize] {
            Tile::Wall => {}
            _ => return None,
        }
        // стена над нами
        if self.delta != 0 && !check_possible_location((tile_pos.0, tile_pos.1 + 2), level, true, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Ladder => VerticalState::Default,
            Tile::Empty | Tile::Platform => VerticalState::Default,
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = if self.delta < 0 { MoveType::PadJump2Left } else { MoveType::PadJump2Right };
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.unit_max_horizontal_speed + properties.ticks_per_second / properties.jump_pad_jump_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 < tile_pos.1 || pos.1 > new_pos.1 || (pos.0 != tile_pos.0 && pos.0 != new_pos.0) {
                    println!("pjs2m position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: true,
                    jump_down: false,
                })
            }),
        })
    }
}

struct PadJumpUpMovement {}

impl TileMovement for PadJumpUpMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => return None,
            VerticalState::Jump(_) => return None,
            VerticalState::PadJump(tiles) => if tiles < 1 { return None; },
        };
        let new_pos = (tile_pos.0, tile_pos.1 + 1);
        if !check_possible_location(new_pos, level, false, false) {
            return None;
        }
        // верхняя часть игрока
        if !check_possible_location((new_pos.0, new_pos.1 + 1), level, false, true) {
            return None;
        }
        let new_vertical_state = match level.tiles[new_pos.0 as usize][new_pos.1 as usize] {
            Tile::Wall => return None,
            Tile::Ladder => VerticalState::Default,
            Tile::Empty | Tile::Platform => {
                if false && level.tiles[new_pos.0 as usize][(new_pos.1 + 1) as usize] == Tile::Ladder {
                    // клетка выше тоже может быть лестницей и остановить прыжок
                    VerticalState::Default
                } else {
                    match vertical_state {
                        VerticalState::PadJump(tiles) =>
                            if tiles > 1 { VerticalState::PadJump(tiles - 1) } else { VerticalState::Default }
                        _ => return None
                    }
                }
            }
            Tile::JumpPad => VerticalState::PadJump(pad_jump_max_tiles(properties)),
        };
        let move_type = MoveType::PadJumpUp;
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: (properties.ticks_per_second / properties.jump_pad_jump_speed).ceil() as i32,
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 < tile_pos.1 || pos.1 > new_pos.1 || pos.0 != new_pos.0 {
                    println!("pjum position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: true,
                    jump_down: false,
                })
            }),
        })
    }
}

struct PadJumpStopMovement {}

impl TileMovement for PadJumpStopMovement {
    fn can_move(&self, tile_pos: (isize, isize), vertical_state: VerticalState, level: &Level, _properties: &Properties) -> Option<Move> {
        match vertical_state {
            VerticalState::Default => return None,
            VerticalState::Jump(_) => return None,
            VerticalState::PadJump(_) => {}
        };
        let new_pos = tile_pos;
        // стенка над головой
        match level.tiles[new_pos.0 as usize][(new_pos.1 + 2) as usize] {
            Tile::Wall => {}
            _ => return None,
        }
        let new_vertical_state = VerticalState::Default;
        let move_type = MoveType::PadJumpStop;
        Some(Move {
            typ: move_type,
            pos1: tile_pos,
            pos2: new_pos,
            ticks: 3, // ?
            vertical_state1: vertical_state,
            vertical_state2: new_vertical_state,
            control: Rc::new(move |position: Vec2F64, vertical_state: VerticalState| {
                if vertical_state == VerticalState::Default && target_reached(position, new_pos, vertical_state) {
                    return ControlResult::TargetReached;
                }
                let pos = (position.x as isize, position.y as isize);
                if pos.1 < tile_pos.1 || pos.1 > new_pos.1 || pos.0 != new_pos.0 {
                    println!("pjstm position {:?} pos {:?} tile_pos {:?} new_pos {:?}", position, pos, tile_pos, new_pos);
                    return ControlResult::Recover;
                }
                ControlResult::MoveAction(MoveAction {
                    typ: move_type,
                    velocity: choose_horizontal_speed(position.x.clone(), new_pos.0 as f64 + 0.5),
                    jump: vertical_state != VerticalState::Default,
                    jump_down: vertical_state == VerticalState::Default,
                })
            }),
        })
    }
}

/// метод, чтобы в случае ошибки движения вернуться к какому-нибудь квадрату, из которого можно будет построить новый маршрут
pub fn get_recover_move() -> Move {
    Move {
        typ: MoveType::Recover,
        pos1: (-1, -1),
        vertical_state1: VerticalState::Default,
        pos2: (-1, -1),
        vertical_state2: VerticalState::Default,
        ticks: 1,
        control: Rc::new(|position: Vec2F64, vertical_state: VerticalState| {
            let pos = (position.x as isize, position.y as isize);
            if target_reached(position, pos, vertical_state) {
                println!("recover done at {:?} {:?}", position, vertical_state);
                return ControlResult::TargetReached;
            }
            println!("recover {:?} {:?}", position, vertical_state);
            ControlResult::MoveAction(MoveAction {
                typ: MoveType::Recover,
                velocity: choose_horizontal_speed(position.x.clone(), pos.0 as f64 + 0.5),
                jump: true,
                jump_down: false,
            })
        }),
    }
}

pub fn get_mine_suicide_move() -> Move {
    Move {
        typ: MoveType::MineSuicide,
        pos1: (-1, -1),
        vertical_state1: VerticalState::Default,
        pos2: (-1, -1),
        vertical_state2: VerticalState::Default,
        ticks: 1,
        control: Rc::new(|position: Vec2F64, vertical_state: VerticalState| {
            println!("mine suicide {:?} {:?}", position, vertical_state);
            ControlResult::MoveAction(MoveAction {
                typ: MoveType::MineSuicide,
                velocity: 0.0,
                jump: false,
                jump_down: false,
            })
        }),
    }
}

pub fn get_movements() -> Vec<Box<dyn TileMovement>> {
    vec![
        Box::new(WalkSideMovement { delta: -1 }), // left
        Box::new(WalkSideMovement { delta: 1 }), // right
        Box::new(LadderMovement { vdelta: 1 }), // up
        Box::new(LadderMovement { vdelta: -1 }), // down
        Box::new(FallMovement { delta: 0 }), // down
        Box::new(FallMovement { delta: -1 }), // down left
        Box::new(FallMovement { delta: 1 }), // down right
        Box::new(Fall2Movement { delta: -1 }), // down left
        Box::new(Fall2Movement { delta: 1 }), // down right
        Box::new(FallEdgeMovement { delta: -2 }), // down left
        Box::new(FallEdgeMovement { delta: -1 }), // down left
        Box::new(FallEdgeMovement { delta: 1 }), // down right
        Box::new(FallEdgeMovement { delta: 2 }), // down right
        Box::new(JumpMovement { delta: 0 }), // up
        Box::new(JumpMovement { delta: -1 }), // up left
        Box::new(JumpMovement { delta: 1 }), // up right
        Box::new(Jump2Movement { delta: -1 }), // up left
        Box::new(Jump2Movement { delta: 1 }), // up right
        Box::new(JumpStopMovement {}),
        Box::new(PadJumpMovement { delta: -1 }), // 2up left
        Box::new(PadJumpMovement { delta: 1 }), // 2up right
        Box::new(PadJump2Movement { delta: -1 }), // up left
        Box::new(PadJump2Movement { delta: 1 }), // up right
        Box::new(PadJumpUpMovement {}), // up
        Box::new(PadJumpStopMovement {}),
    ]
}

fn check_possible_location(pos: TilePos, level: &Level, avoid_ladders: bool, avoid_jump_pad: bool) -> bool {
    // проверяем, что в клетке нет ничего неподходящего
    if (pos.0 < 0) || (pos.0 >= level.width() as isize) {
        return false;
    }
    if (pos.1 < 0) || (pos.1 >= level.height() as isize) {
        return false;
    }
    match level.tiles[pos.0 as usize][pos.1 as usize] {
        Tile::Wall => return false,
        Tile::Ladder => if avoid_ladders { return false; },
        Tile::JumpPad => if avoid_jump_pad { return false; },
        _ => {}
    };
    true
}

fn jump_max_tiles(properties: &Properties) -> usize {
    (properties.unit_jump_time * properties.unit_jump_speed).floor() as usize
}

fn pad_jump_max_tiles(properties: &Properties) -> usize {
    (properties.jump_pad_jump_time * properties.jump_pad_jump_speed).floor() as usize
}

fn unit_is_on_ladder(pos: TilePos, level: &Level) -> bool {
    match level.tiles[pos.0 as usize][pos.1 as usize] {
        Tile::Ladder => return true,
        _ => {}
    };
    match level.tiles[pos.0 as usize][(pos.1 + 1) as usize] {
        Tile::Ladder => return true,
        _ => {}
    };
    false
}