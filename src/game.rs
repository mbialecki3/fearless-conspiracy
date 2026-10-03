use crate::roles::{Faction, Preset, Role};
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

pub const MAX_NAME_BYTES: usize = 20;
pub const MAX_CHAT_BYTES: usize = 512;
pub const MAX_WILL_BYTES: usize = 4 * 1024;
pub const MAX_DEATH_NOTE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u8);

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseKind {
    Lobby,
    RoleReveal,
    DayOne,
    Night,
    Dawn,
    Discussion,
    Nominations,
    Defense,
    Judgment,
    LastWords,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub id: u64,
    pub kind: PhaseKind,
    pub day: u8,
}

impl Phase {
    fn lobby() -> Self {
        Self {
            id: 0,
            kind: PhaseKind::Lobby,
            day: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GameConfig {
    pub preset: Preset,
    pub scenario_roles: Option<Vec<Role>>,
}

impl GameConfig {
    pub fn ordinary(preset: Preset) -> Self {
        Self {
            preset,
            scenario_roles: None,
        }
    }

    pub fn scenario(roles: Vec<Role>) -> Self {
        Self {
            preset: match roles.len() {
                5 => Preset::Demo5,
                7 => Preset::Standard7,
                8 => Preset::Standard8,
                _ => Preset::Full9,
            },
            scenario_roles: Some(roles),
        }
    }

    pub fn expected_players(&self) -> usize {
        self.scenario_roles
            .as_deref()
            .unwrap_or_else(|| self.preset.roles())
            .len()
    }
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub id: PlayerId,
    pub uid: u32,
    pub name: String,
    pub original_role: Option<Role>,
    pub current_role: Option<Role>,
    pub faction: Option<Faction>,
    pub alive: bool,
    pub connected: bool,
    pub pending_forfeit: bool,
    pub ready: bool,
    pub will: String,
    pub death_note: String,
    pub self_heals_remaining: u8,
    pub executions_remaining: u8,
    pub jester_won: bool,
    pub forfeited: bool,
}

impl PlayerState {
    pub fn role(&self) -> Result<Role, GameError> {
        self.current_role.ok_or(GameError::MatchNotStarted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipients {
    All,
    Players(BTreeSet<PlayerId>),
}

impl Recipients {
    pub fn only(player: PlayerId) -> Self {
        Self::Players(BTreeSet::from([player]))
    }

    pub fn contains(&self, player: PlayerId) -> bool {
        match self {
            Self::All => true,
            Self::Players(players) => players.contains(&player),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEvent {
    pub recipients: Recipients,
    pub channel: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameError {
    LobbyFull,
    MatchAlreadyStarted,
    MatchNotStarted,
    UnknownPlayer(PlayerId),
    DuplicateUid,
    DuplicateName,
    InvalidName(String),
    NotReady,
    InvalidScenario(String),
    WrongRole,
    InvalidText(String),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LobbyFull => write!(f, "the lobby is full"),
            Self::MatchAlreadyStarted => write!(f, "the match has already started"),
            Self::MatchNotStarted => write!(f, "the match has not started"),
            Self::UnknownPlayer(player) => write!(f, "unknown player {player}"),
            Self::DuplicateUid => write!(f, "that Linux UID already has a player"),
            Self::DuplicateName => write!(f, "that display name is already in use"),
            Self::InvalidName(reason) => write!(f, "invalid display name: {reason}"),
            Self::NotReady => write!(f, "every required player must be connected and ready"),
            Self::InvalidScenario(reason) => write!(f, "invalid scenario: {reason}"),
            Self::WrongRole => write!(f, "that command is not available to this role"),
            Self::InvalidText(reason) => write!(f, "invalid text: {reason}"),
        }
    }
}

impl std::error::Error for GameError {}

pub struct Game {
    pub config: GameConfig,
    pub phase: Phase,
    pub players: BTreeMap<PlayerId, PlayerState>,
    rng: StdRng,
}

impl Game {
    pub fn with_seed(config: GameConfig, seed: u64) -> Result<Self, GameError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            phase: Phase::lobby(),
            players: BTreeMap::new(),
            rng: StdRng::seed_from_u64(seed),
        })
    }

    pub fn from_os_random(config: GameConfig) -> Result<Self, GameError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            phase: Phase::lobby(),
            players: BTreeMap::new(),
            rng: StdRng::from_os_rng(),
        })
    }

    pub fn join(&mut self, uid: u32, name: &str) -> Result<PlayerId, GameError> {
        if self.phase.kind != PhaseKind::Lobby {
            return Err(GameError::MatchAlreadyStarted);
        }
        if self.players.len() >= self.config.expected_players() {
            return Err(GameError::LobbyFull);
        }
        validate_name(name)?;
        if self.players.values().any(|player| player.uid == uid) {
            return Err(GameError::DuplicateUid);
        }
        if self
            .players
            .values()
            .any(|player| player.name.eq_ignore_ascii_case(name))
        {
            return Err(GameError::DuplicateName);
        }
        let id = PlayerId(
            (1_u8..=9)
                .find(|candidate| !self.players.contains_key(&PlayerId(*candidate)))
                .expect("at most nine players are admitted"),
        );
        self.players.insert(
            id,
            PlayerState {
                id,
                uid,
                name: name.into(),
                original_role: None,
                current_role: None,
                faction: None,
                alive: true,
                connected: true,
                pending_forfeit: false,
                ready: false,
                will: String::new(),
                death_note: String::new(),
                self_heals_remaining: 1,
                executions_remaining: 3,
                jester_won: false,
                forfeited: false,
            },
        );
        Ok(id)
    }

    pub fn leave_lobby(&mut self, id: PlayerId) -> Result<u32, GameError> {
        if self.phase.kind != PhaseKind::Lobby {
            return Err(GameError::MatchAlreadyStarted);
        }
        self.players
            .remove(&id)
            .map(|player| player.uid)
            .ok_or(GameError::UnknownPlayer(id))
    }

    pub fn toggle_ready(&mut self, id: PlayerId) -> Result<bool, GameError> {
        if self.phase.kind != PhaseKind::Lobby {
            return Err(GameError::MatchAlreadyStarted);
        }
        let player = self
            .players
            .get_mut(&id)
            .ok_or(GameError::UnknownPlayer(id))?;
        player.ready = !player.ready;
        Ok(player.ready)
    }

    pub fn can_start(&self) -> bool {
        self.phase.kind == PhaseKind::Lobby
            && self.players.len() == self.config.expected_players()
            && self
                .players
                .values()
                .all(|player| player.connected && player.ready)
    }

    pub fn start(&mut self) -> Result<Vec<GameEvent>, GameError> {
        if self.phase.kind != PhaseKind::Lobby {
            return Err(GameError::MatchAlreadyStarted);
        }
        if !self.can_start() {
            return Err(GameError::NotReady);
        }
        let mut roles = self
            .config
            .scenario_roles
            .clone()
            .unwrap_or_else(|| self.config.preset.roles().to_vec());
        if self.config.scenario_roles.is_none() {
            roles.shuffle(&mut self.rng);
        }
        for (player, role) in self.players.values_mut().zip(roles) {
            player.original_role = Some(role);
            player.current_role = Some(role);
            player.faction = Some(role.faction());
        }
        self.phase = Phase {
            id: 1,
            kind: PhaseKind::RoleReveal,
            day: 1,
        };
        Ok(self.role_reveal_events())
    }

    pub fn player(&self, id: PlayerId) -> Result<&PlayerState, GameError> {
        self.players.get(&id).ok_or(GameError::UnknownPlayer(id))
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Result<&mut PlayerState, GameError> {
        self.players
            .get_mut(&id)
            .ok_or(GameError::UnknownPlayer(id))
    }

    pub fn set_will(&mut self, id: PlayerId, text: &str) -> Result<(), GameError> {
        validate_player_text(text, MAX_WILL_BYTES)?;
        let player = self.player_mut(id)?;
        if !player.alive {
            return Err(GameError::InvalidText(
                "dead players cannot edit frozen wills".into(),
            ));
        }
        player.will = text.into();
        Ok(())
    }

    pub fn set_death_note(&mut self, id: PlayerId, text: &str) -> Result<(), GameError> {
        validate_player_text(text, MAX_DEATH_NOTE_BYTES)?;
        let player = self.player_mut(id)?;
        if !player.role()?.is_mafia_killing() {
            return Err(GameError::WrongRole);
        }
        player.death_note = text.into();
        Ok(())
    }

    fn role_reveal_events(&self) -> Vec<GameEvent> {
        let mafia: Vec<_> = self
            .players
            .values()
            .filter(|player| player.faction == Some(Faction::Mafia))
            .collect();
        self.players
            .values()
            .map(|player| {
                let role = player.current_role.expect("roles assigned");
                let mut text = format!(
                    "You are the {} ({:?}).",
                    role.display_name(),
                    role.faction()
                );
                if role.faction() == Faction::Mafia {
                    let roster = mafia
                        .iter()
                        .map(|mate| {
                            format!(
                                "{} {} ({})",
                                mate.id,
                                mate.name,
                                mate.current_role.expect("roles assigned").display_name()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    text.push_str(&format!(" Mafia roster: {roster}."));
                }
                GameEvent {
                    recipients: Recipients::only(player.id),
                    channel: "private",
                    text,
                }
            })
            .collect()
    }
}

fn validate_config(config: &GameConfig) -> Result<(), GameError> {
    let roles = config
        .scenario_roles
        .as_deref()
        .unwrap_or_else(|| config.preset.roles());
    if !(5..=9).contains(&roles.len()) {
        return Err(GameError::InvalidScenario(
            "a match requires five through nine players".into(),
        ));
    }
    let unique: HashSet<_> = roles.iter().copied().collect();
    if unique.len() != roles.len() {
        return Err(GameError::InvalidScenario(
            "this fixed ruleset allows at most one of each role".into(),
        ));
    }
    if !roles.iter().any(|role| role.faction() == Faction::Town)
        || !roles.iter().any(|role| role.faction() == Faction::Mafia)
    {
        return Err(GameError::InvalidScenario(
            "a scenario requires both Town and Mafia".into(),
        ));
    }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<(), GameError> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(GameError::InvalidName(format!(
            "use 1-{MAX_NAME_BYTES} ASCII characters"
        )));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_' | b'-'))
    {
        return Err(GameError::InvalidName(
            "only ASCII letters, digits, spaces, underscores, and hyphens are allowed".into(),
        ));
    }
    if matches!(name.to_ascii_lowercase().as_str(), "server" | "jailor") {
        return Err(GameError::InvalidName("that name is reserved".into()));
    }
    Ok(())
}

pub fn validate_player_text(text: &str, maximum: usize) -> Result<(), GameError> {
    if text.len() > maximum {
        return Err(GameError::InvalidText(format!(
            "text is {} bytes; maximum is {maximum}",
            text.len()
        )));
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(GameError::InvalidText(
            "terminal control characters are not allowed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_demo_game() -> Game {
        let roles = Preset::Demo5.roles().to_vec();
        let mut game = Game::with_seed(GameConfig::scenario(roles), 7).unwrap();
        for (uid, name) in ["Alice", "Bob", "Cara", "Drew", "Emi"]
            .into_iter()
            .enumerate()
        {
            let id = game.join(uid as u32 + 1000, name).unwrap();
            game.toggle_ready(id).unwrap();
        }
        game
    }

    #[test]
    fn all_presets_have_unique_roles_and_expected_sizes() {
        for (preset, size) in [
            (Preset::Demo5, 5),
            (Preset::Standard7, 7),
            (Preset::Standard8, 8),
            (Preset::Full9, 9),
        ] {
            assert_eq!(preset.roles().len(), size);
            assert_eq!(
                preset.roles().iter().copied().collect::<HashSet<_>>().len(),
                size
            );
            Game::with_seed(GameConfig::ordinary(preset), 1).unwrap();
        }
    }

    #[test]
    fn names_are_unique_case_insensitively_and_reject_terminal_controls() {
        let mut game = Game::with_seed(GameConfig::ordinary(Preset::Demo5), 1).unwrap();
        game.join(1001, "Alice-Smith").unwrap();
        assert_eq!(
            game.join(1002, "ALICE-smith"),
            Err(GameError::DuplicateName)
        );
        assert!(matches!(
            game.join(1003, "bad\u{1b}[31m"),
            Err(GameError::InvalidName(_))
        ));
        assert!(matches!(
            game.join(1003, "Server"),
            Err(GameError::InvalidName(_))
        ));
    }

    #[test]
    fn same_uid_cannot_claim_two_lobby_players() {
        let mut game = Game::with_seed(GameConfig::ordinary(Preset::Demo5), 1).unwrap();
        game.join(1001, "Alice").unwrap();
        assert_eq!(game.join(1001, "Bob"), Err(GameError::DuplicateUid));
    }

    #[test]
    fn fixed_scenario_assigns_roles_by_stable_player_id() {
        let mut game = ready_demo_game();
        let events = game.start().unwrap();
        assert_eq!(game.phase.kind, PhaseKind::RoleReveal);
        assert_eq!(
            game.player(PlayerId(1)).unwrap().role().unwrap(),
            Role::Sheriff
        );
        assert_eq!(
            game.player(PlayerId(5)).unwrap().role().unwrap(),
            Role::Jester
        );
        assert_eq!(events.len(), 5);
        for event in events {
            assert!(matches!(event.recipients, Recipients::Players(ref ids) if ids.len() == 1));
        }
    }

    #[test]
    fn mafia_roster_is_never_sent_to_town_or_jester() {
        let mut game = ready_demo_game();
        let events = game.start().unwrap();
        for event in events {
            let recipient = match &event.recipients {
                Recipients::Players(ids) => *ids.iter().next().unwrap(),
                Recipients::All => panic!("role reveal must be private"),
            };
            let role = game.player(recipient).unwrap().role().unwrap();
            assert_eq!(
                event.text.contains("Mafia roster:"),
                role.faction() == Faction::Mafia
            );
        }
    }

    #[test]
    fn will_and_death_note_limits_and_role_permissions_are_enforced() {
        let mut game = ready_demo_game();
        game.start().unwrap();
        game.set_will(PlayerId(1), "N1 check #2").unwrap();
        assert_eq!(game.player(PlayerId(1)).unwrap().will, "N1 check #2");
        assert_eq!(
            game.set_death_note(PlayerId(1), "boo"),
            Err(GameError::WrongRole)
        );
        game.set_death_note(PlayerId(4), "boo").unwrap();
        assert!(matches!(
            game.set_will(PlayerId(1), "\u{1b}[2J"),
            Err(GameError::InvalidText(_))
        ));
        assert!(matches!(
            game.set_will(PlayerId(1), &"x".repeat(MAX_WILL_BYTES + 1)),
            Err(GameError::InvalidText(_))
        ));
    }
}
