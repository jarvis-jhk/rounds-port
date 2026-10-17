//! Stat blocks: the numbers a card carries in its `Gun`, `GunAmmo`-related,
//! `CharacterStatModifiers` and `Block` components (cards.md 2, 4), and the
//! player's runtime copies they are merged into.
//!
//! Serde field names are the original's serialized field names (typos kept),
//! and booleans accept Unity's `0/1`, so the importer's JSON for a component
//! deserialises directly (`serde_json::from_value::<GunStats>(component)`).

use serde::{Deserialize, Deserializer, Serialize};

/// Accept `true/false` or `0/1`.
pub(crate) fn de_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    #[allow(clippy::enum_variant_names)]
    enum B {
        B(bool),
        I(i64),
        F(f64),
    }
    Ok(match B::deserialize(d)? {
        B::B(b) => b,
        B::I(i) => i != 0,
        B::F(f) => f != 0.0,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Rgba {
    fn default() -> Self {
        Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }
}

impl Rgba {
    pub fn is_black(&self) -> bool {
        self.r == 0.0 && self.g == 0.0 && self.b == 0.0
    }
}

/// `Gun` stat container (weapons.md 3). `Default` = the neutral values found
/// on a card's gun (multipliers 1, adders 0); [`GunStats::player_reset`] is
/// the player's gun after `ResetStats`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GunStats {
    #[serde(rename = "damage")]
    pub damage: f32,
    /// On a card: multiplies the player's reload-time multiplier (weapons.md 4).
    #[serde(rename = "reloadTime")]
    pub reload_time: f32,
    #[serde(rename = "reloadTimeAdd")]
    pub reload_time_add: f32,
    #[serde(rename = "recoilMuiltiplier")]
    pub recoil_multiplier: f32,
    #[serde(rename = "knockback")]
    pub knockback: f32,
    /// Cooldown in game seconds on the player's gun; multiplier on cards.
    #[serde(rename = "attackSpeed")]
    pub attack_speed: f32,
    #[serde(rename = "projectileSpeed")]
    pub projectile_speed: f32,
    #[serde(rename = "projectielSimulatonSpeed")]
    pub projectile_simulation_speed: f32,
    #[serde(rename = "gravity")]
    pub gravity: f32,
    #[serde(rename = "damageAfterDistanceMultiplier")]
    pub damage_after_distance_multiplier: f32,
    #[serde(rename = "bulletDamageMultiplier")]
    pub bullet_damage_multiplier: f32,
    #[serde(rename = "multiplySpread")]
    pub multiply_spread: f32,
    #[serde(rename = "shakeM")]
    pub shake_m: f32,
    /// Added to the magazine (card).
    #[serde(rename = "ammo")]
    pub ammo: i32,
    #[serde(rename = "ammoReg")]
    pub ammo_reg: f32,
    #[serde(rename = "size")]
    pub size: f32,
    #[serde(rename = "overheatMultiplier")]
    pub overheat_multiplier: f32,
    #[serde(rename = "timeToReachFullMovementMultiplier")]
    pub time_to_reach_full_movement_multiplier: f32,
    #[serde(rename = "numberOfProjectiles")]
    pub number_of_projectiles: i32,
    #[serde(rename = "bursts")]
    pub bursts: i32,
    #[serde(rename = "reflects")]
    pub reflects: i32,
    #[serde(rename = "smartBounce")]
    pub smart_bounce: i32,
    #[serde(rename = "bulletPortal")]
    pub bullet_portal: i32,
    #[serde(rename = "randomBounces")]
    pub random_bounces: i32,
    #[serde(rename = "timeBetweenBullets")]
    pub time_between_bullets: f32,
    #[serde(rename = "projectileSize")]
    pub projectile_size: f32,
    #[serde(rename = "speedMOnBounce")]
    pub speed_m_on_bounce: f32,
    #[serde(rename = "dmgMOnBounce")]
    pub dmg_m_on_bounce: f32,
    #[serde(rename = "drag")]
    pub drag: f32,
    #[serde(rename = "dragMinSpeed")]
    pub drag_min_speed: f32,
    #[serde(rename = "spread")]
    pub spread: f32,
    #[serde(rename = "evenSpread")]
    pub even_spread: f32,
    #[serde(rename = "percentageDamage")]
    pub percentage_damage: f32,
    #[serde(rename = "cos")]
    pub cos: f32,
    #[serde(rename = "slow")]
    pub slow: f32,
    #[serde(rename = "chargeDamageMultiplier")]
    pub charge_damage_multiplier: f32,
    #[serde(rename = "chargeSpreadTo")]
    pub charge_spread_to: f32,
    #[serde(rename = "chargeEvenSpreadTo")]
    pub charge_even_spread_to: f32,
    #[serde(rename = "chargeSpeedTo")]
    pub charge_speed_to: f32,
    #[serde(rename = "chargeRecoilTo")]
    pub charge_recoil_to: f32,
    #[serde(rename = "chargeNumberOfProjectilesTo")]
    pub charge_number_of_projectiles_to: f32,
    #[serde(rename = "destroyBulletAfter")]
    pub destroy_bullet_after: f32,
    #[serde(rename = "forceSpecificAttackSpeed")]
    pub force_specific_attack_speed: f32,
    #[serde(rename = "defaultCooldown")]
    pub default_cooldown: f32,
    #[serde(rename = "lockGunToDefault", deserialize_with = "de_bool")]
    pub lock_gun_to_default: bool,
    #[serde(rename = "unblockable", deserialize_with = "de_bool")]
    pub unblockable: bool,
    #[serde(rename = "ignoreWalls", deserialize_with = "de_bool")]
    pub ignore_walls: bool,
    #[serde(rename = "useCharge", deserialize_with = "de_bool")]
    pub use_charge: bool,
    #[serde(rename = "dontAllowAutoFire", deserialize_with = "de_bool")]
    pub dont_allow_auto_fire: bool,
    #[serde(rename = "projectileColor")]
    pub projectile_color: Rgba,
    #[serde(rename = "bodyRecoil")]
    pub body_recoil: f32,
    /// Visual recoil impulse of the held gun (player.md 4.4).
    #[serde(rename = "recoil")]
    pub recoil: f32,
    #[serde(rename = "shake")]
    pub shake: f32,
}

impl Default for GunStats {
    fn default() -> Self {
        GunStats {
            damage: 1.0,
            reload_time: 1.0,
            reload_time_add: 0.0,
            recoil_multiplier: 1.0,
            knockback: 1.0,
            attack_speed: 1.0,
            projectile_speed: 1.0,
            projectile_simulation_speed: 1.0,
            gravity: 1.0,
            damage_after_distance_multiplier: 1.0,
            bullet_damage_multiplier: 1.0,
            multiply_spread: 1.0,
            shake_m: 1.0,
            ammo: 0,
            ammo_reg: 0.0,
            size: 0.0,
            overheat_multiplier: 0.0,
            time_to_reach_full_movement_multiplier: 0.0,
            number_of_projectiles: 0,
            bursts: 0,
            reflects: 0,
            smart_bounce: 0,
            bullet_portal: 0,
            random_bounces: 0,
            time_between_bullets: 0.0,
            projectile_size: 0.0,
            speed_m_on_bounce: 1.0,
            dmg_m_on_bounce: 1.0,
            drag: 0.0,
            drag_min_speed: 1.0,
            spread: 0.0,
            even_spread: 0.0,
            percentage_damage: 0.0,
            cos: 0.0,
            slow: 0.0,
            charge_damage_multiplier: 1.0,
            charge_spread_to: 0.0,
            charge_even_spread_to: 0.0,
            charge_speed_to: 0.0,
            charge_recoil_to: 0.0,
            charge_number_of_projectiles_to: 0.0,
            destroy_bullet_after: 0.0,
            force_specific_attack_speed: 0.0,
            default_cooldown: 1.0,
            lock_gun_to_default: false,
            unblockable: false,
            ignore_walls: false,
            use_charge: false,
            dont_allow_auto_fire: false,
            projectile_color: Rgba::default(),
            body_recoil: 0.0,
            recoil: 0.0,
            shake: 0.0,
        }
    }
}

impl GunStats {
    /// The player's gun after `ResetStats` (weapons.md 3, cards.md 4.3).
    pub fn player_reset() -> GunStats {
        GunStats {
            attack_speed: 0.3,
            number_of_projectiles: 1,
            recoil: 150.0,
            // The player gun's serialized `shake` (screen shake only).
            shake: 1.0,
            ..GunStats::default()
        }
    }

    /// `CopyGunStats` (cards.md 4.1), except `objectsToSpawn` (handled by the
    /// caller, 4.2) and cosmetic sound modifiers.
    pub fn merge_card(&mut self, c: &GunStats) {
        let p = self;
        if c.unblockable {
            p.unblockable = true;
        }
        if c.ignore_walls {
            p.ignore_walls = true;
        }
        let mut num = 1.0f32;
        if c.number_of_projectiles != 0 && p.number_of_projectiles != 1 {
            num = c.number_of_projectiles as f32
                / (c.number_of_projectiles + p.number_of_projectiles) as f32;
        }
        p.damage *= 1.0 - num * (1.0 - c.damage);
        if p.damage < 0.25 {
            p.damage = 0.25;
        }
        p.knockback *= 1.0 - num * (1.0 - c.knockback);
        let mut num2 = 1.0f32;
        if c.charge_number_of_projectiles_to != 0.0 && p.charge_number_of_projectiles_to != 0.0 {
            num2 = c.charge_number_of_projectiles_to
                / (c.charge_number_of_projectiles_to + p.charge_number_of_projectiles_to);
        }
        p.charge_damage_multiplier *= 1.0 - num2 * (1.0 - c.charge_damage_multiplier);
        p.size += c.size;
        p.projectile_speed *= c.projectile_speed;
        p.projectile_simulation_speed *= c.projectile_simulation_speed;
        p.gravity *= c.gravity;
        p.multiply_spread *= c.multiply_spread;
        p.attack_speed *= c.attack_speed;
        p.body_recoil *= c.recoil_multiplier;
        p.speed_m_on_bounce *= c.speed_m_on_bounce;
        p.dmg_m_on_bounce *= c.dmg_m_on_bounce;
        p.bullet_damage_multiplier *= c.bullet_damage_multiplier;
        p.spread += c.spread;
        p.drag += c.drag;
        p.time_between_bullets += c.time_between_bullets;
        p.drag_min_speed += c.drag_min_speed;
        p.even_spread += c.even_spread;
        p.number_of_projectiles += c.number_of_projectiles;
        p.reflects += c.reflects;
        p.smart_bounce += c.smart_bounce;
        p.bullet_portal += c.bullet_portal;
        p.random_bounces += c.random_bounces;
        p.bursts += c.bursts;
        p.slow += c.slow;
        p.overheat_multiplier += c.overheat_multiplier;
        p.projectile_size += c.projectile_size;
        p.percentage_damage += c.percentage_damage;
        p.damage_after_distance_multiplier *= c.damage_after_distance_multiplier;
        p.time_to_reach_full_movement_multiplier *= c.time_to_reach_full_movement_multiplier;
        p.cos += c.cos;
        if c.dont_allow_auto_fire {
            p.dont_allow_auto_fire = true;
        }
        if c.destroy_bullet_after != 0.0 {
            p.destroy_bullet_after = c.destroy_bullet_after;
        }
        p.charge_spread_to += c.charge_spread_to;
        p.charge_speed_to += c.charge_speed_to;
        p.charge_even_spread_to += c.charge_even_spread_to;
        p.charge_number_of_projectiles_to += c.charge_number_of_projectiles_to;
        p.charge_recoil_to += c.charge_recoil_to;
        if c.use_charge {
            p.use_charge = true;
        }
        if c.lock_gun_to_default {
            p.default_cooldown = c.force_specific_attack_speed;
            p.lock_gun_to_default = true;
        }
        if !c.projectile_color.is_black() {
            if p.projectile_color.is_black() {
                p.projectile_color = c.projectile_color;
            }
            let m = |a: f32, b: f32| crate::math::sqrt((a * a + b * b) * 0.5);
            let (r, g, b) = (
                m(p.projectile_color.r, c.projectile_color.r),
                m(p.projectile_color.g, c.projectile_color.g),
                m(p.projectile_color.b, c.projectile_color.b),
            );
            let (r, g, b) = saturate_hsv(r, g, b);
            p.projectile_color = Rgba {
                r,
                g,
                b,
                a: p.projectile_color.a,
            };
        }
    }
}

/// HSV with S = V = 1, back to RGB (cosmetic trail colour, cards.md 4.1).
fn saturate_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    let x = 1.0 - ((h % 2.0) - 1.0).abs();
    match h as i32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    }
}

