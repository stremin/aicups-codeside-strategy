use std::collections::{HashSet, VecDeque};
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::rc::Rc;
use std::time::Instant;

use model::{Bullet, ColorF32, Game, Level, LootBox, Properties, Tile, Unit, Vec2F32, Vec2F64, WeaponType};

use crate::fnv_hash::FnvHashMap;
use crate::path::{ControlResult, get_movements, get_recover_move, Move, MoveAction, MoveType, TilePos, VerticalState, get_mine_suicide_move};
use crate::rand::Random;
use crate::non_nan_f64::NonNan;

const USE_MINE_SUICIDE: bool = true;

pub struct MyStrategy {
    total_time: u128,
    rand: Random,
    paths: Paths,
    distance_map: FnvHashMap<TilePos, FnvHashMap<(TilePos, VerticalState), i32>>,
    unit1_data: UnitData,
    unit2_data: UnitData,
    last_enemy_state: FnvHashMap<i32, Unit>,
}

#[derive(Debug)]
struct UnitData {
    id: i32,
    move_: Option<Move>,
    path: Vec<Move>,
    path_start_tick: i32,
    last_position: Vec2F64,
}

impl MyStrategy {
    pub fn new() -> Self {
        Self {
            total_time: 0,
            rand: Random::new(98754),
            paths: Paths {
                outgoing: FnvHashMap::default(),
                incoming: FnvHashMap::default(),
            },
            distance_map: FnvHashMap::default(),
            unit1_data: UnitData {
                id: -1,
                move_: None,
                path: Vec::new(),
                path_start_tick: 0,
                last_position: Vec2F64 { x: -1.0, y: -1.0 },
            },
            unit2_data: UnitData {
                id: -1,
                move_: None,
                path: Vec::new(),
                path_start_tick: 0,
                last_position: Vec2F64 { x: -1.0, y: -1.0 },
            },
            last_enemy_state: FnvHashMap::default(),
        }
    }
    pub fn get_action(
        &mut self,
        unit: &model::Unit,
        game: &model::Game,
        debug: &mut crate::Debug,
    ) -> model::UnitAction {
        let now = Instant::now();
        println!("tick {}, unit {}, {} ms:  position {:?} vertical_state {:?} on_ground {} jump_state {:?}",
                 game.current_tick, unit.id, self.total_time,
                 unit.position, MyStrategy::get_vertical_state(unit, game), unit.on_ground, unit.jump_state);
        if game.current_tick == 0 {
            if self.unit1_data.id == -1 {
                self.unit1_data.id = unit.id;
            } else if self.unit2_data.id == -1 {
                self.unit2_data.id = unit.id;
            }
            for unit2 in &game.units {
                if unit2.player_id != unit.player_id {
                    self.last_enemy_state.insert(unit2.id, unit2.clone());
                }
            }
        }
        let (unit_data, other_unit_data) =
            if unit.id == self.unit1_data.id {
                (&mut self.unit1_data, &self.unit2_data)
            } else {
                (&mut self.unit2_data, &self.unit1_data)
            };

        {
            // расчитаем пути
            // на первом тике это будет большой расчет, а потом только иногда и маленький
            let pos = tile_pos(unit.position.clone());
            let vertical_state = MyStrategy::get_vertical_state(unit, game);
            if self.paths.outgoing.get(&(pos, vertical_state)).is_none() {
                self.paths.update_paths(pos, vertical_state, &game);
            }
        }
//        self.draw_all_movements2(debug);

        let paths = &self.paths;

        // на первом тике построим карту расстояний до предметов, новых потом уже не будет
        if game.current_tick == 0 {
            let start2 = Instant::now();
            for loot in &game.loot_boxes {
                let pos = tile_pos(loot.position);
                let map = MyStrategy::build_all_paths(pos, paths);
                self.distance_map.insert(pos, map);
            }
            println!("build_all_paths count {}, {} ms", self.distance_map.len(), start2.elapsed().as_millis());
        }

        let distance_map = &self.distance_map;

        let suicide_damage = unit.mines * game.properties.mine_explosion_params.damage +
            if unit.weapon.is_some() && unit.weapon.as_ref().unwrap().typ == WeaponType::RocketLauncher { unit.weapon.as_ref().unwrap().params.explosion.as_ref().unwrap().damage } else { 0 };

        if unit_data.move_.is_some() && unit_data.last_position.x == unit.position.x && unit_data.last_position.y == unit.position.y &&
            unit_data.move_.as_ref().unwrap().typ != MoveType::MineSuicide {
            // застряли, скорее всего на другом игроке, нужен новый план
            println!("got stuck");
            unit_data.path.clear();
            unit_data.move_ = None;
        }
        unit_data.last_position = unit.position.clone();

        let mut move_action: Option<MoveAction> = None;
        let new_move = unit_data.move_.as_ref().and_then(|mr| {
            match (mr.control)(unit.position, MyStrategy::get_vertical_state(unit, game)) {
                ControlResult::TargetReached => None,
                ControlResult::Recover => {
                    let recover_move = get_recover_move();
                    move_action = match (recover_move.control)(unit.position, MyStrategy::get_vertical_state(unit, game)) {
                        ControlResult::TargetReached => None,
                        ControlResult::Recover => unreachable!(),
                        ControlResult::MoveAction(move_action2) => Some(move_action2),
                    };
                    Some(recover_move)
                }
                ControlResult::MoveAction(move_action2) => {
                    move_action = Some(move_action2);
                    None
                }
            }
        });
        if new_move.is_some() {
            if new_move.as_ref().unwrap().typ == MoveType::Recover {
                unit_data.path.clear();
            }
            unit_data.move_ = new_move;
        }
        if unit_data.move_.is_some() && unit_data.move_.as_ref().unwrap().typ == MoveType::MineSuicide &&
            !MyStrategy::suicide_is_effective(unit.position, unit.player_id, suicide_damage, &game) {
            // выжили, или противник убежал
            unit_data.path.clear();
            unit_data.move_ = None;
        }
        if move_action.is_none() {
            let old_path = unit_data.path.clone();

            unit_data.path.clear();
            unit_data.move_ = None;

            // mine suicide
            if USE_MINE_SUICIDE && unit.weapon.is_some() && unit.mines > 0 && MyStrategy::can_plant_mine(tile_pos(unit.position), &game.level) &&
                unit.weapon.as_ref().unwrap().fire_timer.unwrap_or(0.0) <= 1.0 / game.properties.ticks_per_second &&
                MyStrategy::suicide_is_effective(unit.position, unit.player_id, suicide_damage, &game) {
                unit_data.move_ = Some(get_mine_suicide_move());
                move_action = match (unit_data.move_.as_ref().unwrap().control)(unit.position, MyStrategy::get_vertical_state(unit, game)) {
                    ControlResult::TargetReached => unreachable!(),
                    ControlResult::Recover => unreachable!(),
                    ControlResult::MoveAction(move_action2) => Some(move_action2),
                };
            } else {
                let pos = tile_pos(unit.position.clone());
                let vertical_state = MyStrategy::get_vertical_state(unit, game);

                let mut loot_map = FnvHashMap::default();
                for loot in &game.loot_boxes {
                    loot_map.insert(tile_pos(loot.position), loot);
                }

                fn is_weapon(loot: &LootBox) -> bool { if let model::Item::Weapon { .. } = loot.item { true } else { false } }
                fn is_health(loot: &LootBox) -> bool { if let model::Item::HealthPack { .. } = loot.item { true } else { false } }
                fn is_mine(loot: &LootBox) -> bool { if let model::Item::Mine { .. } = loot.item { true } else { false } }

                let max_ticks = 60;
                let mut best_cost = std::f64::MAX;
                let mut best_old = std::f64::MAX;
                let mut best_used_old = false;

                let mut enemy_distance_map = FnvHashMap::default();
                game.units
                    .iter()
                    .filter(|unit2| unit2.player_id != unit.player_id)
                    .for_each(|unit2| {
                        let pos = tile_pos(unit2.position);
                        let map = MyStrategy::build_all_paths(pos, &paths);
                        enemy_distance_map.insert(unit2.id, map);
                    });

                let need_weapon = unit.weapon.is_none() && game.loot_boxes.iter().any(|loot| is_weapon(loot));
                let need_health = unit.health < game.properties.unit_max_health && game.loot_boxes.iter().any(|loot| is_health(loot));
                let simple_target_distance_map = {
                    if need_weapon {
                        game.loot_boxes.iter()
                            .filter(|loot| is_weapon(loot))
                            .map(|loot| tile_pos(loot.position))
                            .min_by_key(|pos2| distance_map[pos2].get(&(pos, vertical_state)).unwrap_or(&std::i32::MAX))
                            .map(|pos2| &distance_map[&pos2])
                            .unwrap()
                    } else if need_health {
                        game.loot_boxes.iter()
                            .filter(|loot| is_health(loot))
                            .map(|loot| tile_pos(loot.position))
                            .min_by_key(|pos2| distance_map[pos2].get(&(pos, vertical_state)).unwrap_or(&std::i32::MAX))
                            .map(|pos2| &distance_map[&pos2])
                            .unwrap()
                    } else {
                        game.units.iter()
                            .filter(|unit2| unit2.player_id != unit.player_id)
                            .min_by_key(|unit2| enemy_distance_map[&unit2.id].get(&(pos, vertical_state)).unwrap_or(&std::i32::MAX))
                            .map(|unit2| &enemy_distance_map[&unit2.id])
                            .unwrap()
                    }
                };

                let very_long_dist = 1000000;

                let path_count = 100;
                'path_loop: for i in 0..path_count {
                    let bullets = Bullets::new(game);
                    let mut bullets_state = BulletsState::new();
                    let empty_vec = vec![];
                    let mut path;
                    let mut ticks;
                    let used_old;
                    if i == path_count - 1 {
                        // проверим быстрый путь
                        path = vec![MyStrategy::make_start_node(pos, vertical_state)];
                        ticks = 0;
                        used_old = false;
                        while ticks < max_ticks {
                            let mov = path.last().unwrap();
                            if let Some(dist) = simple_target_distance_map.get(&(mov.pos2, mov.vertical_state2)) {
                                if *dist == 0 {
                                    break;
                                }
                            }
                            if let Some(mov2) = paths.outgoing.get(&(mov.pos2, mov.vertical_state2)).unwrap_or(&empty_vec).iter()
                                .min_by_key(|mov| mov.ticks + simple_target_distance_map.get(&(mov.pos2, mov.vertical_state2)).unwrap_or(&very_long_dist)) {
                                path.push(mov2.clone());
                                ticks += mov2.ticks;
                            } else {
                                break;
                            }
                        }
                    } else {
                        if i < 5 && old_path.len() > 1 && old_path[1].pos2 == pos && old_path[1].vertical_state2 == vertical_state {
                            // проверим несколько вариантов на основе старого пути
//                            println!("i old");
                            path = old_path[1..].to_vec();
                            ticks = path[1..].iter().map(|mov| mov.ticks).sum();
                            used_old = true;
                        } else {
                            path = vec![MyStrategy::make_start_node(pos, vertical_state)];
                            ticks = 0;
                            used_old = false;
                        }
                        // построим случайный путь
                        while ticks < max_ticks {
                            let mov = path.last().unwrap();
                            let movs = paths.outgoing.get(&(mov.pos2, mov.vertical_state2)).unwrap_or(&empty_vec);
                            if movs.is_empty() {
                                break;
                            }
                            let mov2 = &movs[self.rand.next_u32_bounded(movs.len() as u32) as usize];
                            path.push(mov2.clone());
                            ticks += mov2.ticks;
                        }
                    }

                    // оценим повреждения на пути
                    let mut ticks = 0;
                    let mut damage = 0;
                    for mov in &path[1..] {
                        // проверим, что не столкнемся с другими игроками
                        for mov_tick in 0..mov.ticks {
                            let tick = ticks + mov_tick;
                            let unit_position = MyStrategy::get_unit_position_at_tick(&unit, &path, tick).0;
                            for unit2 in &game.units {
                                if unit2.id == unit.id {
                                    continue;
                                }
                                let unit2_position =
                                    if unit2.player_id == unit.player_id {
                                        if other_unit_data.id == -1 {
                                            unit2.position.clone()
                                        } else {
                                            assert_eq!(unit2.id, other_unit_data.id);
                                            let tick2 = tick + game.current_tick - other_unit_data.path_start_tick;
                                            MyStrategy::get_unit_position_at_tick(&unit2, &other_unit_data.path, tick2).0
                                        }
                                    } else {
                                        unit2.position.clone()
                                    };
                                if (unit_position.x - unit2_position.x).abs() < game.properties.unit_size.x / 2.0 &&
                                    (unit_position.y - unit2_position.y).abs() < game.properties.unit_size.y / 2.0 {
//                                    println!("collision {} {:?} {:?}", tick, unit_position, unit2_position);
                                    continue 'path_loop;
                                }
                            }
                        }

                        let (new_damage, new_bullets_state) = MyStrategy::calc_damage(mov, ticks, unit.id, &bullets_state, &bullets, &game);
                        ticks += mov.ticks;
                        damage += new_damage;
                        bullets_state = new_bullets_state;
                    }

                    // нарисовать путь
//                    for i in 0..path.len() - 1 {
//                        let mov1 = &path[i];
//                        let mov2 = &path[i + 1];
//
//                        debug.draw(model::CustomData::Line {
//                            p1: Vec2F32 { x: mov1.pos2.0 as f32 + 0.5, y: mov1.pos2.1 as f32 + 0.5 },
//                            p2: Vec2F32 { x: mov2.pos2.0 as f32 + 0.5, y: mov2.pos2.1 as f32 + 0.5 },
//                            color: ColorF32 {
//                                r: 0.0,
//                                g: 1.0,
//                                b: 1.0,
//                                a: 0.5,
//                            },
//                            width: 0.1,
//                        });
//                    }
//                    debug.draw(model::CustomData::Rect {
//                        pos: Vec2F32 { x: path.last().unwrap().pos2.0 as f32 + 0.5, y: path.last().unwrap().pos2.1 as f32 + 0.5 },
//                        color: ColorF32 {
//                            r: 0.0,
//                            g: 1.0,
//                            b: 0.0,
//                            a: 0.5,
//                        },
//                        size: Vec2F32 { x: 0.3, y: 0.3 },
//                    });

                    let last_mov = path.last().unwrap();
                    let damage_cost = damage as f64 * 100.0;
                    let cost =
                        if need_weapon {
                            if path.iter()
                                .filter_map(|mov| loot_map.get(&mov.pos2))
                                .any(|loot| is_weapon(loot)) {
                                // встретили по дороге
                                damage_cost
                            } else {
                                // определяем расстояние ближайшего предмета
                                let min_ticks = *game
                                    .loot_boxes
                                    .iter()
                                    .filter(|loot| is_weapon(*loot))
                                    .filter_map(|loot| distance_map.get(&tile_pos(loot.position))
                                        .and_then(|map| map.get(&(last_mov.pos2, last_mov.vertical_state2))))
                                    .min()
                                    .unwrap_or(&very_long_dist);

                                damage_cost + min_ticks as f64
                            }
                        } else if need_health {
                            if path.iter()
                                .filter_map(|mov| loot_map.get(&mov.pos2))
                                .any(|loot| is_health(loot)) {
                                // встретили по дороге
                                damage_cost
                            } else {
                                // определяем расстояние ближайшего предмета
                                let min_ticks = *game
                                    .loot_boxes
                                    .iter()
                                    .filter(|loot| is_health(*loot))
                                    .filter_map(|loot| distance_map.get(&tile_pos(loot.position))
                                        .and_then(|map| map.get(&(last_mov.pos2, last_mov.vertical_state2))))
                                    .min()
                                    .unwrap_or(&very_long_dist);

                                damage_cost + min_ticks as f64
                            }
                        } else if false /*USE_MINE_SUICIDE*/ && unit.mines < 2 && game.loot_boxes.iter().filter(|loot| is_mine(loot)).count() >= 2 - unit.mines as usize {
                            if path.iter()
                                .filter_map(|mov| loot_map.get(&mov.pos2))
                                .any(|loot| is_mine(loot)) {
                                // встретили по дороге
                                damage_cost
                            } else {
                                // определяем расстояние ближайшего предмета
                                let min_ticks = *game
                                    .loot_boxes
                                    .iter()
                                    .filter(|loot| is_mine(*loot))
                                    .filter_map(|loot| distance_map.get(&tile_pos(loot.position))
                                        .and_then(|map| map.get(&(last_mov.pos2, last_mov.vertical_state2))))
                                    .min()
                                    .unwrap_or(&very_long_dist);

                                damage_cost + min_ticks as f64
                            }
                        } else {
                            let min_dist_to_enemy = path.iter()
                                .filter_map(|mov| enemy_distance_map.iter().filter_map(|(id, map)| {
                                    let enemy = game.units
                                        .iter()
                                        .find(|unit2| unit2.id == *id)
                                        .unwrap();
                                    let fire_timer_ticks = enemy.weapon.as_ref().and_then(|weapon| weapon.fire_timer).unwrap_or(0.0) * game.properties.ticks_per_second;
                                    map.get(&(mov.pos2, mov.vertical_state2)).map(|dist| dist + fire_timer_ticks as i32)
                                }).min())
                                .min()
                                .unwrap_or(very_long_dist) as f64;

                            let min_dist = 50.0;
                            let cost = damage_cost + (min_dist_to_enemy - min_dist).abs() * 10.0;
                            cost
                        };

//                println!("i {} cost {} {}", i, cost, if cost < best_cost {"***"} else {""});

                    if cost < best_cost {
                        best_cost = cost;
                        best_used_old = used_old;
                        if path.len() > 1 {
                            unit_data.path = path;
                            unit_data.path_start_tick = game.current_tick;
                        }
                    }
                    if used_old {
                        best_old = cost;
                    }
                }
                println!("used old {}, best old {}", best_used_old, best_old);

                if !unit_data.path.is_empty() {
                    unit_data.move_ = Some(unit_data.path[1].clone());
                    move_action = match (unit_data.move_.as_ref().unwrap().control)(unit.position, MyStrategy::get_vertical_state(unit, game)) {
                        ControlResult::TargetReached => unreachable!(),
                        ControlResult::Recover => unreachable!(),
                        ControlResult::MoveAction(move_action2) => Some(move_action2),
                    };

                    println!("next path {} {:?} {:?}", best_cost, unit_data.path.get(1), unit_data.path.get(2));
                } else {
                    println!("no path from {:?} {:?}", pos, vertical_state);
                }
            }
        }

