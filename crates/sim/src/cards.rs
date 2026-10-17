//! Card hands (cards.md 3) and applying a picked card (cards.md 4).

use crate::content::{CardId, Content, SpawnEntry};
use crate::rng::Pcg32;
use crate::world::World;

/// Number of cards offered per pick.
pub const HAND_SIZE: usize = 5;

/// One weighted pick over the whole pool (cards.md 3.3).
pub fn weighted_pick(content: &Content, rng: &mut Pcg32) -> CardId {
    let total = content.pool_weight();
    let mut x = rng.range(0.0, total);
    for c in &content.pool {
        x -= content.card(*c).rarity.weight();
        if x <= 0.0 {
            return *c;
        }
    }
    *content.pool.last().expect("empty card pool")
}

/// `SpawnUniqueCard` for the next slot (cards.md 3.4). `owned` = the
/// picker's current cards.
pub fn draw_slot(content: &Content, rng: &mut Pcg32, hand: &[CardId], owned: &[CardId]) -> CardId {
    // The original redraws without a cap; a pool too small to satisfy the
    // rules would hang, so give up after many tries (accepting the last draw).
    let mut c = weighted_pick(content, rng);
    for _ in 0..10_000 {
        if !rejected(content, c, hand, owned) {
            return c;
        }
        c = weighted_pick(content, rng);
    }
    c
}

fn rejected(content: &Content, c: CardId, hand: &[CardId], owned: &[CardId]) -> bool {
    let cd = content.card(c);
    let owned_rules = |reject: &mut bool| {
        for o in owned {
            let od = content.card(*o);
            if cd
                .categories
                .iter()
                .any(|cat| od.blacklisted_categories.iter().any(|b| b == cat))
            {
                *reject = true;
            }
            if !od.allow_multiple && od.name == cd.name {
                *reject = true;
            }
        }
    };
    let mut reject = false;
    if hand.is_empty() {
        // Slot 0 quirk: never filtered (unless tuned off).
        if !content.tuning.unfiltered_first_slot {
            owned_rules(&mut reject);
        }
        return reject;
    }
    // The checks live inside the loop over already offered cards.
    for h in hand {
        if *h == c {
            reject = true;
        }
        let lock = cd.gun.as_ref().is_some_and(|g| g.lock_gun_to_default);
        if lock
            && owned.iter().any(|o| {
                content
                    .card(*o)
                    .gun
                    .as_ref()
                    .is_some_and(|g| g.lock_gun_to_default)
            })
        {
            reject = true;
        }
        owned_rules(&mut reject);
    }
    reject
}

/// Draw a full hand at once (tests, tools). The flow draws slot by slot.
pub fn draw_hand(content: &Content, rng: &mut Pcg32, owned: &[CardId]) -> Vec<CardId> {
    let mut hand = Vec::with_capacity(HAND_SIZE);
    for _ in 0..HAND_SIZE {
        let c = draw_slot(content, rng, &hand, owned);
        hand.push(c);
    }
    hand
}

/// Combine `objectsToSpawn` lists (cards.md 4.2).
pub fn merge_spawns(list: &mut Vec<SpawnEntry>, card: &[SpawnEntry]) {
    for e in card {
        let mut matched = false;
        for x in list.iter_mut() {
            let m = if e.effect.is_some() && x.effect.is_some() {
                e.effect == x.effect && x.scale_stacks
            } else if e.add_to_projectile.is_some() && x.add_to_projectile.is_some() {
                e.add_to_projectile == x.add_to_projectile && x.scale_stacks
            } else {
                false
            };
            if m {
                x.stacks += 1;
                matched = true;
                break;
            }
        }
        if !matched {
            list.push(*e);
        }
    }
}

impl World {
    /// `ApplyCardStats` for one recipient (cards.md 4), ending with a full
    /// revive.
    pub fn apply_card(&mut self, content: &Content, i: usize, card: CardId) {
        let cd = content.card(card);
        let p = &mut self.players[i];
        if let Some(g) = &cd.gun {
            p.gun.ammo_stats.merge_card(g);
        }
        p.cards.push(card);
        if let Some(g) = &cd.gun {
            p.gun.stats.merge_card(g);
            let mut list = p.gun.spawns.to_vec();
            merge_spawns(&mut list, content.card_spawns(card));
            p.gun.spawns = list.into();
        }
        if let Some(ch) = &cd.character {
            p.stats.merge_card(ch);
        }
        if let Some(b) = &cd.block {
            p.block.stats.merge_card(b);
        }
        p.configure_mass_and_size();
        // `AddObjectToPlayer` (cards.md 5.2): a new effect object, or a level
        // up of the existing one of the same prefab.
        if cd.character.is_some()
            && let Some(o) = content.card_player_object(card)
        {
            self.add_player_effect(content, i, o);
        }
        self.revive(content, i, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::test_maps::test_content;

    #[test]
    fn hands_are_distinct_after_slot0() {
        let c = test_content();
        let mut rng = Pcg32::new(1, 3);
        for _ in 0..500 {
            let h = draw_hand(&c, &mut rng, &[]);
            assert_eq!(h.len(), 5);
            for a in 0..5 {
                for b in (a + 1)..5 {
                    assert_ne!(h[a], h[b]);
                }
            }
        }
    }

    #[test]
    fn rarity_weights() {
        let c = test_content();
        let mut rng = Pcg32::new(9, 3);
        let mut counts = vec![0u32; c.cards.len()];
        let n = 200_000;
        for _ in 0..n {
            counts[weighted_pick(&c, &mut rng).0 as usize] += 1;
        }
        let total = c.pool_weight();
        for (i, k) in counts.iter().enumerate() {
            let expect = c.cards[i].rarity.weight() / total * n as f32;
            assert!(
                ((*k as f32) - expect).abs() < expect * 0.05 + 30.0,
                "card {i}: {k} vs {expect}"
            );
        }
    }

    #[test]
    fn blacklist_and_once_rules() {
        let c = test_content();
        let scatter = c.card_by_name("Scatter").unwrap();
        let bouncy = c.card_by_name("Bouncy").unwrap();
        let phoenix = c.card_by_name("Phoenix").unwrap();
        let mut rng = Pcg32::new(5, 3);
        for _ in 0..300 {
            let h = draw_hand(&c, &mut rng, &[scatter, phoenix]);
            // Slots 1..4 never offer a blacklisted or an owned once-card.
            for s in &h[1..] {
                assert_ne!(*s, bouncy);
                assert_ne!(*s, phoenix);
            }
        }
    }

    #[test]
    fn stacking_spawn_entries() {
        let c = test_content();
        let ex = c.card_by_name("Explosive").unwrap();
        let mut list = Vec::new();
        merge_spawns(&mut list, c.card_spawns(ex));
        merge_spawns(&mut list, c.card_spawns(ex));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].stacks, 1);
    }
}