/// The player's magazine parameters (`GunAmmo`, weapons.md 4).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmmoStats {
    #[serde(rename = "maxAmmo")]
    pub max_ammo: i32,
    #[serde(rename = "reloadTime")]
    pub reload_time: f32,
    #[serde(rename = "reloadTimeMultiplier")]
    pub reload_time_multiplier: f32,
    #[serde(rename = "reloadTimeAdd")]
    pub reload_time_add: f32,
    #[serde(rename = "ammoReg")]
    pub ammo_reg: f32,
}

impl Default for AmmoStats {
    fn default() -> Self {
        AmmoStats {
            max_ammo: 3,
            reload_time: 2.0,
            reload_time_multiplier: 1.0,
            reload_time_add: 0.0,
            ammo_reg: 0.0,
        }
    }
}

impl AmmoStats {
    /// `(reloadTime + reloadTimeAdd) * reloadTimeMultiplier`.
    pub fn reload_duration(&self) -> f32 {
        (self.reload_time + self.reload_time_add) * self.reload_time_multiplier
    }

    /// Card step 1 (cards.md 4).
    pub fn merge_card(&mut self, c: &GunStats) {
        self.max_ammo = (self.max_ammo + c.ammo).clamp(1, 90);
        self.ammo_reg += c.ammo_reg;
        self.reload_time_multiplier *= c.reload_time;
        self.reload_time_add += c.reload_time_add;
    }
}