        // все достижимые точки
//        MyStrategy::draw_all_movements(unit, game, debug);
        // первые движения
        game.units.iter().for_each(|unit| MyStrategy::draw_first_movements(unit, game, debug));

        if !unit_data.path.is_empty() {
            // нарисовать путь
            for i in 0..unit_data.path.len() - 1 {
                let mov1 = &unit_data.path[i];
                let mov2 = &unit_data.path[i + 1];

                debug.draw(model::CustomData::Line {
                    p1: Vec2F32 { x: mov1.pos2.0 as f32 + 0.5, y: mov1.pos2.1 as f32 + 0.5 },
                    p2: Vec2F32 { x: mov2.pos2.0 as f32 + 0.5, y: mov2.pos2.1 as f32 + 0.5 },
                    color: ColorF32 {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.5,
                    },
                    width: 0.1,
                });
            }
        }

        // показ попаданий в нас
        {
            let bullets = Bullets::new(game);
            for bullet in &bullets.bullets {
                debug.draw(model::CustomData::Line {
                    p1: Vec2F32::from64(bullet.0.position),
                    p2: Vec2F32::from64(bullet.1),
                    color: ColorF32 {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.1,
                    },
                    width: 0.1,
                });
            }

            let mut bullets_state = BulletsState::new();

            for tick in 0..60 {
                if !bullets.need_test(&bullets_state) {
                    break;
                }
                let path_tick = tick + (game.current_tick - unit_data.path_start_tick);
                let (position1, position2) = MyStrategy::get_unit_position_at_tick(&unit, &unit_data.path, path_tick);

                let micro_ticks = 100;
                for micro_tick in 0..micro_ticks {
                    let t = micro_tick as f64 / micro_ticks as f64;
                    let position = position1.add(position2.sub(position1).mul(t));

                    let (bullet_hits, explosion_hits, new_bullets_state) =
                        (&bullets).test(position, unit.id, tick as f64 + t, &bullets_state, &game.properties);
                    bullets_state = new_bullets_state;

                    if bullet_hits.is_some() || explosion_hits.is_some() {
//                        println!("tick {} mtick {} {:?}", tick, micro_tick, explosion_hits);

                        let unit_color = ColorF32 {
                            r: 1.0,
                            g: 1.0,
                            b: 0.0,
                            a: 0.5,
                        };
                        let hit_color = ColorF32 {
                            r: 1.0,
                            g: 0.0,
                            b: 0.8,
                            a: 1.0,
                        };

                        MyStrategy::draw_unit(position, unit_color, 0.1, &game.properties, debug);

                        for explosion in explosion_hits.unwrap_or_default() {
                            let bullet_position = explosion.0;
                            let radius = explosion.2;
                            debug.draw(model::CustomData::Line {
                                p1: Vec2F32 { x: (bullet_position.x - radius) as f32, y: (bullet_position.y - radius) as f32 },
                                p2: Vec2F32 { x: (bullet_position.x - radius) as f32, y: (bullet_position.y + radius) as f32 },
                                color: hit_color.clone(),
                                width: 0.1,
                            });
                            debug.draw(model::CustomData::Line {
                                p1: Vec2F32 { x: (bullet_position.x - radius) as f32, y: (bullet_position.y + radius) as f32 },
                                p2: Vec2F32 { x: (bullet_position.x + radius) as f32, y: (bullet_position.y + radius) as f32 },
                                color: hit_color.clone(),
                                width: 0.1,
                            });
                            debug.draw(model::CustomData::Line {
                                p1: Vec2F32 { x: (bullet_position.x + radius) as f32, y: (bullet_position.y + radius) as f32 },
                                p2: Vec2F32 { x: (bullet_position.x + radius) as f32, y: (bullet_position.y - radius) as f32 },
                                color: hit_color.clone(),
                                width: 0.1,
                            });
                            debug.draw(model::CustomData::Line {
                                p1: Vec2F32 { x: (bullet_position.x + radius) as f32, y: (bullet_position.y - radius) as f32 },
                                p2: Vec2F32 { x: (bullet_position.x - radius) as f32, y: (bullet_position.y - radius) as f32 },
                                color: hit_color.clone(),
                                width: 0.1,
                            });
                        }

                        for bullet_hit in bullet_hits.unwrap_or_default() {
                            let bullet_position = bullet_hit.0;
                            let rect_size = 0.3;
                            debug.draw(model::CustomData::Rect {
                                pos: Vec2F32::from64(bullet_position.sub(Vec2F64 { x: rect_size / 2.0, y: rect_size / 2.0 })),
                                size: Vec2F32 { x: rect_size as f32, y: rect_size as f32 },
                                color: hit_color.clone(),
                            });
                        }
                    }
                }
            }
        }

