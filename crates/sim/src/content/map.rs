//! Map definitions (maps.md). Plain data; an importer adapter fills these
//! from the scene graph (Transform chains + collider/rigidbody/script fields).
//! [`Affine`] helps turning Unity transforms into world shapes.

use crate::geom::{Shape, layers};
use crate::math::{Curve, V2, v2};
use serde::{Deserialize, Serialize};

/// 2D affine transform (column vectors `ax`, `ay` + translation). Unity
/// transforms compose as `parent * child`; with a non-uniform scale under a
/// rotated parent the result can shear, which [`Affine::box_shape`] handles by
/// emitting a polygon.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub ax: V2,
    pub ay: V2,
    pub t: V2,
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        ax: v2(1.0, 0.0),
        ay: v2(0.0, 1.0),
        t: v2(0.0, 0.0),
    };

    /// From a local position, Z rotation in radians and scale.
    pub fn trs(pos: V2, angle: f32, scale: V2) -> Affine {
        let (s, c) = (crate::math::sin(angle), crate::math::cos(angle));
        Affine {
            ax: v2(c, s) * scale.x,
            ay: v2(-s, c) * scale.y,
            t: pos,
        }
    }

    /// From a Unity quaternion (only z/w matter for 2D) - angle = 2*atan2(z, w).
    pub fn trs_quat(pos: V2, qz: f32, qw: f32, scale: V2) -> Affine {
        Affine::trs(pos, 2.0 * crate::math::atan2(qz, qw), scale)
    }

    pub fn mul(&self, o: &Affine) -> Affine {
        Affine {
            ax: self.apply_vec(o.ax),
            ay: self.apply_vec(o.ay),
            t: self.apply(o.t),
        }
    }
    pub fn apply(&self, p: V2) -> V2 {
        self.t + self.ax * p.x + self.ay * p.y
    }
    pub fn apply_vec(&self, v: V2) -> V2 {
        self.ax * v.x + self.ay * v.y
    }
    /// Rotation angle of the x axis.
    pub fn angle(&self) -> f32 {
        self.ax.angle()
    }
    /// Inverse (assumes non-degenerate).
    pub fn inverse(&self) -> Affine {
        let det = self.ax.x * self.ay.y - self.ay.x * self.ax.y;
        let ix = v2(self.ay.y, -self.ax.y) / det;
        let iy = v2(-self.ay.x, self.ax.x) / det;
        let inv = Affine {
            ax: ix,
            ay: iy,
            t: V2::ZERO,
        };
        Affine {
            t: -inv.apply_vec(self.t),
            ..inv
        }
    }

    fn is_orthogonal(&self) -> bool {
        self.ax.dot(self.ay).abs() <= 1e-4 * (self.ax.len() * self.ay.len()).max(1e-9)
    }

    /// World shape of a `BoxCollider2D` (size, offset) under this transform.
    pub fn box_shape(&self, size: V2, offset: V2) -> Shape {
        let c = self.apply(offset);
        if self.is_orthogonal() {
            let hx = self.ax.len() * size.x.abs() * 0.5;
            let hy = self.ay.len() * size.y.abs() * 0.5;
            Shape::Obb {
                c,
                axis: self.ax.normalized(),
                half: v2(hx, hy),
            }
        } else {
            let h = size * 0.5;
            let pts = [v2(-h.x, -h.y), v2(h.x, -h.y), v2(h.x, h.y), v2(-h.x, h.y)]
                .iter()
                .map(|p| self.apply(*p + offset))
                .collect();
            Shape::poly(pts)
        }
    }

    /// World shape of a `CircleCollider2D` (Unity uses the largest axis scale).
    pub fn circle_shape(&self, radius: f32, offset: V2) -> Shape {
        let s = self.ax.len().max(self.ay.len());
        Shape::Circle {
            c: self.apply(offset),
            r: radius * s,
        }
    }

    /// World shape of a convex `PolygonCollider2D` path.
    pub fn poly_shape(&self, pts: &[V2], offset: V2) -> Shape {
        Shape::poly(pts.iter().map(|p| self.apply(*p + offset)).collect())
    }
}

/// A collider: shape (world space for statics, body-local for props).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColliderDef {
    pub shape: Shape,
    #[serde(default)]
    pub layer: u8,
    #[serde(default)]
    pub trigger: bool,
    /// Physics material (default material: friction 0.4, bounciness 0).
    #[serde(default = "default_friction")]
    pub friction: f32,
    #[serde(default)]
    pub bounciness: f32,
}

fn default_friction() -> f32 {
    0.4
}