/// `CharacterStatModifiers` on a card (cards.md 4 step 4). `Default` = neutral.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterStats {
    #[serde(rename = "sizeMultiplier")]
    pub size_multiplier: f32,
    /// Multiplier on MaxHealth.
    #[serde(rename = "health")]
    pub health: f32,
    #[serde(rename = "movementSpeed")]
    pub movement_speed: f32,
    #[serde(rename = "jump")]
    pub jump: f32,
    #[serde(rename = "gravity")]
    pub gravity: f32,
    #[serde(rename = "secondsToTakeDamageOver")]
    pub seconds_to_take_damage_over: f32,
    #[serde(rename = "numberOfJumps")]
    pub number_of_jumps: i32,
    #[serde(rename = "regen")]
    pub regen: f32,
    #[serde(rename = "lifeSteal")]
    pub life_steal: f32,
    #[serde(rename = "refreshOnDamage", deserialize_with = "de_bool")]
    pub refresh_on_damage: bool,
    #[serde(rename = "automaticReload", deserialize_with = "de_bool")]
    pub automatic_reload: bool,
    #[serde(rename = "respawns")]
    pub respawns: i32,
    /// Prefab name of the effect object added to the player (`AddObjectToPlayer`).
    #[serde(rename = "AddObjectToPlayer")]
    pub add_object_to_player: Option<String>,
}

