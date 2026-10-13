//! Server scheduling and host policy, independent of socket I/O.

use crate::game::{Game, GameError, GameEvent, MatchOutcome, Phase, PhaseKind};
use crate::roles::Preset;
use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Durations {
    pub role_reveal: Duration,
    pub day_one: Duration,
    pub night: Duration,
    pub dawn: Duration,
    pub discussion: Duration,
    pub nominations: Duration,
    pub defense: Duration,
    pub judgment: Duration,
    pub last_words: Duration,
}

impl Default for Durations {
    fn default() -> Self {
        Self {
            role_reveal: Duration::from_secs(10),
            day_one: Duration::from_secs(30),
            night: Duration::from_secs(60),
            dawn: Duration::from_secs(10),
            discussion: Duration::from_secs(90),
            nominations: Duration::from_secs(60),
            defense: Duration::from_secs(30),
            judgment: Duration::from_secs(20),
            last_words: Duration::from_secs(15),
        }
    }
}

impl Durations {
    pub fn fast() -> Self {
        Self {
            role_reveal: Duration::from_millis(200),
            day_one: Duration::from_millis(300),
            night: Duration::from_millis(500),
            dawn: Duration::from_millis(200),
            discussion: Duration::from_millis(300),
            nominations: Duration::from_millis(500),
            defense: Duration::from_millis(300),
            judgment: Duration::from_millis(300),
            last_words: Duration::from_millis(200),
        }
    }

    pub fn for_phase(self, phase: PhaseKind) -> Option<Duration> {
        match phase {
            PhaseKind::RoleReveal => Some(self.role_reveal),
            PhaseKind::DayOne => Some(self.day_one),
            PhaseKind::Night => Some(self.night),
            PhaseKind::Dawn => Some(self.dawn),
            PhaseKind::Discussion => Some(self.discussion),
            PhaseKind::Nominations => Some(self.nominations),
            PhaseKind::Defense => Some(self.defense),
            PhaseKind::Judgment => Some(self.judgment),
            PhaseKind::LastWords => Some(self.last_words),
            PhaseKind::Lobby | PhaseKind::Ended => None,
        }
    }
}

pub struct PhaseClock {
    durations: Durations,
    observed: Option<Phase>,
    deadline: Option<Instant>,
    nomination_budget: Option<(u8, Duration)>,
    manual: bool,
}

impl PhaseClock {
    pub fn new(durations: Durations) -> Self {
        Self {
            durations,
            observed: None,
            deadline: None,
            nomination_budget: None,
            manual: false,
        }
    }

    pub fn manual(durations: Durations) -> Self {
        Self {
            manual: true,
            ..Self::new(durations)
        }
    }

    pub fn sync(&mut self, phase: Phase, now: Instant) {
        if self.observed == Some(phase) {
            return;
        }
        if let Some(previous) = self.observed
            && previous.kind == PhaseKind::Nominations
            && let Some(deadline) = self.deadline
        {
            self.nomination_budget = Some((previous.day, deadline.saturating_duration_since(now)));
        }
        let duration = if phase.kind == PhaseKind::Nominations {
            let remaining = self
                .nomination_budget
                .filter(|(day, _)| *day == phase.day)
                .map(|(_, remaining)| remaining)
                .unwrap_or(self.durations.nominations);
            self.nomination_budget = Some((phase.day, remaining));
            Some(remaining)
        } else {
            self.durations.for_phase(phase.kind)
        };
        self.deadline = duration
            .filter(|_| !self.manual)
            .map(|duration| now + duration);
        self.observed = Some(phase);
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }

    pub fn wait(&self, now: Instant) -> Duration {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(now))
            .unwrap_or(Duration::from_millis(100))
            .min(Duration::from_millis(100))
    }
}

pub const SERVER_USAGE: &str =
    "Usage: fearless-conspiracy-server [SOCKET] [--players 3|4|5|6|7|8|9]
  [--preset demo3|standard4|demo5|standard6|standard7|standard8|full9] [--setup]
  [--scenario] [--manual] [--fast] [--allow-same-uid] [--shared-local]
  [--duration PHASE=SECONDS]...
Host commands: help, status, advance, abort, shutdown.";

#[derive(Debug)]
pub struct ServerOptions {
    pub setup: bool,
    pub path: String,
    pub preset: Preset,
    pub scenario: bool,
    pub manual: bool,
    pub allow_same_uid: bool,
    pub shared_local: bool,
    pub durations: Durations,
}