        println!("pos {:?} action {:?}", unit.position, move_action);

        let mut aim = Vec2F64 { x: 0.0, y: 0.0 };
        if let Some(weapon) = &unit.weapon {
            let nearest_enemy = game
                .units
                .iter()
                .filter(|other| other.player_id != unit.player_id)
                .min_by(|a, b| {
                    std::cmp::PartialOrd::partial_cmp(
                        &distance_sqr(a.position, unit.position),
                        &distance_sqr(b.position, unit.position),
                    ).unwrap()
                });
            if let Some(enemy) = nearest_enemy {
                let ticks_to_hit = distance_sqr(unit.position, enemy.position).sqrt() / weapon.params.bullet.speed * game.properties.ticks_per_second;
                let enemy_position = MyStrategy::estimate_enemy_position(&enemy, ticks_to_hit, &self.paths, game);
                MyStrategy::draw_unit(enemy_position, ColorF32 {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 0.5,
                }, 0.1, &game.properties, debug);

                if let Some(last_angle) = weapon.last_angle {
                    let p00 = enemy_position.add(Vec2F64 { x: -enemy.size.x / 2.0, y: 0.0 });
                    let p10 = enemy_position.add(Vec2F64 { x: enemy.size.x / 2.0, y: 0.0 });
                    let p01 = enemy_position.add(Vec2F64 { x: -enemy.size.x / 2.0, y: enemy.size.y });
                    let p11 = enemy_position.add(Vec2F64 { x: enemy.size.x / 2.0, y: enemy.size.y });

                    let angle_to_center = (enemy_position.y - unit.position.y).atan2(enemy_position.x - unit.position.x);
                    let unit_center = unit.position.add(Vec2F64 { x: 0.0, y: game.properties.unit_size.y / 2.0 });
                    let enemy_spread = [p00, p10, p01, p11].iter()
                        .map(|p| delta_angle(angle_to_center, (p.y - unit_center.y).atan2(p.x - unit_center.x)).abs())
                        .max_by_key(|angle| NonNan::new(*angle))
                        .unwrap();
                    let delta = delta_angle(last_angle, angle_to_center);

//                    println!("last_angle {} angle_to_center {} delta {} enemy_spread {} weapon.spread {}", last_angle, angle_to_center, delta, enemy_spread, weapon.spread);

                    //проверим, нужно ли менять прицел, или противник и так в угле поражения?
                    if enemy_spread <= weapon.spread {
                        let miss_angle = delta.abs() + enemy_spread - weapon.spread;
                        if miss_angle > 0.0 {
                            // подвинем на минимальный необходимый угол
                            let new_angle = last_angle + if delta > 0.0 { miss_angle } else { -miss_angle };
//                            println!("last_angle {} new_angle {} rotate {}", last_angle, new_angle, delta_angle(last_angle, new_angle));
                            aim = Vec2F64 {
                                x: new_angle.cos(),
                                y: new_angle.sin(),
                            };
                        } else {
                            aim = Vec2F64 {
                                x: last_angle.cos(),
                                y: last_angle.sin(),
                            };
                        }
                    } else {
                        // прицел точнее, чем размер противника, просто целимся в противника
                        aim = Vec2F64 {
                            x: enemy_position.x - unit.position.x,
                            y: enemy_position.y - unit.position.y,
                        };
                    }
                } else {
                    // просто целимся в противника
                    aim = Vec2F64 {
                        x: enemy_position.x - unit.position.x,
                        y: enemy_position.y - unit.position.y,
                    };
                }
            }
        }

