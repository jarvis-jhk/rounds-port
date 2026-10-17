//! Hand-built arenas and cards for tests, benchmarks and the sandbox before
//! imported data exists. Not game data.

use super::*;
use crate::geom::{Shape, layers};
use crate::math::{Curve, V2, key, v2};
use crate::stats::{BlockCardStats, CharacterStats, GunStats};

fn obb(c: V2, half: V2) -> Shape {
    Shape::Obb {
        c,
        axis: V2::RIGHT,
        half,
    }
}

fn ground(name: &str, c: V2, half: V2) -> StaticDef {
    StaticDef {
        id: String::new(),
        name: name.into(),
        collider: ColliderDef::solid(obb(c, half)),
        damage_box: None,
    }
}

/// Top of the floor of the test arenas.
pub const FLOOR_Y: f32 = -12.0;

fn spawns() -> Vec<SpawnPointDef> {
    let y = FLOOR_Y + 1.0;
    vec![
        SpawnPointDef {
            pos: v2(-20.0, y),
            team_id: 0,
        },
        SpawnPointDef {
            pos: v2(20.0, y),
            team_id: 1,
        },
        SpawnPointDef {
            pos: v2(-10.0, y),
            team_id: 0,
        },
        SpawnPointDef {
            pos: v2(10.0, y),
            team_id: 1,
        },
    ]
}

/// A floor spanning the arena with two side walls.
pub fn flat_arena() -> MapDef {
    MapDef {
        name: "Test_Flat".into(),
        spawn_points: spawns(),
        statics: vec![
            ground("Floor", v2(0.0, FLOOR_Y - 5.0), v2(32.0, 5.0)),
            ground("WallL", v2(-31.0, 0.0), v2(1.0, 12.0)),
            ground("WallR", v2(31.0, 0.0), v2(1.0, 12.0)),
        ],
        camera_size: 20.0,
        ..Default::default()
    }
}

/// Floor, walls, floating platforms (one rotated), a circle piece.
pub fn platforms_arena() -> MapDef {
    let mut m = flat_arena();
    m.name = "Test_Platforms".into();
    m.statics
        .push(ground("PlatL", v2(-12.0, -4.0), v2(5.0, 0.5)));
    m.statics
        .push(ground("PlatR", v2(12.0, -4.0), v2(5.0, 0.5)));
    m.statics
        .push(ground("PlatTop", v2(0.0, 4.0), v2(6.0, 0.5)));
    let a = 0.4f32;
    m.statics.push(StaticDef {
        id: String::new(),
        name: "Ramp".into(),
        collider: ColliderDef::solid(Shape::Obb {
            c: v2(0.0, -9.0),
            axis: V2::from_angle(a),
            half: v2(4.0, 0.5),
        }),
        damage_box: None,
    });
    m.statics.push(StaticDef {
        id: String::new(),
        name: "Ball".into(),
        collider: ColliderDef::solid(Shape::Circle {
            c: v2(-24.0, -6.0),
            r: 1.5,
        }),
        damage_box: None,
    });
    m
}

fn crate_prop(name: &str, pos: V2, half: V2, mass: f32) -> PropDef {
    PropDef {
        id: String::new(),
        name: name.into(),
        pos,
        angle: 0.0,
        colliders: vec![ColliderDef {
            shape: obb(V2::ZERO, half),
            layer: layers::IGNORE_PLAYER,
            trigger: false,
            friction: 0.5,
            bounciness: 0.4,
        }],
        body: BodyDef {
            kind: BodyKind::Dynamic,
            mass,
            linear_drag: 0.2,
            angular_drag: 0.2,
            gravity_scale: 1.0,
            freeze_rotation: false,
        },
        pushable: Some(PushableDef::default()),
        move_sequence: None,
        rig_look_up: None,
        destructible: None,
        damage_box: None,
    }
}

