use crate::roles::{Faction, HauntState, NightIntent, NightOutcome, Preset, Role};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JudgmentChoice {
    Guilty,
    Innocent,
    Abstain,
}

impl JudgmentChoice {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Guilty => "guilty",
            Self::Innocent => "innocent",
            Self::Abstain => "abstain",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Convicted,
    Acquitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchOutcome {
    InProgress,
    TownWin,
    MafiaWin,
    NoFaction,
    Draw,
    Aborted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerOutcome {
    Win,
    Loss,
    Draw,
    Forfeit,
    Aborted,
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
    WrongPhase {
        expected: &'static str,
        actual: PhaseKind,
    },
    StalePhase {
        supplied: u64,
        current: u64,
    },
    DeadPlayer,
    InvalidTarget(String),
    CannotVote(String),
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
            Self::WrongPhase { expected, actual } => {
                write!(
                    f,
                    "command requires {expected}; current phase is {actual:?}"
                )
            }
            Self::StalePhase { supplied, current } => {
                write!(
                    f,
                    "command is for phase {supplied}; current phase is {current}"
                )
            }
            Self::DeadPlayer => write!(f, "dead players cannot perform that action"),
            Self::InvalidTarget(reason) => write!(f, "invalid target: {reason}"),
            Self::CannotVote(reason) => write!(f, "cannot vote: {reason}"),
        }
    }
}

impl std::error::Error for GameError {}

pub struct Game {
    pub config: GameConfig,
    pub phase: Phase,
    pub players: BTreeMap<PlayerId, PlayerState>,
    pub nominations: BTreeMap<PlayerId, PlayerId>,
    pub judgments: BTreeMap<PlayerId, JudgmentChoice>,
    pub accused: Option<PlayerId>,
    pub trials_today: u8,
    pub last_trial_votes: BTreeMap<PlayerId, JudgmentChoice>,
    pub outcome: MatchOutcome,
    pub intents: BTreeMap<PlayerId, NightIntent>,
    pub pending_haunt: Option<HauntState>,
    pub completed_nights: u8,
    pub deathless_cycles: u8,
    pub frames: BTreeSet<PlayerId>,
    pub jail_selection: Option<PlayerId>,
    pub jailed: Option<(PlayerId, PlayerId)>,
    pub(crate) rng: StdRng,
}