impl ServerOptions {
    pub fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut options = Self {
            setup: false,
            path: "/run/fearless-conspiracy/game.sock".into(),
            preset: Preset::Demo5,
            scenario: false,
            manual: false,
            allow_same_uid: false,
            shared_local: false,
            durations: Durations::default(),
        };
        let mut path_set = false;
        let mut overrides = Vec::new();
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--setup" => options.setup = true,
                "--scenario" => options.scenario = true,
                "--manual" => options.manual = true,
                "--allow-same-uid" => options.allow_same_uid = true,
                "--shared-local" => options.shared_local = true,
                "--fast" => options.durations = Durations::fast(),
                "--players" => {
                    options.preset = match arguments.next().map(String::as_str) {
                        Some("3") => Preset::Demo3,
                        Some("4") => Preset::Standard4,
                        Some("5") => Preset::Demo5,
                        Some("6") => Preset::Standard6,
                        Some("7") => Preset::Standard7,
                        Some("8") => Preset::Standard8,
                        Some("9") => Preset::Full9,
                        _ => return Err("--players requires a number from 3 through 9".into()),
                    };
                }
                "--preset" => {
                    options.preset = match arguments.next().map(String::as_str) {
                        Some("demo3" | "3") => Preset::Demo3,
                        Some("standard4" | "4") => Preset::Standard4,
                        Some("demo5" | "5") => Preset::Demo5,
                        Some("standard6" | "6") => Preset::Standard6,
                        Some("standard7" | "7") => Preset::Standard7,
                        Some("standard8" | "8") => Preset::Standard8,
                        Some("full9" | "9") => Preset::Full9,
                        _ => {
                            return Err(
                                "--preset requires demo3, standard4, demo5, standard6, standard7, standard8, or full9".into()
                            );
                        }
                    };
                }
                "--duration" => overrides.push(
                    arguments
                        .next()
                        .ok_or("--duration requires PHASE=SECONDS")?,
                ),
                value if value.starts_with('-') => return Err(format!("unknown option {value}")),
                value if !path_set => {
                    options.path = value.into();
                    path_set = true;
                }
                value => return Err(format!("unexpected argument {value}")),
            }
        }
        for value in overrides {
            let (phase, seconds) = value
                .split_once('=')
                .ok_or("--duration requires PHASE=SECONDS")?;
            let seconds: f64 = seconds.parse().map_err(|_| "invalid duration in seconds")?;
            if !(0.001..=3600.0).contains(&seconds) {
                return Err("duration must be between 0.001 and 3600 seconds".into());
            }
            let duration = Duration::try_from_secs_f64(seconds).map_err(|_| "invalid duration")?;
            let slot = match phase {
                "role-reveal" => &mut options.durations.role_reveal,
                "day-one" => &mut options.durations.day_one,
                "night" => &mut options.durations.night,
                "dawn" => &mut options.durations.dawn,
                "discussion" => &mut options.durations.discussion,
                "nominations" => &mut options.durations.nominations,
                "defense" => &mut options.durations.defense,
                "judgment" => &mut options.durations.judgment,
                "last-words" => &mut options.durations.last_words,
                _ => return Err(format!("unknown duration phase {phase}")),
            };
            *slot = duration;
        }
        Ok(options)
    }

    /// Read setup before starting the host console or binding the game socket.
    /// Explicit command-line settings supply defaults for the questions.
    pub fn configure_lobby(
        &mut self,
        input: &mut impl BufRead,
        output: &mut impl Write,
    ) -> io::Result<()> {
        writeln!(output, "Fearless Conspiracy lobby setup")?;
        writeln!(
            output,
            "Press Enter to accept a default; Ctrl+C to cancel.\n"
        )?;
        let players = prompt_choice(
            input,
            output,
            "How many players do you want? (3-9)",
            &self.preset.roles().len().to_string(),
            &["3", "4", "5", "6", "7", "8", "9"],
        )?;
        self.preset = match players.as_str() {
            "3" => Preset::Demo3,
            "4" => Preset::Standard4,
            "5" => Preset::Demo5,
            "6" => Preset::Standard6,
            "7" => Preset::Standard7,
            "8" => Preset::Standard8,
            "9" => Preset::Full9,
            _ => unreachable!("prompt validates player count"),
        };
        self.manual = prompt_yes_no(
            input,
            output,
            "Do you want manual phase control? (use advance; disables timers)",
            self.manual,
        )?;
        self.scenario = prompt_yes_no(
            input,
            output,
            "Do you want predictable roles? (assigned in player ID order)",
            self.scenario,
        )?;
        if !self.manual {
            writeln!(
                output,
                "Normal timers give players time to act; fast timers use fractions of a second for testing."
            )?;
            match prompt_choice(
                input,
                output,
                "Timer speed (current keeps existing settings; normal/fast)",
                "current",
                &["current", "normal", "fast"],
            )?
            .as_str()
            {
                "normal" => self.durations = Durations::default(),
                "fast" => self.durations = Durations::fast(),
                _ => {}
            }
        }
        self.shared_local = prompt_yes_no(
            input,
            output,
            "Allow players from other local Linux accounts to connect?",
            self.shared_local,
        )?;
        let roles = self
            .preset
            .roles()
            .iter()
            .enumerate()
            .map(|(index, role)| {
                if self.scenario {
                    format!("#{} {}", index + 1, role.display_name())
                } else {
                    role.display_name().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(output, "\nRoles: {roles}")?;
        writeln!(
            output,
            "Starting a {}-player lobby.\n",
            self.preset.roles().len()
        )?;
        Ok(())
    }

    pub fn announcement(&self) -> String {
        let durations = self.durations;
        let times = [
            ("role-reveal", durations.role_reveal),
            ("day-one", durations.day_one),
            ("night", durations.night),
            ("dawn", durations.dawn),
            ("discussion", durations.discussion),
            ("nominations", durations.nominations),
            ("defense", durations.defense),
            ("judgment", durations.judgment),
            ("last-words", durations.last_words),
        ]
        .into_iter()
        .map(|(name, duration)| format!("{name}={}s", duration.as_secs_f64()))
        .collect::<Vec<_>>()
        .join(", ");
        format!(
            "Match settings: {}; {}; {}. {} {} Phase durations: {times}{}.",
            self.preset.label(),
            if self.scenario {
                "SCENARIO: predetermined roles"
            } else {
                "randomized roles"
            },
            if self.manual {
                "MANUAL: host advances phases; timers disabled"
            } else {
                "TIMED: server deadlines"
            },
            if self.allow_same_uid {
                "TEST ONLY: separate-UID checks disabled."
            } else {
                "Distinct Linux UIDs required."
            },
            if self.shared_local {
                "SHARED LOCAL: the socket permits connections from any local user."
            } else {
                "PRIVATE SOCKET: owner/group access only."
            },
            if self.manual {
                " (inactive in manual mode)"
            } else {
                ""
            }
        )
    }
}

fn prompt_choice(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
    default: &str,
    choices: &[&str],
) -> io::Result<String> {
    loop {
        write!(output, "{question} [{default}]: ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "lobby setup cancelled: input closed before setup finished",
            ));
        }
        let answer = line.trim().to_ascii_lowercase();
        if answer.is_empty() {
            return Ok(default.to_string());
        }
        if choices.contains(&answer.as_str()) {
            return Ok(answer);
        }
        writeln!(output, "Please enter {}.", choices.join(" / "))?;
    }
}