/// Floor, walls, crates, a hanging plank on two ropes, a destructible box, a
/// moving platform and a saw.
pub fn physics_arena() -> MapDef {
    let mut m = flat_arena();
    m.name = "Test_Physics".into();
    m.statics
        .push(ground("Ceiling", v2(0.0, 14.0), v2(10.0, 0.5)));
    m.props.push(crate_prop(
        "Crate",
        v2(-5.0, FLOOR_Y + 1.0),
        v2(1.0, 1.0),
        20000.0,
    ));
    m.props.push(crate_prop(
        "Crate",
        v2(5.0, FLOOR_Y + 1.5),
        v2(1.5, 1.5),
        60000.0,
    ));
    m.props.push(crate_prop(
        "Small",
        v2(5.0, FLOOR_Y + 4.0),
        v2(0.5, 0.5),
        1000.0,
    ));
    // Plank hanging from the ceiling (rope start inside the plank, end at the ceiling).
    let mut plank = crate_prop("Plank", v2(0.0, 6.0), v2(4.0, 0.3), 2000.0);
    plank.body.linear_drag = 5.0;
    plank.body.angular_drag = 5.0;
    m.props.push(plank);
    m.ropes.push(RopeDef {
        start: v2(-3.5, 6.0),
        end: v2(-3.5, 13.5),
        kind: RopeKind::Distance,
    });
    m.ropes.push(RopeDef {
        start: v2(3.5, 6.0),
        end: v2(3.5, 13.5),
        kind: RopeKind::Spring,
    });
    // Destructible box.
    let mut d = crate_prop(
        "Destructible",
        v2(-14.0, FLOOR_Y + 1.0),
        v2(1.0, 1.0),
        20000.0,
    );
    let piece = |p: V2| PieceDef {
        id: String::new(),
        pos: p,
        angle: 0.0,
        collider: ColliderDef {
            shape: obb(V2::ZERO, v2(0.5, 0.5)),
            layer: layers::BACKGROUND_OBJECT,
            trigger: false,
            friction: 0.5,
            bounciness: 0.4,
        },
        body: BodyDef {
            mass: 1000.0,
            ..BodyDef::default()
        },
    };
    d.destructible = Some(DestructibleDef {
        hp: 50.0,
        pieces: vec![
            piece(v2(-0.5, -0.5)),
            piece(v2(0.5, -0.5)),
            piece(v2(-0.5, 0.5)),
            piece(v2(0.5, 0.5)),
        ],
    });
    m.props.push(d);
    // Moving platform.
    let mut plat = crate_prop("Stone", v2(-20.0, 0.0), v2(3.0, 0.5), 100000.0);
    plat.body.linear_drag = 3.0;
    plat.body.angular_drag = 3.0;
    plat.body.gravity_scale = 0.0;
    plat.move_sequence = Some(MoveSequenceDef {
        positions: vec![v2(0.0, 0.0), v2(10.0, 0.0)],
        drag: 1.0,
        spring: 300.0,
        cap: 5.0,
        threshold: 3.0,
        time_at_pos: 2.0,
    });
    plat.rig_look_up = Some(3.0);
    m.props.push(plat);
    // Static saw.
    m.statics.push(StaticDef {
        id: String::new(),
        name: "Saw".into(),
        collider: ColliderDef {
            shape: Shape::Circle {
                c: v2(22.0, -2.0),
                r: 1.15,
            },
            layer: layers::IGNORE_MAP,
            trigger: false,
            friction: 0.4,
            bounciness: 0.0,
        },
        damage_box: Some(DamageBoxDef::default()),
    });
    m
}

/// Flat arena with a smasher (animated group with kill box).
pub fn smasher_arena() -> MapDef {
    let mut m = flat_arena();
    m.name = "Test_Smasher".into();
    // Key values of Factory1_M's smasher curve (maps.md 5.2).
    let curve = Curve {
        keys: vec![
            key(0.0, 1.0, 0.0, 0.0),
            key(0.02, 1.0, -0.082, -0.082),
            key(0.1869, 1.1799, 3.6547, 3.6547),
            key(0.2766, 2.1767, 27.948, 27.948),
            key(0.3273, 5.5966, 149.23, 149.23),
            key(0.3377, 8.0, 65.14, 0.0),
            key(0.736, 8.0, 0.0, 0.0),
            key(0.827, 2.36, 0.0, 0.0),
            key(0.931, 1.0, 0.0, 0.0),
            key(1.0, 1.0, 0.0, 0.0),
        ],
    };
    m.animated.push(AnimatedDef {
        id: String::new(),
        name: "Smasher".into(),
        // At full extension (scale 8) the kill box reaches the floor.
        pivot: v2(0.0, -4.0),
        angle: 0.0,
        curve,
        speed: 0.2,
        start_delay: 0.0,
        colliders: vec![ColliderDef::solid(obb(v2(0.0, -0.5), v2(1.0, 0.5)))],
        kill_box: Some(KillBoxDef {
            center: v2(0.0, -0.86),
            half: v2(0.88, 0.15),
            toggle_time: 0.3,
            activate_at: 0.3377,
            damage: 1000.0,
        }),
    });
    m
}

