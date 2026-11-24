//! The rules of the exported world: what Minecraft's `/gamerule` sets
//! (whether mobs spawn, the weather and the day go on, how fast blocks tick,
//! fire spreads, mobs break blocks…), the weather and the time of day it
//! starts with. The export writes them into the world as each version
//! keeps them (see `massif_anvil::level_dat`), so that a world opens as set,
//! with no command to type after each export.

use serde::{Deserialize, Serialize};

/// The weather a world starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weather {
    #[default]
    Clear,
    Rain,
    Thunder,
}

impl Weather {
    pub const ALL: [Weather; 3] = [Weather::Clear, Weather::Rain, Weather::Thunder];

    pub const fn label(self) -> &'static str {
        match self {
            Weather::Clear => "Clear",
            Weather::Rain => "Rain",
            Weather::Thunder => "Thunder",
        }
    }
}

/// The game's rules, by what they do. Their defaults are the game's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameRules {
    /// `doMobSpawning`: animals and monsters appear by themselves.
    pub spawn_mobs: bool,
    /// `doPatrolSpawning`: pillager patrols.
    pub spawn_patrols: bool,
    /// `doTraderSpawning`: the wandering trader.
    pub spawn_traders: bool,
    /// `doInsomnia`: phantoms, after nights without sleep.
    pub spawn_phantoms: bool,
    /// `doDaylightCycle`: the sun moves.
    pub advance_time: bool,
    /// `doWeatherCycle`: the weather changes.
    pub advance_weather: bool,
    /// `randomTickSpeed`: blocks ticked at random per chunk section and
    /// game tick (3 in the game): crops and grass grow, leaves decay, ice
    /// melts. 0 stops all of it.
    pub random_tick_speed: i32,
    /// `doFireTick`: fire spreads and burns out.
    pub fire_spreads: bool,
    /// `mobGriefing`: creepers, endermen, ghasts… change blocks.
    pub mob_griefing: bool,
    /// `keepInventory`: players keep their things when they die.
    pub keep_inventory: bool,
    /// The weather at the start (and for good, when it does not change).
    pub weather: Weather,
    /// The time of day at the start, in ticks (0: sunrise, 6000: noon,
    /// 12000: sunset, 18000: midnight).
    pub time: i64,
}

/// Ticks of the times of day offered.
pub const MORNING: i64 = 1000;
pub const NOON: i64 = 6000;
pub const SUNSET: i64 = 12000;
pub const MIDNIGHT: i64 = 18000;

impl Default for GameRules {
    fn default() -> Self {
        Self::vanilla()
    }
}

/// The rules ready to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Nothing spawns, spreads, grows or decays; no weather nor day going
    /// by: noon for good. To walk round a map as it was made.
    Frozen,
    /// The game's, but fire does not spread and mobs break no blocks.
    SurvivalSafe,
    /// The game's own.
    Vanilla,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Frozen, Preset::SurvivalSafe, Preset::Vanilla];

    pub const fn label(self) -> &'static str {
        match self {
            Preset::Frozen => "Creative / Frozen",
            Preset::SurvivalSafe => "Survival safe",
            Preset::Vanilla => "Vanilla",
        }
    }

    pub fn rules(self) -> GameRules {
        match self {
            Preset::Vanilla => GameRules::vanilla(),
            Preset::SurvivalSafe => GameRules { fire_spreads: false, mob_griefing: false, ..GameRules::vanilla() },
            Preset::Frozen => GameRules {
                spawn_mobs: false,
                spawn_patrols: false,
                spawn_traders: false,
                spawn_phantoms: false,
                advance_time: false,
                advance_weather: false,
                random_tick_speed: 0,
                fire_spreads: false,
                mob_griefing: false,
                keep_inventory: false,
                weather: Weather::Clear,
                time: NOON,
            },
        }
    }

    /// The preset `rules` are, if they are one.
    pub fn of(rules: &GameRules) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.rules() == *rules)
    }
}

impl GameRules {
    /// The game's defaults; the day starts in the morning, as Massif's
    /// exports always did.
    pub const fn vanilla() -> Self {
        Self {
            spawn_mobs: true,
            spawn_patrols: true,
            spawn_traders: true,
            spawn_phantoms: true,
            advance_time: true,
            advance_weather: true,
            random_tick_speed: 3,
            fire_spreads: true,
            mob_griefing: true,
            keep_inventory: false,
            weather: Weather::Clear,
            time: MORNING,
        }
    }

    /// The highest random tick speed offered: past it the game does little
    /// but tick blocks.
    pub const MAX_TICK_SPEED: i32 = 1000;
}

/// A set of rules kept under a name of one's own (Settings ▸ Export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedRules {
    pub name: String,
    pub rules: GameRules,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_are_told_apart() {
        for p in Preset::ALL {
            assert_eq!(Preset::of(&p.rules()), Some(p));
        }
        assert_eq!(Preset::of(&GameRules::default()), Some(Preset::Vanilla));
        assert_eq!(Preset::of(&GameRules { random_tick_speed: 1, ..GameRules::vanilla() }), None);
        // Missing fields read as the game's.
        let read: GameRules = serde_json::from_str(r#"{"spawn_mobs": false}"#).unwrap();
        assert_eq!(read, GameRules { spawn_mobs: false, ..GameRules::vanilla() });
    }
}