        let mut can_suicide = false;
        if suicide_damage > 0 && unit_data.move_.as_ref().is_some() &&
            unit_data.move_.as_ref().unwrap().typ != MoveType::MineSuicide &&
            unit_data.move_.as_ref().unwrap().typ != MoveType::Recover {
            // если в конце текущего хода можно будет применить мины, то не стреляем
            let pos2 = unit_data.move_.as_ref().unwrap().pos2;
            let target = to_unit_position(pos2);
            let ground = MyStrategy::can_plant_mine(pos2, &game.level);
            if ground && MyStrategy::suicide_is_effective(target, unit.player_id, suicide_damage, &game) {
                println!("can_suicide");
                can_suicide = true;
            }
        }

        let mut shoot = !can_suicide && self.shoot(unit, aim, game, debug);
        let mut plant_mine = false;

        if move_action.is_some() && move_action.as_ref().unwrap().typ == MoveType::MineSuicide {
            aim = Vec2F64 { x: 0.0, y: -1.0 };
            plant_mine = true;
            if unit.mines == 0 {
                println!("BOOM!");
                shoot = true;
            } else {
                shoot = false;
            }
        }

        self.total_time += now.elapsed().as_millis();

        // запоминаем последнее состояние противника
        for unit2 in &game.units {
            if unit2.player_id != unit.player_id {
                self.last_enemy_state.insert(unit2.id, unit2.clone());
            }
        }