impl Default for CharacterStats {
    fn default() -> Self {
        CharacterStats {
            size_multiplier: 1.0,
            health: 1.0,
            movement_speed: 1.0,
            jump: 1.0,
            gravity: 1.0,
            seconds_to_take_damage_over: 0.0,
            number_of_jumps: 0,
            regen: 0.0,
            life_steal: 0.0,
            refresh_on_damage: false,
            automatic_reload: true,
            respawns: 0,
            add_object_to_player: None,
        }
    }
}

/// `Block` stats on a card (cards.md 4 step 5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BlockCardStats {
    #[serde(rename = "cdMultiplier")]
    pub cd_multiplier: f32,
    #[serde(rename = "cdAdd")]
    pub cd_add: f32,
    #[serde(rename = "forceToAdd")]
    pub force_to_add: f32,
    #[serde(rename = "forceToAddUp")]
    pub force_to_add_up: f32,
    #[serde(rename = "additionalBlocks")]
    pub additional_blocks: i32,
    #[serde(rename = "healing")]
    pub healing: f32,
    #[serde(rename = "autoBlock", deserialize_with = "de_bool")]
    pub auto_block: bool,
    /// Prefab names spawned at the player on every block.
    #[serde(rename = "objectsToSpawn")]
    pub objects_to_spawn: Vec<String>,
}

