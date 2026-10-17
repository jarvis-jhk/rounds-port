//! Read-only accessors for the presentation layer (renderer, HUD, audio).
//! Everything returned is plain state; nothing here mutates the world.

use crate::content::{CardId, Content, MapId};
use crate::flow::{Phase, PickHand, PickState};
use crate::geom::{RayHit, layers};
use crate::map::{AnimState, MapPhase};
use crate::math::{V2, v2};
use crate::objects::SpawnedObject;
use crate::physics::{Piece, Prop, RopeView};
use crate::player::Player;
use crate::projectile::Projectile;
use crate::scene::Target;
use crate::world::{Rules, World};

/// Where one of a player's legs touches the ground (the sim's leg ray,
/// player.md 5.6), for drawing the feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootPlant {
    /// The leg's hip point the ray belongs to (`pos + (±0.24, -0.36) *
    /// scale`; index 0 = left, 1 = right).
    pub hip: V2,
    /// Ground point under the hip and the surface normal there.
    pub point: V2,
    pub normal: V2,
    /// What was hit.
    pub target: Target,
    /// The sim uses this hit for ground support this tick (not right after
    /// a jump, surface not steeper than 70°).
    pub supporting: bool,
}

/// Map transition state for drawing the slide (match-flow.md 7.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    pub current: Option<MapId>,
    /// Loaded, waiting off-screen right (x = +90) until called in.
    pub loaded: Option<MapId>,
    pub previous: Option<MapId>,
    pub phase: MapPhase,
    /// Ticks since the current map was called in (drives the slide curve).
    pub since_call_in: u32,
    pub seed: u32,
    pub solid: bool,
    pub ready: bool,
}

impl World {
    pub fn players(&self) -> &[Player] {
        &self.players
    }
    pub fn player(&self, i: usize) -> &Player {
        &self.players[i]
    }
    pub fn projectiles(&self) -> &[Projectile] {
        &self.projectiles
    }
    pub fn objects(&self) -> &[SpawnedObject] {
        &self.objects
    }
    /// Physics props of the current map (poses/shapes; empty before the map
    /// has entered - draw `MapDef::props` at their authored pose then).
    pub fn props(&self) -> &[Prop] {
        &self.physics.props
    }
    /// Debris of destroyed boxes. Each [`Piece`] names its source: `prop`
    /// (index into [`World::props`]) and `piece` (index into that prop
    /// def's `destructible.pieces`, whose object carries the sprite).
    pub fn debris(&self) -> &[Piece] {
        &self.physics.pieces
    }
    /// Ropes that currently hold, with world end points (start = the
    /// `RopeDef::start` end) and what each end is attached to. Empty before
    /// the map is released; draw `MapDef::ropes` at their authored points
    /// then. A rope disappears when a prop it is attached to is destroyed.
    pub fn ropes(&self) -> Vec<RopeView> {
        self.physics.rope_views()
    }
    /// Nearest hit of a ray against the gameplay world (static colliders
    /// while the map is solid, animated groups, props, hittable players):
    /// `dir` is a unit vector, `mask` a layer mask (`geom::layers`, e.g.
    /// `LEG_RAY`, `BULLET_WALLS`), `skip_player` excludes one player.
    pub fn raycast(
        &self,
        content: &Content,
        origin: V2,
        dir: V2,
        max: f32,
        mask: u32,
        skip_player: Option<u8>,
    ) -> Option<(RayHit, Target)> {
        let circles = self.player_circles();
        self.scene(content, &circles)
            .raycast(origin, dir, max, mask, skip_player)
    }
    /// The ground under a player's two legs (left, right) by the same rays
    /// the sim casts each frame (player.md 5.6): from `hip + (0, 0.5)`
    /// straight down for `1.0 * scale`. None = the foot hangs free.
    pub fn foot_plants(&self, content: &Content, player: usize) -> [Option<FootPlant>; 2] {
        let p = &self.players[player];
        let circles = self.player_circles();
        let scene = self.scene(content, &circles);
        let mut out = [None, None];
        for (k, sx) in [-1.0f32, 1.0].iter().enumerate() {
            let hip = p.pos + v2(0.24 * sx, -0.36) * p.scale;
            if let Some((h, t)) = scene.raycast(
                hip + v2(0.0, 0.5),
                V2::DOWN,
                p.scale,
                layers::LEG_RAY,
                Some(player as u8),
            ) {
                out[k] = Some(FootPlant {
                    hip,
                    point: h.point,
                    normal: h.normal,
                    target: t,
                    supporting: p.since_jump >= 0.2 && h.normal.angle_deg(V2::UP) <= 70.0,
                });
            }
        }
        out
    }
    pub fn map_view(&self) -> MapView {
        MapView {
            current: self.map.current,
            loaded: self.map.loaded,
            previous: self.map.previous,
            phase: self.map.phase,
            since_call_in: self.map.since_call_in,
            seed: self.map.seed,
            solid: self.map.solid,
            ready: self.map.ready,
        }
    }
    /// Animated map groups (smashers): curve time and current scale.
    pub fn map_anims(&self) -> &[AnimState] {
        &self.map.anims
    }
    pub fn phase(&self) -> Phase {
        self.flow.phase
    }
    pub fn battle_ongoing(&self) -> bool {
        self.flow.battle
    }
    /// The running card pick (picker, offered cards, stage). In a
    /// simultaneous pick phase ([`World::simultaneous_pick`]) this is the
    /// first hand (lowest player index) that is not done yet, so code
    /// written for sequential picks keeps working; use
    /// [`World::pick_hands`] to show every hand.
    pub fn pick(&self) -> Option<&PickState> {
        self.flow
            .pick
            .as_ref()
            .or_else(|| self.active_hand().map(|h| &h.state))
    }
    /// Highlighted slot of the card pick ([`World::pick`]'s hand).
    pub fn pick_selected(&self) -> u8 {
        match (&self.flow.pick, self.active_hand()) {
            (None, Some(h)) => h.selected,
            _ => self.flow.selected,
        }
    }
    fn active_hand(&self) -> Option<&PickHand> {
        self.flow.hands.iter().find(|h| !h.done)
    }
    /// A simultaneous pick phase is running ([`crate::PickMode`]): every
    /// picker has its own hand in [`World::pick_hands`].
    pub fn simultaneous_pick(&self) -> bool {
        self.flow.simultaneous && matches!(self.flow.phase, Phase::Picking { .. })
    }
    /// Hands of the running simultaneous pick phase, in player order:
    /// picker, offered cards, stage, own cursor, picked slot, done. Empty in
    /// sequential mode and outside pick phases. Done hands stay in the list
    /// until every hand is done.
    pub fn pick_hands(&self) -> &[PickHand] {
        &self.flow.hands
    }
    /// `player`'s hand in the running simultaneous pick phase.
    pub fn pick_hand(&self, player: usize) -> Option<&PickHand> {
        self.flow
            .hands
            .iter()
            .find(|h| h.state.player as usize == player)
    }
    /// Players still to pick in this pick phase (sequential: the queue after
    /// the current picker; simultaneous: before the hands are dealt).
    pub fn pick_queue(&self) -> &[u8] {
        &self.flow.pick_queue
    }
    pub fn points(&self) -> &[u8] {
        &self.points
    }
    pub fn rounds(&self) -> &[u8] {
        &self.rounds
    }
    pub fn rules(&self) -> &Rules {
        &self.rules
    }
    /// Leader for the crown (match-flow.md 12): most rounds, then points;
    /// None when tied.
    pub fn leader(&self) -> Option<u8> {
        let n = self.rounds.len();
        let mut best: Option<u8> = None;
        let mut tie = false;
        for t in 0..n {
            match best {
                None => best = Some(t as u8),
                Some(b) => {
                    let (rb, pb) = (self.rounds[b as usize], self.points[b as usize]);
                    let (rt, pt) = (self.rounds[t], self.points[t]);
                    if (rt, pt) > (rb, pb) {
                        best = Some(t as u8);
                        tie = false;
                    } else if (rt, pt) == (rb, pb) {
                        tie = true;
                    }
                }
            }
        }
        if tie { None } else { best }
    }