impl ColliderDef {
    pub fn solid(shape: Shape) -> ColliderDef {
        ColliderDef {
            shape,
            layer: layers::DEFAULT,
            trigger: false,
            friction: 0.4,
            bounciness: 0.0,
        }
    }
}

/// `SpawnPoint` (maps.md 1.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpawnPointDef {
    pub pos: V2,
    #[serde(default)]
    pub team_id: i32,
}

/// `DamageBox` (saws, maps.md 5.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageBoxDef {
    #[serde(default = "d55")]
    pub damage: f32,
    #[serde(default = "d40000")]
    pub force: f32,
    /// Real seconds between hits on the same object.
    #[serde(default = "d02")]
    pub cd: f32,
    #[serde(default = "d05")]
    pub flying_for: f32,
    #[serde(default = "d5")]
    pub shake: f32,
    /// Visual blade rotation (`Rotate.speed`, deg/s of game time).
    #[serde(default = "d500")]
    pub rotate_speed: f32,
}

fn d55() -> f32 {
    55.0
}
fn d40000() -> f32 {
    40000.0
}
fn d02() -> f32 {
    0.2
}
fn d05() -> f32 {
    0.5
}
fn d5() -> f32 {
    5.0
}
fn d500() -> f32 {
    500.0
}

impl Default for DamageBoxDef {
    fn default() -> Self {
        DamageBoxDef {
            damage: 55.0,
            force: 40000.0,
            cd: 0.2,
            flying_for: 0.5,
            shake: 5.0,
            rotate_speed: 500.0,
        }
    }
}

/// A static map piece.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticDef {
    pub name: String,
    /// Source object id in the import (`"<file>:<path id>"`), so the
    /// presentation can match it to imported sprites; empty for hand-built content.
    #[serde(default)]
    pub id: String,
    pub collider: ColliderDef,
    #[serde(default)]
    pub damage_box: Option<DamageBoxDef>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    #[default]
    Dynamic,
    Kinematic,
    Static,
}

/// `Rigidbody2D` parameters (maps.md 3.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyDef {
    #[serde(default)]
    pub kind: BodyKind,
    pub mass: f32,
    #[serde(default)]
    pub linear_drag: f32,
    #[serde(default = "d005")]
    pub angular_drag: f32,
    #[serde(default = "d1")]
    pub gravity_scale: f32,
    #[serde(default)]
    pub freeze_rotation: bool,
}

fn d005() -> f32 {
    0.05
}
fn d1() -> f32 {
    1.0
}

impl Default for BodyDef {
    fn default() -> Self {
        BodyDef {
            kind: BodyKind::Dynamic,
            mass: 20000.0,
            linear_drag: 0.2,
            angular_drag: 0.2,
            gravity_scale: 1.0,
            freeze_rotation: false,
        }
    }
}

/// `MoveSequence` (maps.md 3.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveSequenceDef {
    pub positions: Vec<V2>,
    #[serde(default = "d1")]
    pub drag: f32,
    pub spring: f32,
    #[serde(default = "d5")]
    pub cap: f32,
    #[serde(default = "d3")]
    pub threshold: f32,
    pub time_at_pos: f32,
}

fn d3() -> f32 {
    3.0
}

/// One debris piece of a destructible box (body-local pose relative to the box).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PieceDef {
    /// Source object id in the import (`"<file>:<path id>"`), so the
    /// presentation can match it to imported sprites; empty for hand-built content.
    #[serde(default)]
    pub id: String,
    pub pos: V2,
    #[serde(default)]
    pub angle: f32,
    pub collider: ColliderDef,
    pub body: BodyDef,
}

/// `DamagableEvent` + `DestructibleBoxDestruction` (maps.md 5.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DestructibleDef {
    #[serde(default = "d50")]
    pub hp: f32,
    #[serde(default)]
    pub pieces: Vec<PieceDef>,
}

fn d50() -> f32 {
    50.0
}

/// `NetworkPhysicsObject` parameters (maps.md 3.3; player.md 7.5).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PushableDef {
    #[serde(default = "d05")]
    pub dmg_amount: f32,
    #[serde(default = "d1500")]
    pub force_amount: f32,
    /// The player-collision threshold. player.md 7.5 says 1, maps.md says the
    /// serialized value 5: the importer's value wins; default 5.
    #[serde(default = "d5")]
    pub player_col_threshold: f32,
    /// Push speed constant (`speed`, 8000 in the data; player.md's "propSpeed").
    #[serde(default = "d8000")]
    pub speed: f32,
    #[serde(default = "d20")]
    pub bullet_push_multiplier: f32,
}

