//! Tamagotchi-style "care" state for the pet: three needs that decay over real wall-clock time, a
//! monotonic XP track that unlocks growth stages, currency earned from real agent usage, and a
//! small fixed cosmetic shop. Persisted next to `chat.json`/`stats.json` (see `chat::cache_dir`),
//! same load/save convention as `stats.rs`.
//!
//! No action here can ever make the pet worse off than "visibly neglected": needs never drop below
//! `NEED_FLOOR`, nothing purchased is ever lost, and growth/milestones only ever go up — by design,
//! there is no punishment for leaving the pet alone.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Needs never drop below this: there is no "losing" the pet, only a visibly needy one.
pub const NEED_FLOOR: f64 = 20.0;
pub const NEED_CEIL: f64 = 100.0;

/// Hours to fully decay from `NEED_CEIL` to `NEED_FLOOR`, per need.
const HUNGER_DECAY_HOURS: f64 = 10.0;
const ENERGY_DECAY_HOURS: f64 = 16.0;
const AFFECTION_DECAY_HOURS: f64 = 24.0;

/// XP thresholds for growth stages, each celebrated once (mirrors `Stats`'s streak/tool-call
/// milestones): tier 1 ("just hatched") through tier 5 ("fully grown").
pub const GROWTH_TIERS: [u64; 5] = [50, 150, 400, 900, 2000];

/// +1 currency per this many tool calls watched (real agent usage, not care actions).
const TOOL_CALLS_PER_COIN: u64 = 20;

/// How often a mini-game round can earn a reward: a rate limit against farming, not a punishment.
pub const PLAY_COOLDOWN_MS: u64 = 5 * 60 * 1000;

/// How often each of feed/pet/nap can move its need: without this, clicking the same action
/// over and over maxes it out in seconds instead of it being a real decision over the day (the
/// bug this guards against). Short enough to still feel responsive, long enough that farming it
/// isn't worth the clicks.
pub const CARE_COOLDOWN_MS: u64 = 90 * 1000;