impl Default for BlockCardStats {
    fn default() -> Self {
        BlockCardStats {
            cd_multiplier: 1.0,
            cd_add: 0.0,
            force_to_add: 0.0,
            force_to_add_up: 0.0,
            additional_blocks: 0,
            healing: 0.0,
            auto_block: false,
            objects_to_spawn: Vec::new(),
        }
    }
}

/// The player's block parameters (player.md 12.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlockStats {
    pub cooldown: f32,
    pub cd_multiplier: f32,
    pub cd_add: f32,
    pub force_to_add: f32,
    pub force_to_add_up: f32,
    pub additional_blocks: i32,
    pub healing: f32,
    pub auto_block: bool,
    pub delay_other_actions: bool,
}

impl Default for BlockStats {
    fn default() -> Self {
        BlockStats {
            cooldown: 4.0,
            cd_multiplier: 1.0,
            cd_add: 0.0,
            force_to_add: 0.0,
            force_to_add_up: 0.0,
            additional_blocks: 0,
            healing: 0.0,
            auto_block: false,
            delay_other_actions: false,
        }
    }
}

impl BlockStats {
    pub fn cooldown(&self) -> f32 {
        (self.cooldown + self.cd_add) * self.cd_multiplier
    }
    pub fn merge_card(&mut self, c: &BlockCardStats) {
        self.cd_multiplier *= c.cd_multiplier;
        self.cd_add += c.cd_add;
        self.force_to_add += c.force_to_add;
        self.force_to_add_up += c.force_to_add_up;
        self.additional_blocks += c.additional_blocks;
        self.healing += c.healing;
        if c.auto_block {
            self.auto_block = true;
        }
    }
}

/// The player's own `CharacterStatModifiers` + related fields (player.md 14).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerStats {
    pub size_multiplier: f32,
    pub max_health: f32,
    pub movement_speed: f32,
    pub jump: f32,
    /// `Gravity.gravityForce` (1000 * product of card gravity).
    pub gravity_force: f32,
    pub jumps: i32,
    pub regeneration: f32,
    pub life_steal: f32,
    pub respawns: i32,
    pub seconds_to_take_damage_over: f32,
    pub refresh_on_damage: bool,
    pub automatic_reload: bool,
    pub taste_of_blood_speed: f32,
    pub rage_speed: f32,
}

impl Default for PlayerStats {
    fn default() -> Self {
        PlayerStats {
            size_multiplier: 1.0,
            max_health: 100.0,
            movement_speed: 1.0,
            jump: 1.0,
            gravity_force: 1000.0,
            jumps: 1,
            regeneration: 0.0,
            life_steal: 0.0,
            respawns: 0,
            seconds_to_take_damage_over: 0.0,
            refresh_on_damage: false,
            automatic_reload: true,
            taste_of_blood_speed: 1.0,
            rage_speed: 1.0,
        }
    }
}