fn d1500() -> f32 {
    1500.0
}
fn d8000() -> f32 {
    8000.0
}
fn d20() -> f32 {
    20.0
}

impl Default for PushableDef {
    fn default() -> Self {
        PushableDef {
            dmg_amount: 0.5,
            force_amount: 1500.0,
            player_col_threshold: 5.0,
            speed: 8000.0,
            bullet_push_multiplier: 20.0,
        }
    }
}

/// A physics prop (maps.md 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropDef {
    pub name: String,
    /// Source object id in the import (`"<file>:<path id>"`), so the
    /// presentation can match it to imported sprites; empty for hand-built content.
    #[serde(default)]
    pub id: String,
    pub pos: V2,
    #[serde(default)]
    pub angle: f32,
    /// Body-local colliders (relative to `pos`/`angle`, scale applied).
    pub colliders: Vec<ColliderDef>,
    pub body: BodyDef,
    /// Present on networked props (all props in the shipped maps).
    #[serde(default)]
    pub pushable: Option<PushableDef>,
    #[serde(default)]
    pub move_sequence: Option<MoveSequenceDef>,
    /// `RigLookUp.force` (self-righting torque).
    #[serde(default)]
    pub rig_look_up: Option<f32>,
    #[serde(default)]
    pub destructible: Option<DestructibleDef>,
    #[serde(default)]
    pub damage_box: Option<DamageBoxDef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RopeKind {
    Spring,
    Distance,
}

/// `MapObjet_Rope`: resolved against bodies at the map-ready tick (maps.md 4).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RopeDef {
    pub start: V2,
    pub end: V2,
    pub kind: RopeKind,
}

/// Hand-authored joints (`FixedJoint2D`, `SpringJoint2D`, `DistanceJoint2D`,
/// `HingeJoint2D`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum JointKind {
    Fixed,
    Spring {
        frequency: f32,
        damping_ratio: f32,
        distance: Option<f32>,
    },
    Distance {
        distance: Option<f32>,
        max_only: bool,
    },
    Hinge {
        limits: Option<(f32, f32)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JointDef {
    /// Index into `MapDef::props`.
    pub body_a: u32,
    /// Index into `MapDef::props`, or `None` = attached to the world.
    pub body_b: Option<u32>,
    pub kind: JointKind,
    /// Anchor in A's local space.
    pub anchor_a: V2,
    /// Anchor in B's local space (or world position when `body_b` is None).
    pub anchor_b: V2,
    #[serde(default)]
    pub collide_connected: bool,
}

/// `KillBox` (maps.md 5.2): a box in the animated group's local space that
/// kills players whose centre is inside while armed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillBoxDef {
    pub center: V2,
    pub half: V2,
    /// Real seconds armed per activation.
    pub toggle_time: f32,
    /// Curve time at which it is activated each cycle.
    pub activate_at: f32,
    #[serde(default = "d1000")]
    pub damage: f32,
}

fn d1000() -> f32 {
    1000.0
}

/// A `CurveAnimation` group scaling its children's Y (the smashers of
/// `Factory1_M`, maps.md 5.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimatedDef {
    pub name: String,
    /// Source object id in the import (`"<file>:<path id>"`), so the
    /// presentation can match it to imported sprites; empty for hand-built content.
    #[serde(default)]
    pub id: String,
    /// World transform of the animated object at scale 1.
    pub pivot: V2,
    #[serde(default)]
    pub angle: f32,
    /// Scale factor over curve time 0..1.
    pub curve: Curve,
    /// Curve time advanced per game second.
    pub speed: f32,
    /// Game seconds before the loop starts (0 = plays on awake).
    #[serde(default)]
    pub start_delay: f32,
    /// Solid colliders in pivot-local space (unscaled).
    #[serde(default)]
    pub colliders: Vec<ColliderDef>,
    #[serde(default)]
    pub kill_box: Option<KillBoxDef>,
}

/// One map scene.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MapDef {
    pub name: String,
    /// In hierarchy order ("SPAWN POINT 1..4").
    pub spawn_points: Vec<SpawnPointDef>,
    pub statics: Vec<StaticDef>,
    #[serde(default)]
    pub props: Vec<PropDef>,
    #[serde(default)]
    pub ropes: Vec<RopeDef>,
    #[serde(default)]
    pub joints: Vec<JointDef>,
    #[serde(default)]
    pub animated: Vec<AnimatedDef>,
    /// `Map.size` (camera ortho size target; 20 everywhere).
    #[serde(default = "d20")]
    pub camera_size: f32,
}