        model::UnitAction {
            velocity: move_action.as_ref().map(|mov| mov.velocity).unwrap_or(0.0),
            jump: move_action.as_ref().map(|mov| mov.jump).unwrap_or(false),
            jump_down: move_action.as_ref().map(|mov| mov.jump_down).unwrap_or(false),
            aim: aim.mul(2.0), // защита от 0.5
            shoot,
            reload: false,
            swap_weapon: false,
            plant_mine,
        }
    }

    fn can_plant_mine(pos: TilePos, level: &Level) -> bool {
        if level.tiles[pos.0 as usize][pos.1 as usize] == Tile::Ladder || level.tiles[pos.0 as usize][(pos.1 + 1) as usize] == Tile::Ladder {
            return false;
        }
        match level.tiles[pos.0 as usize][(pos.1 - 1) as usize] {
            Tile::Wall => true,
            Tile::Platform => true,
            _ => false,
        }
    }

    fn get_vertical_state(unit: &Unit, game: &Game) -> VerticalState {
        if !unit.jump_state.can_jump || unit.jump_state.max_time == game.properties.unit_jump_time {
            VerticalState::Default
        } else {
            if unit.jump_state.speed == game.properties.jump_pad_jump_speed {
                VerticalState::PadJump((unit.jump_state.speed * unit.jump_state.max_time).floor() as usize)
            } else {
                VerticalState::Jump((unit.jump_state.speed * unit.jump_state.max_time).floor() as usize)
            }
        }
    }

    fn draw_first_movements(unit: &Unit, game: &Game, debug: &mut crate::Debug) {
        let tile_pos = tile_pos(unit.position.clone());
        let vertical_state = MyStrategy::get_vertical_state(unit, game);
        for movement in get_movements() {
            if let Some(mov2) = movement.can_move(tile_pos, vertical_state, &game.level, &game.properties) {
                debug.draw(model::CustomData::Rect {
                    pos: Vec2F32 { x: mov2.pos2.0 as f32 + 0.5, y: mov2.pos2.1 as f32 + 0.5 },
                    size: Vec2F32 { x: 0.1, y: 0.1 },
                    color: ColorF32 {
                        r: 0.0,
                        g: 1.0,
                        b: 1.0,
                        a: 0.5,
                    },
                });
            }
        }
        debug.draw(model::CustomData::Rect {
            pos: Vec2F32 { x: tile_pos.0 as f32 + 0.5, y: tile_pos.1 as f32 + 0.5 },
            size: Vec2F32 { x: 0.1, y: 0.1 },
            color: ColorF32 {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 0.5,
            },
        });
    }

    #[allow(dead_code)]
    fn draw_all_movements(unit: &Unit, game: &Game, debug: &mut crate::Debug) {
        let now = Instant::now();
        let mut known: HashSet<(TilePos, VerticalState)> = HashSet::new();
        let mut queue = VecDeque::new();
        let pos = tile_pos(unit.position.clone());
        let vertical_state = MyStrategy::get_vertical_state(unit, game);
        queue.push_back(MyStrategy::make_start_node(pos, vertical_state));
        while !queue.is_empty() {
            let mov = queue.pop_front().unwrap();
            if !known.insert((mov.pos2, mov.vertical_state2)) {
                continue;
            }
            debug.draw(model::CustomData::Rect {
                pos: Vec2F32 { x: mov.pos2.0 as f32 + 0.5, y: mov.pos2.1 as f32 + 0.5 },
                size: Vec2F32 { x: 0.1, y: 0.1 },
                color: ColorF32 {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                },
            });
            for movement in get_movements() {
                if let Some(mov2) = movement.can_move(mov.pos2, mov.vertical_state2, &game.level, &game.properties) {
                    if !known.contains(&(mov2.pos2, mov2.vertical_state2)) {
                        queue.push_back(mov2);
                    }
                }
            }
        }
        println!("known {} {} ms", known.len(), now.elapsed().as_millis());
    }

    #[allow(dead_code)]
    fn draw_all_movements2(&self, debug: &mut crate::Debug) {
        self.paths.outgoing.values().flat_map(|vec| vec).for_each(|mov| {
            debug.draw(model::CustomData::Rect {
                pos: Vec2F32 { x: mov.pos2.0 as f32 + 0.5, y: mov.pos2.1 as f32 + 0.5 },
                size: Vec2F32 { x: 0.1, y: 0.1 },
                color: ColorF32 {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                },
            });
        });
    }

    fn make_start_node(pos: TilePos, vertical_state: VerticalState) -> Move {
        Move {
            typ: MoveType::Start,
            pos1: pos,
            vertical_state1: vertical_state,
            pos2: pos,
            vertical_state2: vertical_state,
            ticks: 0,
            control: Rc::new(|_, _| ControlResult::TargetReached),
        }
    }

    /// посчитать, где пуля столкнется со стеной/границей мира
    fn bullet_end(position: Vec2F64, aim: Vec2F64, weapon_type: &WeaponType, level: &Level, properties: &Properties) -> (Vec2F64, f64) {
        let weapon_params = &properties.weapon_params[weapon_type];
        let speed = {
            let aim_length = ((aim.x).powi(2) + (aim.y).powi(2)).sqrt();
            Vec2F64 { x: aim.x * weapon_params.bullet.speed / aim_length, y: aim.y * weapon_params.bullet.speed / aim_length }
        };
        let half_bullet_size = weapon_params.bullet.size / 2.0;
        let mut position2 = position;
        let time_step = 1.0 / properties.ticks_per_second / 100.0;// 100 microticks
        loop {
            position2 = position2.add(speed.mul(time_step));
            if MyStrategy::wall_collision(Vec2F64 { x: position2.x - half_bullet_size, y: position2.y - half_bullet_size }, &level) ||
                MyStrategy::wall_collision(Vec2F64 { x: position2.x + half_bullet_size, y: position2.y - half_bullet_size }, &level) ||
                MyStrategy::wall_collision(Vec2F64 { x: position2.x - half_bullet_size, y: position2.y + half_bullet_size }, &level) ||
                MyStrategy::wall_collision(Vec2F64 { x: position2.x + half_bullet_size, y: position2.y + half_bullet_size }, &level) {
                let len = distance_sqr(position, position2).sqrt();
                let tick = len / weapon_params.bullet.speed * properties.ticks_per_second;
                return (position2, tick);
            }
        }
    }

    fn wall_collision(pos: Vec2F64, level: &Level) -> bool {
        if pos.x <= 0.0 || pos.x >= level.width() as f64 || pos.y <= 0.0 || pos.y >= level.height() as f64 {
            return true;
        }
        match level.tiles[pos.x as usize][pos.y as usize] {
            Tile::Wall => true,
            _ => false
        }
    }

    fn shoot(&self, unit: &Unit, aim: Vec2F64, game: &Game, _debug: &mut crate::Debug) -> bool {
        if let Some(weapon) = &unit.weapon {
            if weapon.fire_timer.is_none() {
                // стрелять только в случае, если есть заметный шанс попасть (с учетом explosion)

                // учесть разброс от aim
                let angle = aim.y.atan2(aim.x);
                let spread = (weapon.spread +
                    if weapon.last_angle.is_some() { delta_angle(weapon.last_angle.unwrap(), angle).abs() } else { 0.0 })
                    .min(weapon.params.max_spread);

                let bullet_from = Vec2F64 { x: unit.position.x, y: unit.position.y + unit.size.y / 2.0 };
                let parts = 10; // сколько направлений проверяем
                let mut damage_myself = 0.0;
                let mut damage_enemy = 0.0;
                for i in 0..parts {
                    let angle = -spread + i as f64 * (spread * 2.0 / (parts - 1) as f64);
                    let (mut bullet_end, mut bullet_end_tick) = MyStrategy::bullet_end(bullet_from.clone(), aim.rotate(angle), &weapon.typ, &game.level, &game.properties);

                    // проверить, не попадем ли в какого-то игрока (кроме стреляющего)
                    let mut unit_hit_player: Option<i32> = None;
                    let mut unit_hit_id: Option<i32> = None;
                    let mut last_bullet_pos = bullet_from;
                    for tick in 0..=bullet_end_tick.floor() as i32 {
                        let bullet_at_tick = bullet_from.add(bullet_end.sub(bullet_from).mul(if bullet_end_tick < 1e-6 { 0.0 } else { tick as f64 / bullet_end_tick }));

                        for unit2 in &game.units {
                            if unit2.id == unit.id {
                                continue;
                            }

                            let bullet_radius = weapon.params.bullet.size / 2.0;

                            let unit2_position =
                                if unit2.player_id == unit.player_id {
                                    let unit_data = if self.unit1_data.id == unit2.id { &self.unit1_data } else { &self.unit2_data };
                                    let path_tick = tick + (game.current_tick - unit_data.path_start_tick);
                                    let path = /*&unit_data.path;*/ if unit_data.path.is_empty() { &unit_data.path } else { &unit_data.path[0..1] };
                                    MyStrategy::get_unit_position_at_tick(&unit2, path, path_tick).0
                                } else {
                                    unit2.position.clone()
                                };

                            let p00 = unit2_position.add(Vec2F64 { x: -unit2.size.x / 2.0 - bullet_radius, y: 0.0 - bullet_radius });
                            let p10 = unit2_position.add(Vec2F64 { x: unit2.size.x / 2.0 + bullet_radius, y: 0.0 - bullet_radius });
                            let p01 = unit2_position.add(Vec2F64 { x: -unit2.size.x / 2.0 - bullet_radius, y: unit2.size.y + bullet_radius });
                            let p11 = unit2_position.add(Vec2F64 { x: unit2.size.x / 2.0 + bullet_radius, y: unit2.size.y + bullet_radius });

//                            if game.current_tick == 100 {
//                                println!("tick {}, bullet_at_tick {:?}, p {:?} {:?}", tick, bullet_at_tick, p00, p11);
//                            }

                            for segment in &[(p00, p10), (p10, p11), (p11, p01), (p01, p00)] {
                                if let Some(pos) = segments_intersection(last_bullet_pos, bullet_at_tick, segment.0, segment.1) {
                                    let dist_pos_sqr = distance_sqr(bullet_from, pos);
                                    let dist_end_sqr = distance_sqr(bullet_from, bullet_at_tick);
//                                    if game.current_tick == 100 {
//                                        println!("pos {:?} dp {} de {} et {}", pos, dist_pos_sqr, dist_end_sqr, dist_pos_sqr.sqrt() / dist_end_sqr.sqrt());
//                                    }
                                    if dist_pos_sqr <= dist_end_sqr {
                                        bullet_end = pos;
                                        bullet_end_tick *= dist_pos_sqr.sqrt() / dist_end_sqr.sqrt();
                                        unit_hit_player = Some(unit2.player_id);
                                        unit_hit_id = Some(unit2.id);

                                        MyStrategy::draw_unit(unit2_position, ColorF32 {
                                            r: 1.0,
                                            g: 1.0,
                                            b: 0.5,
                                            a: 0.5,
                                        }, 0.05, &game.properties, _debug);
                                    }
                                }
                            }
                        }

                        if unit_hit_player.is_some() {
                            break;
                        }

                        last_bullet_pos = bullet_at_tick;
                    }

                    _debug.draw(model::CustomData::Line {
                        p1: Vec2F32::from64(bullet_from),
                        p2: Vec2F32::from64(bullet_end),
                        color: ColorF32 {
                            r: 1.0,
                            g: 1.0,
                            b: 0.0,
                            a: 0.2,
                        },
                        width: 0.05,
                    });

                    // корректируем оценку ущерба врагу в зависимости от расстояния (чтобы балансировать с ущербом себе)
                    let enemy_damage_coef = 1.0 / (bullet_end_tick * game.properties.unit_max_horizontal_speed / game.properties.ticks_per_second).max(1.0);

                    let mut damage_per_unit = FnvHashMap::default();

                    // оценка ущерба от пули
                    if let Some(unit_hit_player) = unit_hit_player {
                        let damage = weapon.params.bullet.damage;
                        if unit_hit_player != unit.player_id {
                            println!("tick {} enemy_damage_coef {}", bullet_end_tick, enemy_damage_coef);
                            let damage2 = damage as f64 * enemy_damage_coef;
                            damage_enemy += damage2;
                            *damage_per_unit.entry(unit_hit_id.unwrap()).or_insert(0.0) += damage2;
                        } else {
                            damage_myself += damage as f64;
                            *damage_per_unit.entry(unit_hit_id.unwrap()).or_insert(0.0) += damage as f64;
                        }
                    }

                    // оценка ущерба от взрыва
                    if weapon.params.explosion.is_some() {
                        let explosion_radius = weapon.params.explosion.as_ref().unwrap().radius;
                        let damage = weapon.params.explosion.as_ref().unwrap().damage;
                        for unit2 in &game.units {
                            let unit_position =
                                if unit2.player_id == unit.player_id {
                                    let unit_data = if self.unit1_data.id == unit2.id { &self.unit1_data } else { &self.unit2_data };
                                    let path_tick = bullet_end_tick.floor() as i32 + (game.current_tick - unit_data.path_start_tick);
                                    let t = bullet_end_tick % 1.0;
                                    let (position1, position2) = MyStrategy::get_unit_position_at_tick(&unit2, &unit_data.path, path_tick);
                                    position1.add(position2.sub(position1).mul(t))
                                } else {
                                    unit2.position.clone()
                                };
                            if MyStrategy::damage_unit_by_explosion(unit_position, bullet_end, explosion_radius, unit.size.y) {
                                if unit2.player_id != unit.player_id {
                                    println!("tick {} enemy_damage_coef {}", bullet_end_tick, enemy_damage_coef);
                                    let damage2 = damage as f64 * enemy_damage_coef;
                                    damage_enemy += damage2;
                                    *damage_per_unit.entry(unit2.id).or_insert(0.0) += damage2;
                                } else {
                                    MyStrategy::draw_unit(unit_position, ColorF32 {
                                        r: 1.0,
                                        g: 0.5,
                                        b: 0.0,
                                        a: 0.5,
                                    }, 0.1, &game.properties, _debug);
                                    damage_myself += damage as f64;
                                    *damage_per_unit.entry(unit2.id).or_insert(0.0) += damage as f64;
                                }
                            }
                        }
                    }

//                    // добавим скор за смерть
//                    let dead_cost = 100.0;
//                    for unit2 in &game.units {
//                        let unit_damage = *damage_per_unit.get(&unit2.id).unwrap_or(&0.0);
//                        if (unit2.health as f64) < unit_damage {
//                            if unit2.player_id != unit.player_id {
//                                println!("dead_cost enemy {}", unit2.id);
//                                damage_enemy += dead_cost;
//                            } else {
//                                println!("dead_cost myself {}", unit2.id);
//                                damage_myself += dead_cost;
//                            }
//                        }
//                    }
                }

//                println!("damage_myself {} damage_enemy {}", damage_myself / parts, damage_enemy / parts);

                println!("damage_enemy {} damage_myself {}", damage_enemy / parts as f64, damage_myself / parts as f64);
                damage_enemy > 0.0 && damage_enemy > damage_myself
            } else {
                false
            }
        } else {
            false
        }
    }

    fn damage_unit_by_explosion(unit_position: Vec2F64, explosion: Vec2F64, explosion_radius: f64, unit_height: f64) -> bool {
        MyStrategy::damage_by_explosion(unit_position.add(Vec2F64 { x: -0.5, y: 0.0 }), explosion.clone(), explosion_radius) ||
            MyStrategy::damage_by_explosion(unit_position.add(Vec2F64 { x: 0.5, y: 0.0 }), explosion.clone(), explosion_radius) ||
            MyStrategy::damage_by_explosion(unit_position.add(Vec2F64 { x: -0.5, y: unit_height }), explosion.clone(), explosion_radius) ||
            MyStrategy::damage_by_explosion(unit_position.add(Vec2F64 { x: 0.5, y: unit_height }), explosion.clone(), explosion_radius)
    }

    fn damage_by_explosion(position: Vec2F64, explosion: Vec2F64, explosion_radius: f64) -> bool {
        (position.x - explosion.x).abs() <= explosion_radius && (position.y - explosion.y).abs() <= explosion_radius
    }

    fn build_all_paths(target_pos: TilePos, paths: &Paths) -> FnvHashMap<(TilePos, VerticalState), i32> {
        let mut map: FnvHashMap<(TilePos, VerticalState), i32> = FnvHashMap::default();
        let mut queue = VecDeque::new();
        paths.incoming
            .keys()
            .filter(|(pos, _vertical_state)| *pos == target_pos)
            .for_each(|(pos, vertical_state)| queue.push_back((*pos, *vertical_state, 0)));
        let empty_vec = vec![];
        while !queue.is_empty() {
            let (pos, vertical_state, ticks) = queue.pop_front().unwrap();
            match map.entry((pos, vertical_state)) {
                Vacant(e) => {
                    e.insert(ticks);
                }
                Occupied(mut e) => {
                    if *e.get() > ticks { e.insert(ticks); } else { continue; }
                }
            };
            paths.incoming.get(&(pos, vertical_state)).unwrap_or_else(|| &empty_vec)
                .iter()
                .for_each(|mov| queue.push_back((mov.pos1, mov.vertical_state1, ticks + mov.ticks)));
        }
        map
    }

    fn calc_damage(mov: &Move, from_tick: i32, unit_id: i32, bullets_state: &BulletsState, bullets: &Bullets, game: &Game) -> (i32, BulletsState) {
        // проверим на урон
        let mut damage = 0;
        let mut bullets_state = bullets_state.clone();
        if bullets.need_test(&bullets_state) {
            for mov_tick in 0..mov.ticks {
                let micro_ticks = 100;
                for micro_tick in 0..micro_ticks {
                    let t = micro_tick as f64 / micro_ticks as f64;
                    let position = Vec2F64 {
                        x: mov.pos1.0 as f64 + (mov.pos2.0 as f64 - mov.pos1.0 as f64) * (mov_tick as f64 + t) / mov.ticks as f64 + 0.5,
                        y: mov.pos1.1 as f64 + (mov.pos2.1 as f64 - mov.pos1.1 as f64) * (mov_tick as f64 + t) / mov.ticks as f64,
                    };

                    let (bullet_hits, explosion_hits, new_bullets_state) =
                        bullets.test(position, unit_id, from_tick as f64 + mov_tick as f64 + t, &bullets_state, &game.properties);
                    bullets_state = new_bullets_state;
                    bullet_hits.unwrap_or_default().iter().for_each(|hit| damage += hit.1);
                    explosion_hits.unwrap_or_default().iter().for_each(|hit| damage += hit.1);
                }
            }
        }
        (damage, bullets_state)
    }

    /// возвращает положение на начало тика и на конец тика
    fn get_unit_position_at_tick(unit: &Unit, path: &[Move], path_tick: i32) -> (Vec2F64, Vec2F64) {
        let mut result = (unit.position.clone(), unit.position.clone());
        if !path.is_empty() {
            let mut tick2 = path_tick;
            let mut found = false;
            for i in 1..path.len() {
                if tick2 < path[i].ticks as i32 {
                    let position1 = to_unit_position(path[i].pos1);
                    let position2 = to_unit_position(path[i].pos2);
                    let t1 = tick2 as f64 / path[i].ticks as f64;
                    let t2 = (tick2 + 1) as f64 / path[i].ticks as f64;
                    result = (
                        position1.add(position2.sub(position1).mul(t1)),
                        position1.add(position2.sub(position1).mul(t2)),
                    );
                    found = true;
                    break;
                } else {
                    tick2 -= path[i].ticks;
                }
            }
            if !found {
                result = (to_unit_position(path.last().unwrap().pos2), to_unit_position(path.last().unwrap().pos2));
            }
        }
        result
    }

    fn draw_unit(unit_position: Vec2F64, color: ColorF32, width: f32, properties: &Properties, debug: &mut crate::Debug) {
        let unit_size = properties.unit_size.clone();
        let p00 = unit_position.add(Vec2F64 { x: -unit_size.x / 2.0, y: 0.0 });
        let p10 = unit_position.add(Vec2F64 { x: unit_size.x / 2.0, y: 0.0 });
        let p01 = unit_position.add(Vec2F64 { x: -unit_size.x / 2.0, y: unit_size.y });
        let p11 = unit_position.add(Vec2F64 { x: unit_size.x / 2.0, y: unit_size.y });

        debug.draw(model::CustomData::Line {
            p1: Vec2F32::from64(p00.clone()),
            p2: Vec2F32::from64(p01.clone()),
            color: color.clone(),
            width: width,
        });
        debug.draw(model::CustomData::Line {
            p1: Vec2F32::from64(p01.clone()),
            p2: Vec2F32::from64(p11.clone()),
            color: color.clone(),
            width: width,
        });
        debug.draw(model::CustomData::Line {
            p1: Vec2F32::from64(p11.clone()),
            p2: Vec2F32::from64(p10.clone()),
            color: color.clone(),
            width: width,
        });
        debug.draw(model::CustomData::Line {
            p1: Vec2F32::from64(p10.clone()),
            p2: Vec2F32::from64(p00.clone()),
            color: color.clone(),
            width: width,
        });
    }

    fn estimate_enemy_position(enemy: &Unit, tick: f64, paths: &Paths, game: &Game) -> Vec2F64 {
        let mut average_position = Vec2F64 { x: 0.0, y: 0.0 };
        let mut count = 0;
        let mut stack = Vec::new();
        stack.push((MyStrategy::make_start_node(tile_pos(enemy.position), MyStrategy::get_vertical_state(enemy, game)), 0));
        while let Some((mov, tick2)) = stack.pop() {
            if tick2 as f64 >= tick.min(20.0) {
                average_position = average_position.add(to_unit_position(mov.pos2));
                count += 1;
                continue;
            }
            for mov2 in paths.outgoing.get(&(mov.pos2, mov.vertical_state2)).unwrap_or(&Vec::new()) {
                stack.push((mov2.clone(), tick2 + mov2.ticks));
            }
        }
        if count == 0 {
            enemy.position
        } else {
            average_position.mul(1.0 / count as f64)
        }
    }

    fn suicide_is_effective(planting_unit_position: Vec2F64, my_player_id: i32, suicide_damage: i32, game: &Game) -> bool {
        fn in_mine_explosion_radius(unit: &Unit, planting_unit_position: Vec2F64, properties: &Properties) -> bool {
            let mine_position = planting_unit_position;
            (unit.position.x - mine_position.x).abs() <= properties.mine_explosion_params.radius &&
                (unit.position.y - mine_position.y).abs() <= properties.mine_explosion_params.radius
        }

        game.units.iter()
            .filter(|unit2| unit2.player_id != my_player_id)
            .any(|unit2| unit2.health <= suicide_damage && in_mine_explosion_radius(unit2, planting_unit_position, &game.properties))
    }
}