fn prompt_yes_no(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
    default: bool,
) -> io::Result<bool> {
    let answer = prompt_choice(
        input,
        output,
        question,
        if default { "yes" } else { "no" },
        &["y", "yes", "n", "no"],
    )?;
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum HostCommand {
    Help,
    Status,
    Advance,
    Abort,
    Shutdown,
}

impl HostCommand {
    pub fn parse(line: &str) -> Result<Self, &'static str> {
        match line.trim() {
            "help" => Ok(Self::Help),
            "status" => Ok(Self::Status),
            "advance" => Ok(Self::Advance),
            "abort" => Ok(Self::Abort),
            "shutdown" => Ok(Self::Shutdown),
            _ => Err("unknown host command; use help, status, advance, abort, or shutdown"),
        }
    }
}

/// Commit forfeits before resolving a boundary. If that cancels the trial,
/// give the resumed phase its own deadline instead of advancing it again.
pub fn advance_boundary(game: &mut Game) -> Result<Vec<GameEvent>, GameError> {
    if game.outcome != MatchOutcome::InProgress {
        return Err(GameError::WrongPhase {
            expected: "an active match",
            actual: game.phase.kind,
        });
    }
    let phase_id = game.phase.id;
    let mut events = game.apply_pending_forfeits();
    if game.outcome == MatchOutcome::InProgress && game.phase.id == phase_id {
        events.extend(match game.phase.kind {
            PhaseKind::Night => game.resolve_night()?.events,
            PhaseKind::Judgment => game.resolve_judgment(phase_id)?.1,
            _ => game.advance_phase(phase_id)?,
        });
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameConfig, PlayerId};
    use crate::roles::Preset;

    #[test]
    fn player_counts_select_presets_and_reject_unsupported_sizes() {
        for players in 3..=9 {
            let options = ServerOptions::parse(&["--players".into(), players.to_string()]).unwrap();
            assert_eq!(options.preset.roles().len(), players);
            let scenario = GameConfig::scenario(options.preset.roles().to_vec());
            assert_eq!(scenario.preset, options.preset);
            assert_eq!(scenario.expected_players(), players);
        }
        for args in [
            vec!["--players"],
            vec!["--players", "2"],
            vec!["--players", "10"],
            vec!["--players", "six"],
        ] {
            assert!(
                ServerOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>())
                    .is_err()
            );
        }
        assert_eq!(
            ServerOptions::parse(&["--preset".into(), "standard6".into()])
                .unwrap()
                .preset,
            Preset::Standard6
        );
    }

    #[test]
    fn setup_retries_invalid_answers_and_leaves_host_commands_unread() {
        let mut options = ServerOptions::parse(&["--setup".into()]).unwrap();
        let mut input = &b"2\nsix\n 6 \nmaybe\n YES \ny\nn\nstatus\n"[..];
        let mut output = Vec::new();
        options.configure_lobby(&mut input, &mut output).unwrap();
        assert_eq!(options.preset, Preset::Standard6);
        assert!(options.manual);
        assert!(options.scenario);
        assert!(!options.shared_local);
        assert_eq!(input, b"status\n");
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Please enter 3 / 4 / 5 / 6 / 7 / 8 / 9"));
        assert!(output.contains("Please enter y / yes / n / no"));
        assert!(output.contains("#4 Escort"));
        assert!(!output.contains("Timer speed"));
    }

    #[test]
    fn setup_accepts_small_counts_and_named_presets_as_defaults() {
        for (players, name, preset) in [
            (3, "demo3", Preset::Demo3),
            (4, "standard4", Preset::Standard4),
        ] {
            for alias in [name.to_string(), players.to_string()] {
                let options = ServerOptions::parse(&["--preset".into(), alias]).unwrap();
                assert_eq!(options.preset, preset);
            }
            let mut options = ServerOptions::parse(&["--preset".into(), name.into()]).unwrap();
            options
                .configure_lobby(&mut &b"\n\n\n\n\n"[..], &mut Vec::new())
                .unwrap();
            assert_eq!(options.preset, preset);
            let mut options = ServerOptions::parse(&[]).unwrap();
            let answers = format!("{players}\ny\ny\nn\n");
            let mut output = Vec::new();
            options
                .configure_lobby(&mut answers.as_bytes(), &mut output)
                .unwrap();
            assert_eq!(options.preset, preset);
            assert!(
                String::from_utf8(output)
                    .unwrap()
                    .contains(&format!("#{players} Mafioso"))
            );
        }
    }

    #[test]
    fn small_matches_can_resolve_night_actions_and_execute_mafia() {
        use crate::game::{JudgmentChoice, Recipients};
        use crate::roles::NightIntent;

        for preset in [Preset::Demo3, Preset::Standard4] {
            let mut game =
                Game::with_seed(GameConfig::scenario(preset.roles().to_vec()), 7).unwrap();
            let mafia = PlayerId(preset.roles().len() as u8);
            for number in 1..=mafia.0 {
                let player = game.join(number.into(), &format!("P{number}")).unwrap();
                game.toggle_ready(player).unwrap();
            }
            game.start().unwrap();
            advance_boundary(&mut game).unwrap();
            advance_boundary(&mut game).unwrap();
            assert_eq!(game.phase.kind, PhaseKind::Night);
            let phase = game.phase.id;
            game.select_night_intent(PlayerId(1), NightIntent::Investigate(mafia), phase)
                .unwrap();
            game.select_night_intent(PlayerId(2), NightIntent::Heal(PlayerId(1)), phase)
                .unwrap();
            game.select_night_intent(mafia, NightIntent::Kill(PlayerId(1)), phase)
                .unwrap();
            if preset == Preset::Standard4 {
                game.select_night_intent(PlayerId(3), NightIntent::Watch(PlayerId(1)), phase)
                    .unwrap();
            }
            let events = advance_boundary(&mut game).unwrap();
            assert!(
                events
                    .iter()
                    .any(|event| event.recipients == Recipients::only(PlayerId(1))
                        && event.text.contains("suspicious"))
            );
            assert!(game.players.values().all(|player| player.alive));
            advance_boundary(&mut game).unwrap();
            advance_boundary(&mut game).unwrap();
            assert_eq!(game.phase.kind, PhaseKind::Nominations);
            let phase = game.phase.id;
            for number in 1..mafia.0 {
                game.nominate(PlayerId(number), mafia, phase).unwrap();
            }
            assert_eq!(game.phase.kind, PhaseKind::Defense);
            advance_boundary(&mut game).unwrap();
            let phase = game.phase.id;
            for number in 1..mafia.0 {
                game.judge(PlayerId(number), JudgmentChoice::Guilty, phase)
                    .unwrap();
            }
            advance_boundary(&mut game).unwrap();
            assert_eq!(game.phase.kind, PhaseKind::LastWords);
            advance_boundary(&mut game).unwrap();
            assert_eq!(game.outcome, MatchOutcome::TownWin);
            assert_eq!(game.phase.kind, PhaseKind::Ended);
        }
    }

    #[test]
    fn setup_defaults_preserve_flags_and_duration_overrides() {
        let mut options = ServerOptions::parse(
            &[
                "--setup",
                "--players",
                "8",
                "--scenario",
                "--shared-local",
                "--duration",
                "night=42",
            ]
            .map(String::from),
        )
        .unwrap();
        options
            .configure_lobby(&mut &b"\n\n\n\n\n"[..], &mut Vec::new())
            .unwrap();
        assert_eq!(options.preset, Preset::Standard8);
        assert!(options.scenario);
        assert!(!options.manual);
        assert!(options.shared_local);
        assert_eq!(options.durations.night, Duration::from_secs(42));
        assert!(!options.allow_same_uid);
    }

    #[test]
    fn setup_can_change_flags_and_select_automatic_timer_speed() {
        for (speed, night) in [
            ("normal", Duration::from_secs(60)),
            ("fast", Duration::from_millis(500)),
        ] {
            let mut options = ServerOptions::parse(
                &[
                    "--manual",
                    "--scenario",
                    "--shared-local",
                    "--fast",
                    "--allow-same-uid",
                ]
                .map(String::from),
            )
            .unwrap();
            let answers = format!("9\nno\nno\n{speed}\nno\n");
            options
                .configure_lobby(&mut answers.as_bytes(), &mut Vec::new())
                .unwrap();
            assert_eq!(options.preset, Preset::Full9);
            assert!(!options.manual);
            assert!(!options.scenario);
            assert!(!options.shared_local);
            assert!(options.allow_same_uid);
            assert_eq!(options.durations.night, night);
        }
    }

    #[test]
    fn setup_aborts_on_eof_instead_of_starting_with_partial_answers() {
        for answers in ["", "6\n", "6\ny\ny\n", "6\nn\nn\nnormal\n"] {
            let mut options = ServerOptions::parse(&[]).unwrap();
            let error = options
                .configure_lobby(&mut answers.as_bytes(), &mut Vec::new())
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
    }

    #[test]
    fn duration_options_validate_and_override_fast_independent_of_order() {
        let args = [
            "--duration",
            "night=1.25",
            "--fast",
            "--manual",
            "--scenario",
        ]
        .map(String::from);
        let options = ServerOptions::parse(&args).unwrap();
        assert_eq!(options.durations.night, Duration::from_millis(1250));
        assert!(options.announcement().contains("SCENARIO"));
        assert!(options.announcement().contains("timers disabled"));
        for value in [
            "night=0",
            "night=-1",
            "night=NaN",
            "night=inf",
            "night=3601",
            "lobby=1",
            "night",
        ] {
            assert!(ServerOptions::parse(&["--duration".into(), value.into()]).is_err());
        }
        for args in [
            vec!["--preset"],
            vec!["--duration"],
            vec!["--unknown"],
            vec!["one", "two"],
        ] {
            assert!(
                ServerOptions::parse(&args.into_iter().map(String::from).collect::<Vec<_>>())
                    .is_err()
            );
        }
    }

    #[test]
    fn shared_local_is_explicit_and_announced() {
        let default = ServerOptions::parse(&[]).unwrap();
        assert!(!default.shared_local);

        let shared = ServerOptions::parse(&["--shared-local".into()]).unwrap();
        assert!(shared.shared_local);
        assert!(shared.announcement().contains("any local user"));
        assert!(SERVER_USAGE.contains("--shared-local"));
    }

    #[test]
    fn manual_mode_never_expires_but_majority_nominations_still_trigger_trials() {
        let mut game = nominations();
        let mut clock = PhaseClock::manual(Durations::fast());
        let now = Instant::now();
        clock.sync(game.phase, now);
        assert!(!clock.expired(now + Duration::from_secs(86400)));
        trial(&mut game);
        clock.sync(game.phase, now);
        assert!(!clock.expired(now + Duration::from_secs(86400)));
        advance_boundary(&mut game).unwrap();
        assert_eq!(game.phase.kind, PhaseKind::Judgment);
    }

    fn nominations() -> Game {
        let mut game =
            Game::with_seed(GameConfig::scenario(Preset::Demo5.roles().to_vec()), 7).unwrap();
        for id in 1..=5 {
            let player = game.join(id, &format!("Player{id}")).unwrap();
            game.toggle_ready(player).unwrap();
        }
        game.start().unwrap();
        while game.phase.kind != PhaseKind::Nominations {
            advance_boundary(&mut game).unwrap();
        }
        game
    }

    fn trial(game: &mut Game) {
        for voter in [1, 3, 4] {
            game.nominate(PlayerId(voter), PlayerId(2), game.phase.id)
                .unwrap();
        }
        assert_eq!(game.phase.kind, PhaseKind::Defense);
    }

    #[test]
    fn acquittals_share_one_nomination_budget_and_next_day_resets_it() {
        let mut game = nominations();
        let mut clock = PhaseClock::new(Durations::default());
        let start = Instant::now();
        clock.sync(game.phase, start);
        trial(&mut game);
        clock.sync(game.phase, start + Duration::from_secs(17));
        advance_boundary(&mut game).unwrap();
        clock.sync(game.phase, start + Duration::from_secs(47));
        advance_boundary(&mut game).unwrap(); // abstentions acquit
        let resumed = start + Duration::from_secs(67);
        clock.sync(game.phase, resumed);
        assert_eq!(clock.deadline, Some(resumed + Duration::from_secs(43)));
        trial(&mut game);
        clock.sync(game.phase, resumed + Duration::from_secs(10));
        advance_boundary(&mut game).unwrap();
        clock.sync(game.phase, resumed + Duration::from_secs(40));
        advance_boundary(&mut game).unwrap();
        let resumed_again = resumed + Duration::from_secs(60);
        clock.sync(game.phase, resumed_again);
        assert!(!clock.expired(resumed_again + Duration::from_secs(32)));
        assert!(clock.expired(resumed_again + Duration::from_secs(33)));
        advance_boundary(&mut game).unwrap();
        clock.sync(game.phase, resumed_again + Duration::from_secs(33));
        while game.phase.kind != PhaseKind::Nominations {
            advance_boundary(&mut game).unwrap();
        }
        clock.sync(game.phase, resumed_again + Duration::from_secs(100));
        assert_eq!(
            clock.deadline,
            Some(resumed_again + Duration::from_secs(160))
        );
    }

    #[test]
    fn accused_forfeit_resumes_remaining_time_without_skipping_nominations() {
        let mut game = nominations();
        let mut clock = PhaseClock::new(Durations::default());
        let now = Instant::now();
        clock.sync(game.phase, now);
        trial(&mut game);
        clock.sync(game.phase, now + Duration::from_secs(59));
        game.mark_disconnected(PlayerId(2)).unwrap();
        advance_boundary(&mut game).unwrap();
        assert_eq!(game.phase.kind, PhaseKind::Nominations);
        clock.sync(game.phase, now + Duration::from_secs(89));
        assert!(!clock.expired(now + Duration::from_millis(89999)));
        assert!(clock.expired(now + Duration::from_secs(90)));
    }

    #[test]
    fn exhausted_nomination_budget_is_not_replenished_by_a_trial() {
        let now = Instant::now();
        let mut clock = PhaseClock::new(Durations::default());
        clock.sync(
            Phase {
                id: 1,
                kind: PhaseKind::Nominations,
                day: 2,
            },
            now,
        );
        clock.sync(
            Phase {
                id: 2,
                kind: PhaseKind::Defense,
                day: 2,
            },
            now + Duration::from_secs(60),
        );
        clock.sync(
            Phase {
                id: 3,
                kind: PhaseKind::Nominations,
                day: 2,
            },
            now + Duration::from_secs(110),
        );
        assert!(clock.expired(now + Duration::from_secs(110)));
    }
}