/// id -> cost, the only accessories the shop will ever sell. Mirrored for display in
/// `app/ui/js/panel.js` and `plugin/panel.luau`; kept here too so a tampered client can't grant
/// itself a free item.
pub const ACCESSORIES: &[(&str, u64)] = &[
    ("party_hat", 20),
    ("bow", 20),
    ("sunglasses", 30),
    ("scarf", 30),
    ("headphones", 40),
    ("monocle", 45),
    ("flower_crown", 50),
    ("chef_hat", 50),
    ("wizard_hat", 60),
    ("top_hat", 65),
    ("crown", 70),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Care {
    pub hunger: f64,
    pub energy: f64,
    pub affection: f64,
    /// Heartbeat for elapsed-time decay: set on first use, so a brand-new file doesn't instantly
    /// age, and kept across daemon restarts so decay resumes from real elapsed time.
    pub last_tick_ms: u64,
    /// Earned by care actions only (feed/pet/nap/play); only ever increases.
    pub xp: u64,
    /// Earned by real agent usage only (tool calls), spent in the shop.
    pub currency: u64,
    /// The `total_tool_calls` baseline currency was last derived from.
    pub tool_calls_counted: u64,
    pub owned: BTreeSet<String>,
    pub equipped: Option<String>,
    pub last_play_ms: u64,
    /// Heartbeats for `CARE_COOLDOWN_MS`, one per action so feeding doesn't also gate petting.
    pub last_feed_ms: u64,
    pub last_pet_ms: u64,
    pub last_nap_ms: u64,
    /// The highest growth tier already celebrated (see `check_growth`), so each fires once.
    pub growth_celebrated: u64,
}

impl Default for Care {
    fn default() -> Self {
        Care {
            hunger: NEED_CEIL,
            energy: NEED_CEIL,
            affection: NEED_CEIL,
            last_tick_ms: 0,
            xp: 0,
            currency: 0,
            tool_calls_counted: 0,
            owned: BTreeSet::new(),
            equipped: None,
            last_play_ms: 0,
            last_feed_ms: 0,
            last_pet_ms: 0,
            last_nap_ms: 0,
            growth_celebrated: 0,
        }
    }
}

impl Care {
    pub fn load(path: &Path) -> Care {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Apply elapsed-time decay since `last_tick_ms`. Returns true if anything was worth saving
    /// (including just having set the heartbeat for the first time).
    pub fn decay(&mut self, now_ms: u64) -> bool {
        if self.last_tick_ms == 0 {
            self.last_tick_ms = now_ms;
            return true;
        }
        let elapsed_hours = now_ms.saturating_sub(self.last_tick_ms) as f64 / 3_600_000.0;
        if elapsed_hours <= 0.0 {
            return false;
        }
        self.last_tick_ms = now_ms;
        let span = NEED_CEIL - NEED_FLOOR;
        let drop = |decay_hours: f64| (span / decay_hours) * elapsed_hours;
        self.hunger = (self.hunger - drop(HUNGER_DECAY_HOURS)).max(NEED_FLOOR);
        self.energy = (self.energy - drop(ENERGY_DECAY_HOURS)).max(NEED_FLOOR);
        self.affection = (self.affection - drop(AFFECTION_DECAY_HOURS)).max(NEED_FLOOR);
        true
    }

    /// The highest growth tier `xp` reaches under `GROWTH_TIERS`, 0 if none — an earned stage, so
    /// it can only go up even if nothing happens for a while.
    pub fn growth_tier(&self) -> u64 {
        GROWTH_TIERS.iter().filter(|&&t| self.xp >= t).count() as u64
    }

    /// Fires once, the moment `xp` first crosses into a new tier.
    fn check_growth(&mut self) -> Option<u64> {
        let tier = self.growth_tier();
        if tier > self.growth_celebrated {
            self.growth_celebrated = tier;
            Some(tier)
        } else {
            None
        }
    }

    fn gain_xp(&mut self, n: u64) -> Option<u64> {
        self.xp += n;
        self.check_growth()
    }

    /// Rate-limited against `CARE_COOLDOWN_MS` (see its doc comment for why): `Err` names why a
    /// too-soon click was refused, same shape as `play`.
    pub fn feed(&mut self, now_ms: u64) -> Result<Option<u64>, &'static str> {
        if self.last_feed_ms != 0 && now_ms.saturating_sub(self.last_feed_ms) < CARE_COOLDOWN_MS {
            return Err("still full from the last snack");
        }
        self.last_feed_ms = now_ms;
        self.hunger = (self.hunger + 35.0).min(NEED_CEIL);
        Ok(self.gain_xp(5))
    }

    pub fn pet(&mut self, now_ms: u64) -> Result<Option<u64>, &'static str> {
        if self.last_pet_ms != 0 && now_ms.saturating_sub(self.last_pet_ms) < CARE_COOLDOWN_MS {
            return Err("already had plenty of cuddles");
        }
        self.last_pet_ms = now_ms;
        self.affection = (self.affection + 25.0).min(NEED_CEIL);
        Ok(self.gain_xp(5))
    }

    /// A one-shot "sent for a nap" boost (not real sleep-duration tracking).
    pub fn nap_boost(&mut self, now_ms: u64) -> Result<Option<u64>, &'static str> {
        if self.last_nap_ms != 0 && now_ms.saturating_sub(self.last_nap_ms) < CARE_COOLDOWN_MS {
            return Err("just woke up from a nap");
        }
        self.last_nap_ms = now_ms;
        self.energy = (self.energy + 30.0).min(NEED_CEIL);
        Ok(self.gain_xp(5))
    }

    /// A finished mini-game round: `score` (0..=100) converts to a currency reward, rate-limited
    /// so it can't be farmed. `Err` names why it was refused.
    pub fn play(&mut self, score: u32, now_ms: u64) -> Result<Option<u64>, &'static str> {
        if self.last_play_ms != 0 && now_ms.saturating_sub(self.last_play_ms) < PLAY_COOLDOWN_MS {
            return Err("the mini-game is still cooling down");
        }
        self.last_play_ms = now_ms;
        let score = score.min(100) as u64;
        self.currency += 1 + score / 10; // 1..=11 coins
        self.energy = (self.energy - 5.0).max(NEED_FLOOR);
        self.affection = (self.affection + 10.0).min(NEED_CEIL);
        Ok(self.gain_xp(10))
    }

    /// Currency earned passively from real agent usage: `+1` per `TOOL_CALLS_PER_COIN` tool calls
    /// seen since the last award (same delta style as `Stats::check_milestones`).
    pub fn accrue_currency(&mut self, total_tool_calls: u64) -> u64 {
        let earned = total_tool_calls.saturating_sub(self.tool_calls_counted) / TOOL_CALLS_PER_COIN;
        if earned > 0 {
            self.tool_calls_counted += earned * TOOL_CALLS_PER_COIN;
            self.currency += earned;
        }
        earned
    }

    pub fn buy(&mut self, id: &str) -> Result<(), &'static str> {
        let Some(&(_, cost)) = ACCESSORIES.iter().find(|(aid, _)| *aid == id) else {
            return Err("unknown accessory");
        };
        if self.owned.contains(id) {
            return Err("already owned");
        }
        if self.currency < cost {
            return Err("not enough currency");
        }
        self.currency -= cost;
        self.owned.insert(id.to_string());
        Ok(())
    }

    /// Equip `id`, or `""` to go bare.
    pub fn equip(&mut self, id: &str) -> Result<(), &'static str> {
        if id.is_empty() {
            self.equipped = None;
            return Ok(());
        }
        if !self.owned.contains(id) {
            return Err("not owned");
        }
        self.equipped = Some(id.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_is_proportional_to_elapsed_time_and_floors() {
        let mut c = Care::default();
        assert!(c.decay(1_000)); // first call just sets the heartbeat
        assert_eq!((c.hunger, c.energy, c.affection), (NEED_CEIL, NEED_CEIL, NEED_CEIL));
        let five_hours_ms = 5 * 3_600_000;
        assert!(c.decay(1_000 + five_hours_ms));
        // Hunger decays fastest (10h to floor): half its span should be gone after 5h.
        assert!((c.hunger - 60.0).abs() < 0.01, "hunger={}", c.hunger);
        // A huge gap (e.g. the daemon was off for a week) still only floors, never goes negative.
        assert!(c.decay(1_000 + five_hours_ms + 1_000 * 3_600_000));
        assert_eq!((c.hunger, c.energy, c.affection), (NEED_FLOOR, NEED_FLOOR, NEED_FLOOR));
    }

    #[test]
    fn decay_with_no_elapsed_time_is_a_noop_after_the_first_call() {
        let mut c = Care::default();
        c.decay(5_000);
        assert!(!c.decay(5_000));
    }

    #[test]
    fn feed_pet_nap_raise_their_need_and_xp_without_exceeding_the_ceiling() {
        let mut c = Care { hunger: 90.0, ..Care::default() };
        assert_eq!(c.feed(1_000), Ok(None)); // not enough xp yet to cross tier 1
        assert_eq!(c.hunger, NEED_CEIL);
        assert_eq!(c.xp, 5);
        c.affection = 50.0;
        c.pet(1_000).unwrap();
        assert_eq!(c.affection, 75.0);
        c.energy = 50.0;
        c.nap_boost(1_000).unwrap();
        assert_eq!(c.energy, 80.0);
    }

    #[test]
    fn feed_pet_nap_rate_limit_independently_of_each_other() {
        let mut c = Care { hunger: 50.0, affection: 50.0, energy: 50.0, ..Care::default() };
        assert!(c.feed(1_000).is_ok());
        assert_eq!(c.feed(1_000 + CARE_COOLDOWN_MS - 1), Err("still full from the last snack"));
        // Petting and napping are on their own clocks: still available right away.
        assert!(c.pet(1_000).is_ok());
        assert!(c.nap_boost(1_000).is_ok());
        assert!(c.feed(1_000 + CARE_COOLDOWN_MS).is_ok(), "the cooldown has fully elapsed");
    }

    #[test]
    fn growth_tiers_fire_once_and_never_go_down() {
        let mut c = Care::default();
        let mut t = 1_000;
        for _ in 0..9 {
            c.feed(t).unwrap(); // +5 xp each, 45 total
            t += CARE_COOLDOWN_MS;
        }
        assert_eq!(c.feed(t), Ok(Some(1))); // crosses 50 -> tier 1
        assert_eq!(c.growth_tier(), 1);
        t += CARE_COOLDOWN_MS;
        assert_eq!(c.feed(t), Ok(None)); // already celebrated
        assert_eq!(c.growth_tier(), 1);
    }

    #[test]
    fn currency_accrues_from_tool_call_deltas_like_milestones() {
        let mut c = Care::default();
        assert_eq!(c.accrue_currency(19), 0);
        assert_eq!(c.accrue_currency(20), 1);
        assert_eq!(c.currency, 1);
        assert_eq!(c.accrue_currency(20), 0, "already counted");
        assert_eq!(c.accrue_currency(65), 2); // 45 more calls -> 2 more coins, remainder kept
        assert_eq!(c.currency, 3);
    }

    #[test]
    fn play_rewards_and_rate_limits() {
        let mut c = Care { energy: 50.0, ..Care::default() };
        assert_eq!(c.play(100, 1_000), Ok(None));
        assert_eq!(c.currency, 11);
        assert_eq!(c.energy, 45.0);
        assert_eq!(c.play(0, 2_000), Err("the mini-game is still cooling down"));
        assert_eq!(c.currency, 11, "the refused round grants nothing");
        assert!(c.play(0, 1_000 + 5 * 60 * 1000).is_ok());
    }

    #[test]
    fn buy_validates_catalog_ownership_and_funds() {
        let mut c = Care::default();
        assert_eq!(c.buy("not_a_real_item"), Err("unknown accessory"));
        assert_eq!(c.buy("party_hat"), Err("not enough currency"));
        c.currency = 20;
        assert_eq!(c.buy("party_hat"), Ok(()));
        assert_eq!(c.currency, 0);
        assert!(c.owned.contains("party_hat"));
        c.currency = 100;
        assert_eq!(c.buy("party_hat"), Err("already owned"));
    }

    #[test]
    fn equip_requires_ownership_and_empty_id_goes_bare() {
        let mut c = Care::default();
        assert_eq!(c.equip("bow"), Err("not owned"));
        c.owned.insert("bow".to_string());
        assert_eq!(c.equip("bow"), Ok(()));
        assert_eq!(c.equipped, Some("bow".to_string()));
        assert_eq!(c.equip(""), Ok(()));
        assert_eq!(c.equipped, None);
    }

    #[test]
    fn load_missing_file_is_default() {
        let c = Care::load(Path::new("/nonexistent/care.json"));
        assert_eq!((c.hunger, c.energy, c.affection, c.currency), (NEED_CEIL, NEED_CEIL, NEED_CEIL, 0));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sushi-care-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("care.json");
        let mut c = Care::default();
        c.feed(1_000).unwrap();
        c.currency = 42;
        c.owned.insert("bow".to_string());
        c.save(&path);
        let loaded = Care::load(&path);
        assert_eq!((loaded.xp, loaded.currency), (5, 42));
        assert!(loaded.owned.contains("bow"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