fn distance_sqr(a: Vec2F64, b: Vec2F64) -> f64 {
    (a.x - b.x).powi(2) + (a.y - b.y).powi(2)
}

fn segments_intersection(a1: Vec2F64, a2: Vec2F64, b1: Vec2F64, b2: Vec2F64) -> Option<Vec2F64> {
    let d = (a1.x - a2.x) * (b2.y - b1.y) - (a1.y - a2.y) * (b2.x - b1.x);
    let da = (a1.x - b1.x) * (b2.y - b1.y) - (a1.y - b1.y) * (b2.x - b1.x);
    let db = (a1.x - a2.x) * (a1.y - b1.y) - (a1.y - a2.y) * (a1.x - b1.x);

    if d.abs() < 1e-9 {
        return None; // almost parallel lines
    }

    let ta = da / d;
    let tb = db / d;

    if ta < 0.0 || ta > 1.0 || tb < 0.0 || tb > 1.0 {
        return None;
    }

    Some(Vec2F64 { x: a1.x + ta * (a2.x - a1.x), y: a1.y + ta * (a2.y - a1.y) })
}

fn tile_pos(position: Vec2F64) -> TilePos {
    (position.x as isize, position.y as isize)
}

fn to_unit_position(pos: TilePos) -> Vec2F64 {
    Vec2F64 { x: pos.0 as f64 + 0.5, y: pos.1 as f64 }
}