/// A small card set exercising the stat merge paths.
pub fn test_cards() -> Vec<CardDef> {
    let gun = |g: GunStats| Some(g);
    vec![
        CardDef {
            name: "Big Damage".into(),
            rarity: Rarity::Common,
            gun: gun(GunStats {
                damage: 2.0,
                reload_time_add: 0.25,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Scatter".into(),
            rarity: Rarity::Uncommon,
            gun: gun(GunStats {
                damage: 0.4,
                number_of_projectiles: 4,
                ammo: 5,
                spread: 0.5,
                drag: 10.0,
                destroy_bullet_after: 0.2,
                projectile_speed: 3.5,
                gravity: 0.0,
                ..Default::default()
            }),
            blacklisted_categories: vec!["Bounces".into()],
            ..Default::default()
        },
        CardDef {
            name: "Bouncy".into(),
            rarity: Rarity::Uncommon,
            categories: vec!["Bounces".into()],
            gun: gun(GunStats {
                reflects: 3,
                damage: 0.8,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Rapid".into(),
            rarity: Rarity::Common,
            gun: gun(GunStats {
                attack_speed: 0.3,
                ammo: 6,
                damage: 0.5,
                spread: 0.1,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Huge".into(),
            rarity: Rarity::Common,
            character: Some(CharacterStats {
                health: 2.0,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Echo".into(),
            rarity: Rarity::Uncommon,
            character: Some(CharacterStats {
                health: 1.3,
                ..Default::default()
            }),
            block: Some(BlockCardStats {
                additional_blocks: 1,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Explosive".into(),
            rarity: Rarity::Uncommon,
            gun: gun(GunStats {
                attack_speed: 2.0,
                reload_time_add: 0.25,
                ..Default::default()
            }),
            objects_to_spawn: vec![ObjectsToSpawnDef {
                effect: Some("A_Explosion".into()),
                scale_stacks: true,
                scale_from_damage: 0.5,
                ..Default::default()
            }],
            ..Default::default()
        },
        CardDef {
            name: "Phoenix".into(),
            rarity: Rarity::Rare,
            allow_multiple: false,
            character: Some(CharacterStats {
                health: 0.5,
                respawns: 1,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Burst".into(),
            rarity: Rarity::Common,
            gun: gun(GunStats {
                bursts: 2,
                time_between_bullets: 0.1,
                ammo: 3,
                damage: 0.6,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Leech".into(),
            rarity: Rarity::Common,
            character: Some(CharacterStats {
                health: 1.3,
                life_steal: 0.75,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Decay".into(),
            rarity: Rarity::Uncommon,
            character: Some(CharacterStats {
                health: 1.5,
                seconds_to_take_damage_over: 4.0,
                ..Default::default()
            }),
            ..Default::default()
        },
        CardDef {
            name: "Cold".into(),
            rarity: Rarity::Common,
            gun: gun(GunStats {
                slow: 0.7,
                reload_time_add: 0.25,
                ..Default::default()
            }),
            ..Default::default()
        },
    ]
}

pub fn test_objects() -> Vec<ObjectDef> {
    vec![ObjectDef::explosion(
        "A_Explosion",
        ExplosionDef {
            damage: 20.0,
            force: 5000.0,
            range: 2.0,
            ..Default::default()
        },
    )]
}

/// Content with all test arenas and cards.
pub fn test_content() -> Content {
    Content::new(ContentDefs {
        maps: vec![
            flat_arena(),
            platforms_arena(),
            physics_arena(),
            smasher_arena(),
        ],
        cards: test_cards(),
        pool: vec![],
        objects: test_objects(),
        projectiles: vec![],
        tuning: Tuning::default(),
        sound_events: Default::default(),
    })
}