    /// The card effect objects of a player (`AddObjectToPlayer`), in pick order.
    pub fn player_effects(&self, player: usize) -> impl Iterator<Item = &SpawnedObject> {
        self.players[player]
            .effects
            .iter()
            .filter_map(|id| self.obj_index(*id).map(|i| &self.objects[i]))
    }

    /// Fill of an effect object's ring/counter in [0, 1] for drawing: the
    /// Abyssal/Divine Countdown ring (`AbyssalCountdown.counter`, 1 while in
    /// the form and draining) or the Healing Field's `CounterUI`.
    pub fn effect_progress(&self, content: &Content, object: u32) -> Option<f32> {
        use crate::script::{CompState, Kind};
        let o = &self.objects[self.obj_index(object)?];
        let st = o.st.as_ref()?;
        content.prefab(o.def).comps.iter().find_map(|c| {
            let s = st.comps.get(c.slot? as usize)?;
            match (&c.kind, s) {
                (Kind::AbyssalCountdown { .. }, CompState::Abyssal { counter, .. }) => {
                    Some(counter.max(0.0))
                }
                (Kind::CounterUi { .. }, CompState::Timer { t, .. }) => Some(t.max(0.0)),
                _ => None,
            }
        })
    }

    /// `activeInHierarchy` of a node of an object (presentation of effect
    /// objects whose parts are switched on and off).
    pub fn object_node_active(&self, content: &Content, object: u32, node: u16) -> bool {
        let Some(i) = self.obj_index(object) else {
            return false;
        };
        let o = &self.objects[i];
        match &o.st {
            Some(st) => content.prefab(o.def).active_in_hierarchy(st.active, node),
            None => true,
        }
    }

    /// Give a card outside the pick flow (sandbox / debug). Deterministic
    /// only if every peer calls it at the same tick.
    pub fn debug_give_card(&mut self, content: &Content, player: usize, card: CardId) {
        self.apply_card(content, player, card);
    }
}