fn delta_angle(angle_from: f64, angle_to: f64) -> f64 {
    normalize_angle(angle_to - angle_from)
}

fn normalize_angle(mut angle: f64) -> f64 {
    while angle > std::f64::consts::PI {
        angle -= 2.0 * std::f64::consts::PI;
    }
    while angle < -std::f64::consts::PI {
        angle += 2.0 * std::f64::consts::PI;
    }
    angle
}

struct Paths {
    outgoing: FnvHashMap<(TilePos, VerticalState), Vec<Move>>,
    incoming: FnvHashMap<(TilePos, VerticalState), Vec<Move>>,
}

impl Paths {
    fn update_paths(&mut self, pos: TilePos, vertical_state: VerticalState, game: &Game) {
        let now = Instant::now();
        let mut queue = VecDeque::new();
        queue.push_back(MyStrategy::make_start_node(pos, vertical_state));
        let mut count_o = 0;
        let mut count_i = 0;
        while !queue.is_empty() {
            let mov = queue.pop_front().unwrap();
            if self.outgoing.get(&(mov.pos2, mov.vertical_state2)).is_some() {
                continue;
            }
            self.outgoing.insert((mov.pos2, mov.vertical_state2), Vec::new());
            for movement in get_movements() {
                if let Some(mov2) = movement.can_move(mov.pos2, mov.vertical_state2, &game.level, &game.properties) {
                    let outgoing_entry = self.outgoing.entry((mov2.pos1, mov2.vertical_state1)).or_insert_with(|| Vec::new());
                    if !outgoing_entry.contains(&mov2) {
                        outgoing_entry.push(mov2.clone());
                        count_o += 1;
                    }
                    let incoming_entry = self.incoming.entry((mov2.pos2, mov2.vertical_state2)).or_insert_with(|| Vec::new());
                    if !incoming_entry.contains(&mov2) {
                        incoming_entry.push(mov2.clone());
                        count_i += 1;
                    }
                    if self.outgoing.get(&(mov2.pos2, mov2.vertical_state2)).is_none() {
                        queue.push_back(mov2);
                    }
                }
            }
        }
        println!("outgoing {} incoming {} count_o {} count_i {}, {} ms", self.outgoing.len(), self.incoming.len(), count_o, count_i, now.elapsed().as_millis());
    }
}