impl Game {
    pub fn with_seed(config: GameConfig, seed: u64) -> Result<Self, GameError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            phase: Phase::lobby(),
            players: BTreeMap::new(),
            nominations: BTreeMap::new(),
            judgments: BTreeMap::new(),
            accused: None,
            trials_today: 0,
            last_trial_votes: BTreeMap::new(),
            outcome: MatchOutcome::InProgress,
            intents: BTreeMap::new(),
            pending_haunt: None,
            completed_nights: 0,
            deathless_cycles: 0,
            frames: BTreeSet::new(),
            jail_selection: None,
            jailed: None,
            rng: StdRng::seed_from_u64(seed),
        })
    }

    pub fn from_os_random(config: GameConfig) -> Result<Self, GameError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            phase: Phase::lobby(),
            players: BTreeMap::new(),
            nominations: BTreeMap::new(),
            judgments: BTreeMap::new(),
            accused: None,
            trials_today: 0,
            last_trial_votes: BTreeMap::new(),
            outcome: MatchOutcome::InProgress,
            intents: BTreeMap::new(),
            pending_haunt: None,
            completed_nights: 0,
            deathless_cycles: 0,
            frames: BTreeSet::new(),
            jail_selection: None,
            jailed: None,
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

    pub fn advance_phase(&mut self, phase_id: u64) -> Result<Vec<GameEvent>, GameError> {
        self.ensure_phase_id(phase_id)?;
        match self.phase.kind {
            PhaseKind::RoleReveal => {
                self.move_to(PhaseKind::DayOne, self.phase.day);
                Ok(vec![self.phase_event(
                    "Day 1 introductions have begun. No trial is held today.",
                )])
            }
            PhaseKind::DayOne => {
                let mut events = self.begin_night();
                events.insert(0, self.phase_event("Night 1 has begun."));
                Ok(events)
            }
            PhaseKind::Dawn => {
                self.move_to(PhaseKind::Discussion, self.phase.day);
                Ok(vec![self.phase_event("Discussion has begun.")])
            }
            PhaseKind::Discussion => {
                self.nominations.clear();
                self.trials_today = 0;
                self.move_to(PhaseKind::Nominations, self.phase.day);
                Ok(vec![self.phase_event("Nominations are open.")])
            }
            PhaseKind::Nominations => {
                let mut events = self.begin_night();
                events.insert(
                    0,
                    self.phase_event("Nomination time expired. Night has begun."),
                );
                Ok(events)
            }
            PhaseKind::Defense => {
                self.judgments.clear();
                self.move_to(PhaseKind::Judgment, self.phase.day);
                Ok(vec![self.phase_event(
                    "Judgment voting is open; the accused cannot vote.",
                )])
            }
            PhaseKind::LastWords => self.execute_accused(),
            PhaseKind::Lobby => Err(GameError::MatchNotStarted),
            PhaseKind::Night => Err(GameError::WrongPhase {
                expected: "night resolution",
                actual: self.phase.kind,
            }),
            PhaseKind::Judgment => Err(GameError::WrongPhase {
                expected: "verdict resolution",
                actual: self.phase.kind,
            }),
            PhaseKind::Ended => Err(GameError::MatchAlreadyStarted),
        }
    }

    pub fn finish_night_for_test_or_resolver(&mut self) -> Result<GameEvent, GameError> {
        if self.phase.kind != PhaseKind::Night {
            return Err(GameError::WrongPhase {
                expected: "night",
                actual: self.phase.kind,
            });
        }
        self.move_to(PhaseKind::Dawn, self.phase.day.saturating_add(1));
        Ok(self.phase_event(&format!("Dawn {} has begun.", self.phase.day)))
    }

    pub fn nominate(
        &mut self,
        actor: PlayerId,
        target: PlayerId,
        phase_id: u64,
    ) -> Result<Vec<GameEvent>, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Nominations, "nominations")?;
        self.ensure_living(actor)?;
        self.ensure_living(target)?;
        if actor == target {
            return Err(GameError::CannotVote(
                "self-nominations are not allowed".into(),
            ));
        }
        let previous = self.nominations.insert(actor, target);
        let total = self.nomination_count(target);
        let threshold = self.nomination_threshold();
        let mut events = vec![GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: match previous {
                Some(old) if old != target => format!(
                    "{actor} changed their nomination from {old} to {target} ({total}/{threshold})."
                ),
                _ => format!("{actor} nominated {target} ({total}/{threshold})."),
            },
        }];
        if total >= threshold {
            self.accused = Some(target);
            self.move_to(PhaseKind::Defense, self.phase.day);
            events.push(self.phase_event(&format!(
                "{target} is on trial and may now speak in defense."
            )));
        }
        Ok(events)
    }

    pub fn withdraw_nomination(
        &mut self,
        actor: PlayerId,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Nominations, "nominations")?;
        self.ensure_living(actor)?;
        let old = self
            .nominations
            .remove(&actor)
            .ok_or_else(|| GameError::CannotVote("no active nomination to withdraw".into()))?;
        Ok(GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!("{actor} withdrew their nomination of {old}."),
        })
    }

    pub fn judge(
        &mut self,
        actor: PlayerId,
        choice: JudgmentChoice,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Judgment, "judgment")?;
        self.ensure_living(actor)?;
        if self.accused == Some(actor) {
            return Err(GameError::CannotVote("the accused cannot vote".into()));
        }
        self.judgments.insert(actor, choice);
        Ok(GameEvent {
            recipients: Recipients::only(actor),
            channel: "private_system",
            text: format!("Judgment choice set to {}.", choice.label()),
        })
    }

    pub fn resolve_judgment(
        &mut self,
        phase_id: u64,
    ) -> Result<(Verdict, Vec<GameEvent>), GameError> {
        self.ensure_phase(phase_id, PhaseKind::Judgment, "judgment")?;
        let accused = self
            .accused
            .ok_or_else(|| GameError::CannotVote("judgment has no accused player".into()))?;
        let mut guilty = 0;
        let mut innocent = 0;
        let mut published = Vec::new();
        self.last_trial_votes.clear();
        for player in self
            .players
            .values()
            .filter(|player| player.alive && player.id != accused)
        {
            let choice = self
                .judgments
                .get(&player.id)
                .copied()
                .unwrap_or(JudgmentChoice::Abstain);
            self.last_trial_votes.insert(player.id, choice);
            match choice {
                JudgmentChoice::Guilty => guilty += 1,
                JudgmentChoice::Innocent => innocent += 1,
                JudgmentChoice::Abstain => {}
            }
            published.push(format!("{}: {}", player.id, choice.label()));
        }
        let verdict = if guilty > innocent {
            Verdict::Convicted
        } else {
            Verdict::Acquitted
        };
        let mut events = vec![GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!(
                "Judgment votes: {}. Totals: {guilty} guilty, {innocent} innocent. Verdict: {:?}.",
                published.join(", "),
                verdict
            ),
        }];
        self.trials_today += 1;
        match verdict {
            Verdict::Convicted => {
                self.move_to(PhaseKind::LastWords, self.phase.day);
                events.push(self.phase_event(&format!("{accused} may speak their last words.")));
            }
            Verdict::Acquitted => {
                self.accused = None;
                self.nominations.clear();
                self.judgments.clear();
                if self.trials_today >= 3 {
                    events.extend(self.begin_night());
                    events.push(self.phase_event("The trial limit was reached. Night has begun."));
                } else {
                    self.move_to(PhaseKind::Nominations, self.phase.day);
                    events.push(self.phase_event("The accused was acquitted. Nominations resume."));
                }
            }
        }
        Ok((verdict, events))
    }

    pub fn say(&self, actor: PlayerId, text: &str, phase_id: u64) -> Result<GameEvent, GameError> {
        self.ensure_phase_id(phase_id)?;
        validate_player_text(text, MAX_CHAT_BYTES)?;
        self.ensure_living(actor)?;
        let allowed = match self.phase.kind {
            PhaseKind::DayOne | PhaseKind::Discussion | PhaseKind::Nominations => true,
            PhaseKind::Defense => self.accused == Some(actor),
            PhaseKind::LastWords => self.accused == Some(actor),
            _ => false,
        };
        if !allowed {
            return Err(GameError::WrongPhase {
                expected: "an eligible public speaking phase",
                actual: self.phase.kind,
            });
        }
        Ok(GameEvent {
            recipients: Recipients::All,
            channel: "public",
            text: format!("{actor} {}: {text}", self.player(actor)?.name),
        })
    }

    pub fn whisper(
        &self,
        actor: PlayerId,
        target: PlayerId,
        text: &str,
        phase_id: u64,
    ) -> Result<Vec<GameEvent>, GameError> {
        self.ensure_phase_id(phase_id)?;
        validate_player_text(text, MAX_CHAT_BYTES)?;
        self.ensure_living(actor)?;
        self.ensure_living(target)?;
        if actor == target {
            return Err(GameError::InvalidTarget(
                "cannot whisper to yourself".into(),
            ));
        }
        if !matches!(
            self.phase.kind,
            PhaseKind::DayOne | PhaseKind::Discussion | PhaseKind::Nominations
        ) {
            return Err(GameError::WrongPhase {
                expected: "introductions, discussion, or nominations",
                actual: self.phase.kind,
            });
        }
        Ok(vec![
            GameEvent {
                recipients: Recipients::Players(BTreeSet::from([actor, target])),
                channel: "whisper",
                text: format!("{actor} -> {target}: {text}"),
            },
            GameEvent {
                recipients: Recipients::All,
                channel: "public_system",
                text: format!("{actor} whispered to {target}."),
            },
        ])
    }

    pub fn mafia_chat(
        &self,
        actor: PlayerId,
        text: &str,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Night, "night")?;
        validate_player_text(text, MAX_CHAT_BYTES)?;
        self.ensure_living(actor)?;
        if self.player(actor)?.faction != Some(Faction::Mafia) {
            return Err(GameError::WrongRole);
        }
        if self.jailed.is_some_and(|(_, prisoner)| prisoner == actor) {
            return Err(GameError::WrongPhase {
                expected: "Mafia night chat while not jailed",
                actual: self.phase.kind,
            });
        }
        let recipients = self
            .players
            .values()
            .filter(|player| player.alive && player.faction == Some(Faction::Mafia))
            .map(|player| player.id)
            .collect();
        Ok(GameEvent {
            recipients: Recipients::Players(recipients),
            channel: "mafia",
            text: format!("{actor}: {text}"),
        })
    }

    pub fn select_jail(
        &mut self,
        actor: PlayerId,
        target: PlayerId,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase_id(phase_id)?;
        self.ensure_living(actor)?;
        self.ensure_living(target)?;
        if self.player(actor)?.role()? != Role::Jailor {
            return Err(GameError::WrongRole);
        }
        if actor == target {
            return Err(GameError::InvalidTarget(
                "the Jailor cannot jail themself".into(),
            ));
        }
        if !matches!(
            self.phase.kind,
            PhaseKind::DayOne
                | PhaseKind::Discussion
                | PhaseKind::Nominations
                | PhaseKind::Defense
                | PhaseKind::Judgment
                | PhaseKind::LastWords
        ) {
            return Err(GameError::WrongPhase {
                expected: "an eligible daytime phase",
                actual: self.phase.kind,
            });
        }
        self.jail_selection = Some(target);
        Ok(GameEvent {
            recipients: Recipients::only(actor),
            channel: "private_system",
            text: format!("You will jail {target} tonight if they remain alive."),
        })
    }

    pub fn jail_chat(
        &self,
        actor: PlayerId,
        text: &str,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Night, "night")?;
        validate_player_text(text, MAX_CHAT_BYTES)?;
        let (jailor, prisoner) = self.jailed.ok_or(GameError::WrongPhase {
            expected: "an active jail conversation",
            actual: self.phase.kind,
        })?;
        if actor != jailor && actor != prisoner {
            return Err(GameError::WrongRole);
        }
        Ok(GameEvent {
            recipients: Recipients::Players(BTreeSet::from([jailor, prisoner])),
            channel: "jail",
            text: if actor == jailor {
                format!("Jailor: {text}")
            } else {
                format!("{prisoner}: {text}")
            },
        })
    }

    pub fn dead_chat(
        &self,
        actor: PlayerId,
        text: &str,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase_id(phase_id)?;
        validate_player_text(text, MAX_CHAT_BYTES)?;
        if self.player(actor)?.alive {
            return Err(GameError::WrongPhase {
                expected: "dead chat after death",
                actual: self.phase.kind,
            });
        }
        let recipients = self
            .players
            .values()
            .filter(|player| !player.alive)
            .map(|player| player.id)
            .collect();
        Ok(GameEvent {
            recipients: Recipients::Players(recipients),
            channel: "dead",
            text: format!("{actor}: {text}"),
        })
    }

    pub fn select_night_intent(
        &mut self,
        actor: PlayerId,
        intent: NightIntent,
        phase_id: u64,
    ) -> Result<Vec<GameEvent>, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Night, "night")?;
        let role = self.player(actor)?.role()?;
        let is_haunt = matches!(intent, NightIntent::Haunt(_));
        if !is_haunt {
            self.ensure_living(actor)?;
            if self.jailed.is_some_and(|(_, prisoner)| prisoner == actor) {
                return Err(GameError::WrongPhase {
                    expected: "night action while not jailed",
                    actual: self.phase.kind,
                });
            }
        }
        let matches_role = matches!(
            (role, intent),
            (Role::Sheriff, NightIntent::Investigate(_))
                | (Role::Doctor, NightIntent::Heal(_))
                | (Role::Lookout, NightIntent::Watch(_))
                | (Role::TavernKeeper, NightIntent::Block(_))
                | (Role::Godfather | Role::Mafioso, NightIntent::Kill(_))
                | (Role::Framer, NightIntent::Frame(_))
                | (Role::Jailor, NightIntent::Execute | NightIntent::Spare)
                | (Role::Jester, NightIntent::Haunt(_))
        );
        if !matches_role {
            return Err(GameError::WrongRole);
        }
        if matches!(intent, NightIntent::Execute | NightIntent::Spare) {
            let (jailor, _) = self
                .jailed
                .ok_or_else(|| GameError::InvalidTarget("there is no active prisoner".into()))?;
            if jailor != actor {
                return Err(GameError::WrongRole);
            }
            if intent == NightIntent::Execute {
                if self.phase.day == 1 {
                    return Err(GameError::InvalidTarget(
                        "execution is not allowed on Night 1".into(),
                    ));
                }
                if self.player(actor)?.executions_remaining == 0 {
                    return Err(GameError::InvalidTarget("no executions remain".into()));
                }
            }
        }
        if let Some(target) = intent.target() {
            self.ensure_living(target)?;
            if actor == target {
                if intent != NightIntent::Heal(actor) {
                    return Err(GameError::InvalidTarget(
                        "this ability cannot target its actor".into(),
                    ));
                }
                if self.player(actor)?.self_heals_remaining == 0 {
                    return Err(GameError::InvalidTarget(
                        "the self-heal is exhausted".into(),
                    ));
                }
            }
            if matches!(role, Role::Godfather | Role::Mafioso | Role::Framer)
                && self.player(target)?.faction == Some(Faction::Mafia)
            {
                return Err(GameError::InvalidTarget(
                    "Mafia abilities cannot target a Mafia teammate".into(),
                ));
            }
            if is_haunt {
                let haunt = self.pending_haunt.as_ref().ok_or(GameError::WrongRole)?;
                if haunt.jester != actor || !haunt.eligible.contains(&target) {
                    return Err(GameError::InvalidTarget(
                        "haunt must target an eligible guilty or abstaining voter".into(),
                    ));
                }
            }
        }
        self.intents.insert(actor, intent);

        let mut events = vec![GameEvent {
            recipients: Recipients::only(actor),
            channel: "private_system",
            text: match intent.target() {
                Some(target) => {
                    format!("Night choice set to {target}. You may revise it before cutoff.")
                }
                None => format!("Night choice set to {intent:?}."),
            },
        }];
        if role.faction() == Faction::Mafia {
            let recipients = self
                .players
                .values()
                .filter(|player| player.alive && player.faction == Some(Faction::Mafia))
                .map(|player| player.id)
                .collect();
            events.push(GameEvent {
                recipients: Recipients::Players(recipients),
                channel: "mafia_system",
                text: format!("{actor} updated their {:?} selection.", role),
            });
        }
        if matches!(intent, NightIntent::Execute | NightIntent::Spare)
            && let Some((_, prisoner)) = self.jailed
        {
            events.push(GameEvent {
                recipients: Recipients::only(prisoner),
                channel: "jail",
                text: if intent == NightIntent::Execute {
                    "The Jailor selected execution.".into()
                } else {
                    "The Jailor selected spare.".into()
                },
            });
        }
        Ok(events)
    }

    pub fn cancel_night_intent(
        &mut self,
        actor: PlayerId,
        phase_id: u64,
    ) -> Result<GameEvent, GameError> {
        self.ensure_phase(phase_id, PhaseKind::Night, "night")?;
        let jailor_spared = self.player(actor)?.current_role == Some(Role::Jailor)
            && self.jailed.is_some_and(|(jailor, _)| jailor == actor);
        if jailor_spared {
            self.intents.insert(actor, NightIntent::Spare);
        } else {
            self.intents.remove(&actor);
        }
        Ok(GameEvent {
            recipients: Recipients::only(actor),
            channel: "private_system",
            text: if jailor_spared {
                "Execution canceled; the prisoner will be spared.".into()
            } else {
                "Night choice cleared.".into()
            },
        })
    }

    pub fn resolve_night(&mut self) -> Result<NightOutcome, GameError> {
        if self.phase.kind != PhaseKind::Night {
            return Err(GameError::WrongPhase {
                expected: "night",
                actual: self.phase.kind,
            });
        }
        crate::roles::resolve_night(self)
    }

    pub fn mark_disconnected(&mut self, id: PlayerId) -> Result<(), GameError> {
        let in_game = self.phase.kind != PhaseKind::Lobby;
        let player = self.player_mut(id)?;
        player.connected = false;
        if in_game && player.alive {
            player.pending_forfeit = true;
            self.intents.remove(&id);
        }
        Ok(())
    }

    pub fn apply_pending_forfeits(&mut self) -> Vec<GameEvent> {
        let forfeits: Vec<_> = self
            .players
            .values()
            .filter(|player| player.alive && player.pending_forfeit)
            .map(|player| player.id)
            .collect();
        let mut events = Vec::new();
        for id in forfeits {
            if let Ok(player) = self.player_mut(id) {
                player.alive = false;
                player.pending_forfeit = false;
                player.forfeited = true;
                let role = player.current_role;
                let name = player.name.clone();
                let will = player.will.clone();
                events.push(GameEvent {
                    recipients: Recipients::All,
                    channel: "public_system",
                    text: format!(
                        "{id} {name} forfeited. Role: {}. Will: {}",
                        role.map(Role::display_name).unwrap_or("unassigned"),
                        if will.is_empty() { "(empty)" } else { &will }
                    ),
                });
            }
            self.nominations.remove(&id);
            self.nominations.retain(|_, target| *target != id);
            self.judgments.remove(&id);
            self.intents.remove(&id);
            if self.accused == Some(id) {
                self.accused = None;
                self.nominations.clear();
                if matches!(
                    self.phase.kind,
                    PhaseKind::Defense | PhaseKind::Judgment | PhaseKind::LastWords
                ) {
                    self.move_to(PhaseKind::Nominations, self.phase.day);
                    events.push(self.phase_event(
                        "The accused forfeited; the trial was canceled and nominations resume.",
                    ));
                }
            }
        }
        if !events.is_empty() {
            self.evaluate_victory();
        }
        events
    }

    pub fn evaluate_victory(&mut self) -> MatchOutcome {
        if self.outcome != MatchOutcome::InProgress {
            return self.outcome;
        }
        if self.pending_haunt.as_ref().is_some_and(|haunt| {
            haunt
                .eligible
                .iter()
                .any(|target| self.players.get(target).is_some_and(|player| player.alive))
        }) {
            self.outcome = MatchOutcome::InProgress;
            return self.outcome;
        }
        let town = self
            .players
            .values()
            .filter(|player| player.alive && player.faction == Some(Faction::Town))
            .count();
        let mafia = self
            .players
            .values()
            .filter(|player| player.alive && player.faction == Some(Faction::Mafia))
            .count();
        self.outcome = match (town, mafia) {
            (0, 0) => MatchOutcome::NoFaction,
            (0, _) => MatchOutcome::MafiaWin,
            (_, 0) => MatchOutcome::TownWin,
            _ => MatchOutcome::InProgress,
        };
        if self.outcome != MatchOutcome::InProgress {
            self.move_to(PhaseKind::Ended, self.phase.day);
        }
        self.outcome
    }

    pub fn player_outcome(&self, id: PlayerId) -> Result<PlayerOutcome, GameError> {
        let player = self.player(id)?;
        if player.forfeited {
            return Ok(PlayerOutcome::Forfeit);
        }
        if self.outcome == MatchOutcome::Aborted && !player.jester_won {
            return Ok(PlayerOutcome::Aborted);
        }
        if player.original_role == Some(Role::Jester) {
            return Ok(if player.jester_won {
                PlayerOutcome::Win
            } else {
                PlayerOutcome::Loss
            });
        }
        Ok(match self.outcome {
            MatchOutcome::TownWin if player.faction == Some(Faction::Town) => PlayerOutcome::Win,
            MatchOutcome::MafiaWin if player.faction == Some(Faction::Mafia) => PlayerOutcome::Win,
            MatchOutcome::TownWin | MatchOutcome::MafiaWin | MatchOutcome::NoFaction => {
                PlayerOutcome::Loss
            }
            MatchOutcome::Draw => PlayerOutcome::Draw,
            MatchOutcome::Aborted => PlayerOutcome::Aborted,
            MatchOutcome::InProgress => PlayerOutcome::Loss,
        })
    }

    /// End immediately, preserving already earned personal wins and forfeits.
    pub fn abort(&mut self) -> Vec<GameEvent> {
        if self.outcome != MatchOutcome::InProgress {
            return Vec::new();
        }
        self.outcome = MatchOutcome::Aborted;
        self.intents.clear();
        self.pending_haunt = None;
        self.jailed = None;
        self.jail_selection = None;
        self.accused = None;
        self.nominations.clear();
        self.judgments.clear();
        for player in self.players.values_mut() {
            player.pending_forfeit = false;
        }
        self.move_to(PhaseKind::Ended, self.phase.day);
        vec![self.phase_event("The host aborted the match. No new wins are awarded.")]
    }

    pub fn final_event(&self) -> GameEvent {
        let roster = self
            .players
            .values()
            .map(|player| {
                format!(
                    "{} {}: original={}, current={}, outcome={:?}",
                    player.id,
                    player.name,
                    player
                        .original_role
                        .map(Role::display_name)
                        .unwrap_or("unassigned"),
                    player
                        .current_role
                        .map(Role::display_name)
                        .unwrap_or("unassigned"),
                    self.player_outcome(player.id)
                        .expect("roster player must have an outcome")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!("Final faction result: {:?}. {roster}", self.outcome),
        }
    }

    pub fn nomination_threshold(&self) -> usize {
        self.players.values().filter(|player| player.alive).count() / 2 + 1
    }

    fn execute_accused(&mut self) -> Result<Vec<GameEvent>, GameError> {
        let accused = self
            .accused
            .take()
            .ok_or_else(|| GameError::CannotVote("there is no convicted player".into()))?;
        let player = self.player_mut(accused)?;
        player.alive = false;
        if player.current_role == Some(Role::Jester) {
            player.jester_won = true;
        }
        let role = player.role()?;
        let name = player.name.clone();
        let will = player.will.clone();
        if role == Role::Jester {
            let eligible = self
                .last_trial_votes
                .iter()
                .filter_map(|(voter, choice)| {
                    matches!(choice, JudgmentChoice::Guilty | JudgmentChoice::Abstain)
                        .then_some(*voter)
                })
                .collect();
            self.pending_haunt = Some(HauntState {
                jester: accused,
                eligible,
            });
        }
        let mut events = vec![GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!(
                "{accused} {name} was publicly executed. Role: {}. Will: {}",
                role.display_name(),
                if will.is_empty() { "(empty)" } else { &will }
            ),
        }];
        if self.evaluate_victory() == MatchOutcome::InProgress {
            events.extend(self.begin_night());
            events.push(self.phase_event("The public execution is complete. Night has begun."));
        }
        Ok(events)
    }

    pub(crate) fn begin_night(&mut self) -> Vec<GameEvent> {
        self.nominations.clear();
        self.judgments.clear();
        self.accused = None;
        self.move_to(PhaseKind::Night, self.phase.day);
        self.jailed = None;
        let jailor = self
            .players
            .values()
            .find(|player| player.alive && player.current_role == Some(Role::Jailor))
            .map(|player| player.id);
        let prisoner = self
            .jail_selection
            .filter(|target| self.players.get(target).is_some_and(|player| player.alive));
        let (Some(jailor), Some(prisoner)) = (jailor, prisoner) else {
            return Vec::new();
        };
        self.jailed = Some((jailor, prisoner));
        let mut events = vec![
            GameEvent {
                recipients: Recipients::only(jailor),
                channel: "private_system",
                text: format!("You jailed {prisoner}. Use /execute or /spare."),
            },
            GameEvent {
                recipients: Recipients::only(prisoner),
                channel: "private_system",
                text:
                    "You were jailed. Your ordinary ability is suppressed; use /jailchat to speak."
                        .into(),
            },
        ];
        if self
            .player(prisoner)
            .is_ok_and(|player| player.faction == Some(Faction::Mafia))
        {
            let mafia = self
                .players
                .values()
                .filter(|player| player.alive && player.faction == Some(Faction::Mafia))
                .map(|player| player.id)
                .collect();
            events.push(GameEvent {
                recipients: Recipients::Players(mafia),
                channel: "mafia_system",
                text: format!("Mafia teammate {prisoner} was jailed and may listen but not reply."),
            });
        }
        events
    }

    fn nomination_count(&self, target: PlayerId) -> usize {
        self.nominations
            .values()
            .filter(|vote| **vote == target)
            .count()
    }

    fn ensure_living(&self, id: PlayerId) -> Result<(), GameError> {
        if self.player(id)?.alive {
            Ok(())
        } else {
            Err(GameError::DeadPlayer)
        }
    }

    fn ensure_phase_id(&self, phase_id: u64) -> Result<(), GameError> {
        if phase_id == self.phase.id {
            Ok(())
        } else {
            Err(GameError::StalePhase {
                supplied: phase_id,
                current: self.phase.id,
            })
        }
    }

    fn ensure_phase(
        &self,
        phase_id: u64,
        kind: PhaseKind,
        label: &'static str,
    ) -> Result<(), GameError> {
        self.ensure_phase_id(phase_id)?;
        if self.phase.kind == kind {
            Ok(())
        } else {
            Err(GameError::WrongPhase {
                expected: label,
                actual: self.phase.kind,
            })
        }
    }

    pub(crate) fn move_to(&mut self, kind: PhaseKind, day: u8) {
        self.phase.id += 1;
        self.phase.kind = kind;
        self.phase.day = day;
    }

    pub(crate) fn phase_event(&self, text: &str) -> GameEvent {
        GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!("[phase {} · {:?}] {text}", self.phase.id, self.phase.kind),
        }
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

    #[test]
    fn abort_is_terminal_and_awards_no_new_wins() {
        let mut game = ready_demo_game();
        game.start().unwrap();
        game.abort();
        assert_eq!(game.phase.kind, PhaseKind::Ended);
        assert_eq!(game.evaluate_victory(), MatchOutcome::Aborted);
        for id in 1..=5 {
            assert_eq!(
                game.player_outcome(PlayerId(id)).unwrap(),
                PlayerOutcome::Aborted
            );
        }
        let phase = game.phase;
        assert!(game.abort().is_empty());
        assert_eq!(game.phase, phase);

        let mut won = ready_demo_game();
        won.start().unwrap();
        won.player_mut(PlayerId(5)).unwrap().jester_won = true;
        won.player_mut(PlayerId(1)).unwrap().forfeited = true;
        won.abort();
        assert_eq!(won.player_outcome(PlayerId(5)).unwrap(), PlayerOutcome::Win);
        assert_eq!(
            won.player_outcome(PlayerId(1)).unwrap(),
            PlayerOutcome::Forfeit
        );
    }

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

    fn nomination_game() -> Game {
        let mut game = ready_demo_game();
        game.start().unwrap();
        let phase = game.phase.id;
        game.advance_phase(phase).unwrap();
        let phase = game.phase.id;
        game.advance_phase(phase).unwrap();
        game.finish_night_for_test_or_resolver().unwrap();
        let phase = game.phase.id;
        game.advance_phase(phase).unwrap();
        let phase = game.phase.id;
        game.advance_phase(phase).unwrap();
        assert_eq!(game.phase.kind, PhaseKind::Nominations);
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

    #[test]
    fn phase_ids_reject_late_commands_and_day_one_has_no_nominations() {
        let mut game = ready_demo_game();
        game.start().unwrap();
        let role_reveal = game.phase.id;
        game.advance_phase(role_reveal).unwrap();
        assert_eq!(game.phase.kind, PhaseKind::DayOne);
        assert!(matches!(
            game.nominate(PlayerId(1), PlayerId(2), game.phase.id),
            Err(GameError::WrongPhase { .. })
        ));
        assert!(matches!(
            game.say(PlayerId(1), "late", role_reveal),
            Err(GameError::StalePhase { .. })
        ));
    }

    #[test]
    fn nomination_majority_trial_and_private_judgment_reach_execution() {
        let mut game = nomination_game();
        game.set_will(PlayerId(4), "I claim Doctor").unwrap();
        let phase = game.phase.id;
        game.nominate(PlayerId(1), PlayerId(4), phase).unwrap();
        game.nominate(PlayerId(2), PlayerId(4), phase).unwrap();
        let events = game.nominate(PlayerId(3), PlayerId(4), phase).unwrap();
        assert_eq!(game.nomination_threshold(), 3);
        assert_eq!(game.phase.kind, PhaseKind::Defense);
        assert!(events.iter().any(|event| event.text.contains("on trial")));
        assert!(
            game.say(PlayerId(4), "I am innocent", game.phase.id)
                .is_ok()
        );
        assert!(game.say(PlayerId(1), "interrupt", game.phase.id).is_err());

        game.advance_phase(game.phase.id).unwrap();
        let phase = game.phase.id;
        let acknowledgment = game
            .judge(PlayerId(1), JudgmentChoice::Guilty, phase)
            .unwrap();
        assert_eq!(acknowledgment.recipients, Recipients::only(PlayerId(1)));
        game.judge(PlayerId(2), JudgmentChoice::Guilty, phase)
            .unwrap();
        game.judge(PlayerId(3), JudgmentChoice::Innocent, phase)
            .unwrap();
        assert!(
            game.judge(PlayerId(4), JudgmentChoice::Guilty, phase)
                .is_err()
        );
        let (verdict, events) = game.resolve_judgment(phase).unwrap();
        assert_eq!(verdict, Verdict::Convicted);
        assert!(events[0].text.contains("#5: abstain"));
        assert_eq!(game.phase.kind, PhaseKind::LastWords);

        let events = game.advance_phase(game.phase.id).unwrap();
        assert!(!game.player(PlayerId(4)).unwrap().alive);
        assert!(events[0].text.contains("I claim Doctor"));
        assert_eq!(game.outcome, MatchOutcome::TownWin);
        assert_eq!(game.phase.kind, PhaseKind::Ended);
    }

    #[test]
    fn tied_judgment_acquits_and_clears_nominations() {
        let mut game = nomination_game();
        let phase = game.phase.id;
        for voter in [PlayerId(1), PlayerId(2), PlayerId(3)] {
            game.nominate(voter, PlayerId(4), phase).unwrap();
        }
        game.advance_phase(game.phase.id).unwrap();
        let phase = game.phase.id;
        game.judge(PlayerId(1), JudgmentChoice::Guilty, phase)
            .unwrap();
        game.judge(PlayerId(2), JudgmentChoice::Innocent, phase)
            .unwrap();
        let (verdict, _) = game.resolve_judgment(phase).unwrap();
        assert_eq!(verdict, Verdict::Acquitted);
        assert_eq!(game.phase.kind, PhaseKind::Nominations);
        assert!(game.nominations.is_empty());
        assert_eq!(game.trials_today, 1);
    }

    #[test]
    fn whisper_payload_is_only_addressed_to_participants() {
        let mut game = ready_demo_game();
        game.start().unwrap();
        game.advance_phase(game.phase.id).unwrap();
        let events = game
            .whisper(PlayerId(1), PlayerId(2), "secret", game.phase.id)
            .unwrap();
        assert!(events[0].recipients.contains(PlayerId(1)));
        assert!(events[0].recipients.contains(PlayerId(2)));
        assert!(!events[0].recipients.contains(PlayerId(3)));
        assert!(!events[1].text.contains("secret"));
    }

    #[test]
    fn mafia_chat_recipients_exclude_town_and_jester() {
        let roles = Preset::Standard7.roles().to_vec();
        let mut game = Game::with_seed(GameConfig::scenario(roles), 1).unwrap();
        for number in 1..=7 {
            let id = game.join(2000 + number, &format!("P{number}")).unwrap();
            game.toggle_ready(id).unwrap();
        }
        game.start().unwrap();
        game.advance_phase(game.phase.id).unwrap();
        game.advance_phase(game.phase.id).unwrap();
        let event = game
            .mafia_chat(PlayerId(5), "target seven", game.phase.id)
            .unwrap();
        assert!(event.recipients.contains(PlayerId(5)));
        assert!(event.recipients.contains(PlayerId(6)));
        assert!(!event.recipients.contains(PlayerId(1)));
        assert!(!event.recipients.contains(PlayerId(7)));
    }

    #[test]
    fn mafia_numeric_superiority_does_not_end_match_early() {
        let roles = Preset::Standard7.roles().to_vec();
        let mut game = Game::with_seed(GameConfig::scenario(roles), 1).unwrap();
        for number in 1..=7 {
            let id = game.join(3000 + number, &format!("P{number}")).unwrap();
            game.toggle_ready(id).unwrap();
        }
        game.start().unwrap();
        for id in [PlayerId(1), PlayerId(2), PlayerId(3)] {
            game.player_mut(id).unwrap().alive = false;
        }
        assert_eq!(game.evaluate_victory(), MatchOutcome::InProgress);
    }

    #[test]
    fn jail_chat_omits_jailor_identity_and_jailed_mafia_can_only_listen() {
        let roles = Preset::Full9.roles().to_vec();
        let mut game = Game::with_seed(GameConfig::scenario(roles), 4).unwrap();
        for number in 1..=9 {
            let id = game.join(5_000 + number, &format!("P{number}")).unwrap();
            game.toggle_ready(id).unwrap();
        }
        game.start().unwrap();
        game.advance_phase(game.phase.id).unwrap();
        game.select_jail(PlayerId(5), PlayerId(8), game.phase.id)
            .unwrap();
        game.advance_phase(game.phase.id).unwrap();
        let jail_message = game
            .jail_chat(PlayerId(5), "state your role", game.phase.id)
            .unwrap();
        assert!(!jail_message.text.contains("#5"));
        assert_eq!(
            jail_message.recipients,
            Recipients::Players(BTreeSet::from([PlayerId(5), PlayerId(8)]))
        );
        let mafia_message = game
            .mafia_chat(PlayerId(6), "Framer can hear this", game.phase.id)
            .unwrap();
        assert!(mafia_message.recipients.contains(PlayerId(8)));
        assert!(
            game.mafia_chat(PlayerId(8), "I should not send", game.phase.id)
                .is_err()
        );
    }

    #[test]
    fn jester_public_execution_earns_win_and_defers_faction_result_for_haunt() {
        let mut game = nomination_game();
        let phase = game.phase.id;
        for voter in [PlayerId(1), PlayerId(2), PlayerId(3)] {
            game.nominate(voter, PlayerId(5), phase).unwrap();
        }
        game.advance_phase(game.phase.id).unwrap();
        let phase = game.phase.id;
        game.judge(PlayerId(1), JudgmentChoice::Guilty, phase)
            .unwrap();
        game.judge(PlayerId(2), JudgmentChoice::Innocent, phase)
            .unwrap();
        game.judge(PlayerId(3), JudgmentChoice::Guilty, phase)
            .unwrap();
        game.resolve_judgment(phase).unwrap();
        game.advance_phase(game.phase.id).unwrap();
        assert!(game.player(PlayerId(5)).unwrap().jester_won);
        let haunt = game.pending_haunt.as_ref().unwrap();
        assert!(haunt.eligible.contains(&PlayerId(1)));
        assert!(!haunt.eligible.contains(&PlayerId(2)));
        assert!(haunt.eligible.contains(&PlayerId(4)));
        assert_eq!(game.phase.kind, PhaseKind::Night);
        assert_eq!(game.outcome, MatchOutcome::InProgress);
    }

    #[test]
    fn final_results_separate_faction_and_personal_outcomes() {
        let mut game = ready_demo_game();
        game.start().unwrap();
        game.outcome = MatchOutcome::TownWin;
        game.player_mut(PlayerId(5)).unwrap().jester_won = true;
        assert_eq!(
            game.player_outcome(PlayerId(1)).unwrap(),
            PlayerOutcome::Win
        );
        assert_eq!(
            game.player_outcome(PlayerId(4)).unwrap(),
            PlayerOutcome::Loss
        );
        assert_eq!(
            game.player_outcome(PlayerId(5)).unwrap(),
            PlayerOutcome::Win
        );
        game.player_mut(PlayerId(2)).unwrap().forfeited = true;
        assert_eq!(
            game.player_outcome(PlayerId(2)).unwrap(),
            PlayerOutcome::Forfeit
        );
        let final_event = game.final_event();
        assert!(final_event.text.contains("original=Sheriff"));
        assert!(final_event.text.contains("outcome=Forfeit"));
    }
}
