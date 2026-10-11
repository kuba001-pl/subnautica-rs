//! The outcrops (`BreakableResource`) and the luck that evens out what
//! they give (`FairRandomizer`, `PlayerEntropy`).

use super::TechType;
use crate::V3;

/// `FairRandomizer`: one per tech type in the player's `PlayerEntropy`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FairRandomizer {
    pub entropy: f32,
}

impl FairRandomizer {
    /// `CheckChance`: `draw` gives `Random.value`, asked only when the
    /// chance is strictly between 0 and 1. Every miss carries the chance
    /// over to the next try, so over many tries the hit rate is the
    /// chance.
    pub fn check_chance(&mut self, chance: f32, draw: &mut impl FnMut() -> f32) -> bool {
        if chance <= 0.0 {
            return false;
        }
        if chance >= 1.0 {
            return true;
        }
        self.entropy += chance;
        if self.entropy > draw() {
            self.entropy -= 1.0;
            true
        } else {
            false
        }
    }
}

/// `PlayerEntropy.randomizers`, in the order read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerEntropy {
    pub randomizers: Vec<(TechType, FairRandomizer)>,
}

impl PlayerEntropy {
    /// `CheckChance`: false for a tech type without a randomizer, even at
    /// a chance of 1 (the game's rule). With several for one type, all
    /// are tried and the last answers.
    pub fn check_chance(
        &mut self,
        tech: TechType,
        chance: f32,
        draw: &mut impl FnMut() -> f32,
    ) -> bool {
        let mut result = false;
        for (t, r) in &mut self.randomizers {
            if *t == tech {
                result = r.check_chance(chance, draw);
            }
        }
        result
    }
}

/// What an outcrop gives: an entry of `prefabList` or `defaultPrefab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Listed(usize),
    Default,
}

/// How the player hit (`BreakableResource.OnHandClick`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    /// Bare hand: one of `hitsToBreak`.
    Hand,
    /// A bashing tool held (`PlayBash`): the bash breaks it.
    Bash,
    /// In a Prawn suit: breaks it.
    Mech,
}

/// One outcrop's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcrop {
    /// `hitsToBreak`, counted down.
    pub hits_left: i32,
    pub broken: bool,
}

impl Outcrop {
    pub fn new(hits_to_break: i32) -> Outcrop {
        Outcrop {
            hits_left: hits_to_break,
            broken: false,
        }
    }

    /// A hit: true when it breaks now (`HitResource` reaching exactly 0,
    /// or `BreakIntoResources`; a broken outcrop never breaks again).
    pub fn hit(&mut self, kind: HitKind) -> bool {
        if kind == HitKind::Hand {
            self.hits_left = self.hits_left.saturating_sub(1);
            if self.hits_left != 0 {
                return false;
            }
        }
        if self.broken {
            return false;
        }
        self.broken = true;
        true
    }

    /// `position + up × verticalSpawnOffset`: where each resource
    /// appears.
    pub fn spawn_point(position: V3, up: V3, vertical_offset: f64) -> V3 {
        position + up * vertical_offset
    }
}

/// `BreakIntoResources`' draw: `numChances` times the first
/// `prefabList` entry whose chance passes the player's luck for its tech
/// type (`ChooseRandomResource`); if no try found one, the default once.
pub fn draw_resources(
    list: &[(TechType, f32)],
    num_chances: i32,
    luck: &mut PlayerEntropy,
    draw: &mut impl FnMut() -> f32,
) -> Vec<Resource> {
    let mut out = Vec::new();
    for _ in 0..num_chances.max(0) {
        if let Some(i) = list
            .iter()
            .position(|&(tech, chance)| luck.check_chance(tech, chance, draw))
        {
            out.push(Resource::Listed(i));
        }
    }
    if out.is_empty() {
        out.push(Resource::Default);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SplitMix64 → [0, 1).
    fn values(seed: u64) -> impl FnMut() -> f32 {
        let mut s = seed;
        move || {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    #[test]
    fn the_hit_rate_converges_on_the_chance() {
        for chance in [0.1f32, 0.25, 0.5, 0.9] {
            let mut r = FairRandomizer::default();
            let mut draw = values(7);
            let n = 100_000;
            let hits = (0..n).filter(|_| r.check_chance(chance, &mut draw)).count();
            let rate = hits as f32 / n as f32;
            assert!((rate - chance).abs() < 0.001, "{chance}: {rate}");
            // The luck stays bounded: never more than one hit owed.
            assert!(r.entropy > -1.0 && r.entropy < 1.0, "{}", r.entropy);
        }
    }

    #[test]
    fn luck_evens_out_short_runs() {
        // At 0.25, the entropy reaches 1 after four misses: the fourth try
        // at the latest hits, whatever the draws.
        let mut r = FairRandomizer::default();
        let mut worst = || 0.999_999;
        let hits: Vec<bool> = (0..8).map(|_| r.check_chance(0.25, &mut worst)).collect();
        assert_eq!(hits, [false, false, false, true, false, false, false, true]);
    }

    #[test]
    fn sure_and_impossible_chances_draw_nothing() {
        let mut r = FairRandomizer::default();
        let mut never = || -> f32 { panic!("drew") };
        assert!(r.check_chance(1.0, &mut never));
        assert!(!r.check_chance(0.0, &mut never));
        assert_eq!(r.entropy, 0.0);
    }

    #[test]
    fn the_outcrop_falls_back_to_the_default() {
        let mut luck = PlayerEntropy {
            randomizers: vec![(5, FairRandomizer::default())],
        };
        let mut low = || 0.0;
        let mut high = || 0.999;
        // Tech 6 has no randomizer: never drawn, even at chance 1.
        assert_eq!(
            draw_resources(&[(6, 1.0)], 1, &mut luck, &mut low),
            [Resource::Default]
        );
        assert_eq!(
            draw_resources(&[(6, 1.0), (5, 0.5)], 1, &mut luck, &mut low),
            [Resource::Listed(1)]
        );
        // Entropy now -0.5: 0.5 more is 0, not above 0.999.
        assert_eq!(
            draw_resources(&[(5, 0.5)], 1, &mut luck, &mut high),
            [Resource::Default]
        );
        assert_eq!(
            draw_resources(&[(5, 1.0)], 3, &mut luck, &mut high),
            [Resource::Listed(0); 3]
        );
        assert_eq!(
            draw_resources(&[], -2, &mut luck, &mut low),
            [Resource::Default]
        );
    }

    #[test]
    fn hits_break_at_exactly_zero() {
        let mut o = Outcrop::new(3);
        assert!(!o.hit(HitKind::Hand));
        assert!(!o.hit(HitKind::Hand));
        assert!(o.hit(HitKind::Hand));
        assert!(!o.hit(HitKind::Hand));
        assert!(!o.hit(HitKind::Bash));
        // Starting at 0 the hand never breaks it; a bash does.
        let mut o = Outcrop::new(0);
        assert!(!o.hit(HitKind::Hand));
        assert!(o.hit(HitKind::Bash));
        assert!(Outcrop::new(5).hit(HitKind::Mech));
    }
}