#[derive(Clone)]
struct Bullets {
    bullets: Vec<(Bullet, Vec2F64, f64)>, // Bullet, end pos, end tick (with microticks)
}

impl Bullets {
    fn new(game: &Game) -> Bullets {
        let mut bullets = Vec::new();
        for bullet in &game.bullets {
            let (end, tick) = MyStrategy::bullet_end(bullet.position, bullet.velocity, &bullet.weapon_type, &game.level, &game.properties);
            bullets.push((bullet.clone(), end, tick));
        }
        Bullets {
            bullets
        }
    }

    /// Оценивает попадания и убирает пули, которые попали в игрока или в стены/границы
    fn test(&self, unit_position: Vec2F64, unit_id: i32, tick: f64, bullets_state: &BulletsState, properties: &Properties)
            -> (Option<Vec<(Vec2F64, i32)>>, Option<Vec<(Vec2F64, i32, f64)>>, BulletsState) { // bullet hits (pos, damage), explosion hits (pos, damage, radius), removed bullets
        let unit_center = unit_position.add(Vec2F64 { x: 0.0, y: properties.unit_size.y / 2.0 });

        let mut bullet_hits = None;
        let mut explosion_hits = None;

        let mut new_bullet_state = bullets_state.clone();

        for (index, bullet) in self.bullets.iter().enumerate() {
            if bullets_state.is_bullet_removed(index) {
                continue;
            }

            let mut explode = false;

            let bullet_position = Vec2F64 {
                x: bullet.0.position.x as f64 + bullet.0.velocity.x * tick / properties.ticks_per_second,
                y: bullet.0.position.y as f64 + bullet.0.velocity.y * tick / properties.ticks_per_second,
            };

            if bullet.2 < tick {
                new_bullet_state.remove_bullet(index);
                explode = bullet.0.explosion_params.is_some();
            } else {
                // попадание пули
                // на свою пулю наткнуться нельзя
                let half_bullet_size = bullet.0.size / 2.0;
                if (unit_center.x - bullet_position.x).abs() <= properties.unit_size.x / 2.0 + half_bullet_size &&
                    (unit_center.y - bullet_position.y).abs() <= properties.unit_size.y / 2.0 + half_bullet_size &&
                    bullet.0.unit_id != unit_id {
                    new_bullet_state.remove_bullet(index);
                    explode = bullet.0.explosion_params.is_some();
                    if bullet_hits.is_none() {
                        bullet_hits = Some(Vec::new());
                    }
                    bullet_hits.as_mut().unwrap().push((bullet_position, bullet.0.damage));
                }
            }

            // взрыв
            if explode {
                let radius = bullet.0.explosion_params.as_ref().unwrap().radius;
                if (unit_center.x - bullet_position.x).abs() <= properties.unit_size.x / 2.0 + radius &&
                    (unit_center.y - bullet_position.y).abs() <= properties.unit_size.y / 2.0 + radius {
                    if explosion_hits.is_none() {
                        explosion_hits = Some(Vec::new());
                    }
                    explosion_hits.as_mut().unwrap().push((bullet_position, bullet.0.explosion_params.as_ref().unwrap().damage, bullet.0.explosion_params.as_ref().unwrap().radius));
                }
            }
        };

        (bullet_hits, explosion_hits, new_bullet_state)
    }

    fn need_test(&self, bullets_state: &BulletsState) -> bool {
        self.bullets.len() > bullets_state.count_removed_bullets()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct BulletsState {
    removed_bullets: Vec<bool>, // индекс пули в Bullets
}

impl BulletsState {
    fn new() -> Self {
        Self {
            removed_bullets: vec![]
        }
    }

    fn is_bullet_removed(&self, index: usize) -> bool {
        *self.removed_bullets.get(index).unwrap_or(&false)
    }

    fn remove_bullet(&mut self, index: usize) {
        while self.removed_bullets.len() <= index {
            self.removed_bullets.push(false);
        }
        self.removed_bullets[index] = true;
    }

    fn count_removed_bullets(&self) -> usize {
        self.removed_bullets.iter().filter(|b| **b).count()
    }
}