impl PlayerStats {
    pub fn attack_speed_multiplier(&self) -> f32 {
        self.taste_of_blood_speed * self.rage_speed
    }

    /// Card step 4 (cards.md 4).
    pub fn merge_card(&mut self, c: &CharacterStats) {
        self.size_multiplier *= c.size_multiplier;
        self.max_health *= c.health;
        self.movement_speed *= c.movement_speed;
        self.jump *= c.jump;
        self.jumps += c.number_of_jumps;
        self.gravity_force *= c.gravity;
        self.regeneration += c.regen;
        self.life_steal += c.life_steal;
        self.respawns += c.respawns;
        self.seconds_to_take_damage_over += c.seconds_to_take_damage_over;
        if c.refresh_on_damage {
            self.refresh_on_damage = true;
        }
        if !c.automatic_reload {
            self.automatic_reload = false;
        }
    }

    /// `ConfigureMassAndSize` (player.md 11.1): (scale, mass).
    pub fn scale_and_mass(&self) -> (f32, f32) {
        let h = self.max_health / 100.0 * 1.2;
        let scale = 1.2 * crate::math::pow(h, 0.2) * self.size_multiplier;
        let mass = 100.0 * crate::math::pow(h, 0.8) * self.size_multiplier;
        (scale, mass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_mass_defaults() {
        let (s, m) = PlayerStats::default().scale_and_mass();
        assert!((s - 1.2446).abs() < 1e-3, "{s}");
        assert!((m - 115.70).abs() < 1e-2, "{m}");
    }

    #[test]
    fn projectile_count_damping() {
        // Player with 5 projectiles, card adds 4 with damage factor 0.3: 0.689.
        let mut p = GunStats::player_reset();
        p.number_of_projectiles = 5;
        let c = GunStats {
            number_of_projectiles: 4,
            damage: 0.3,
            ..GunStats::default()
        };
        p.merge_card(&c);
        assert!(
            (p.damage - (1.0 - 4.0 / 9.0 * 0.7)).abs() < 1e-5,
            "{}",
            p.damage
        );
        assert_eq!(p.number_of_projectiles, 9);
        // First projectile-adding card applies in full.
        let mut p = GunStats::player_reset();
        p.merge_card(&c);
        assert!((p.damage - 0.3).abs() < 1e-6);
        // Damage floor.
        let mut p = GunStats::player_reset();
        p.merge_card(&GunStats {
            damage: 0.1,
            ..GunStats::default()
        });
        assert_eq!(p.damage, 0.25);
    }

    #[test]
    fn drag_min_speed_grows_per_card() {
        let mut p = GunStats::player_reset();
        p.merge_card(&GunStats::default());
        p.merge_card(&GunStats::default());
        assert_eq!(p.drag_min_speed, 3.0);
    }

    #[test]
    fn deserialize_unity_shape() {
        let j = serde_json::json!({"damage": 0.4, "numberOfProjectiles": 4, "lockGunToDefault": 0, "ignoreWalls": 1,
            "projectielSimulatonSpeed": 2.0, "unknownField": 5});
        let g: GunStats = serde_json::from_value(j).unwrap();
        assert_eq!(g.number_of_projectiles, 4);
        assert!(g.ignore_walls && !g.lock_gun_to_default);
        assert_eq!(g.projectile_simulation_speed, 2.0);
        assert_eq!(g.knockback, 1.0);
    }

    #[test]
    fn ammo_merge() {
        let mut a = AmmoStats::default();
        a.merge_card(&GunStats {
            ammo: 100,
            reload_time: 0.7,
            reload_time_add: 0.25,
            ..GunStats::default()
        });
        assert_eq!(a.max_ammo, 90);
        assert!((a.reload_duration() - 2.25 * 0.7).abs() < 1e-5);
    }
}
