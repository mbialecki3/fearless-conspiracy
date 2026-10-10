use crate::game::{Game, GameError, GameEvent, JudgmentChoice, PhaseKind, PlayerId, Recipients};
use crate::roles::Role;
use crate::roles::{Faction, NightIntent};

#[derive(Clone, Debug, Default)]
pub struct CommandOutput {
    pub events: Vec<GameEvent>,
    pub quit: bool,
}

pub fn dispatch(
    game: &mut Game,
    actor: PlayerId,
    phase_id: u64,
    input: &str,
) -> Result<CommandOutput, GameError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(GameError::InvalidText("command is empty".into()));
    }
    if input == "/quit" {
        return Ok(CommandOutput {
            quit: true,
            ..CommandOutput::default()
        });
    }
    if game.phase.kind == PhaseKind::Ended {
        if let Some(text) = input
            .strip_prefix("/say ")
            .or_else(|| (!input.starts_with('/')).then_some(input))
        {
            return Ok(events(vec![game.postgame_chat(actor, text, phase_id)?]));
        }
        let read_only = matches!(
            input,
            "/help"
                | "/role"
                | "/players"
                | "/graveyard"
                | "/history"
                | "/will show"
                | "/deathnote show"
        ) || input.starts_with("/help ");
        if !read_only {
            return Err(GameError::WrongPhase {
                expected: "postgame chat or read commands; gameplay is frozen",
                actual: game.phase.kind,
            });
        }
    }
    if input == "/help" {
        if game.phase.kind == PhaseKind::Ended {
            return Ok(private(
                actor,
                "Postgame: /say TEXT or bare text chats with everyone. Reads: /role, /players, /graveyard, /history, /will show, /deathnote show, /help ROLE. /quit leaves.",
            ));
        }
        return Ok(private(
            actor,
            "Commands: /ready, /role, /players, /say, /whisper, /vote, /unvote, /guilty, /innocent, /abstain, role night commands, /cancel, /will, /deathnote, /dead, /mafia, /quit.",
        ));
    }
    if let Some(role_name) = input.strip_prefix("/help ") {
        let role = Role::ALL
            .into_iter()
            .find(|role| {
                role.display_name().eq_ignore_ascii_case(role_name)
                    || format!("{role:?}").eq_ignore_ascii_case(role_name)
            })
            .ok_or_else(|| GameError::InvalidText(format!("unknown role {role_name:?}")))?;
        let guide = full_role_help(role);
        return Ok(private(actor, &guide));
    }
    if input == "/ready" {
        ensure_lobby_phase(game, phase_id)?;
        let ready = game.toggle_ready(actor)?;
        return Ok(public(format!(
            "{actor} is {}.",
            if ready { "ready" } else { "not ready" }
        )));
    }
    if input == "/role" {
        let player = game.player(actor)?;
        let text = match player.current_role {
            Some(role) => format!(
                "You are the {} ({:?}). Self-heals: {}. Executions: {}. Role guide: /help {}",
                role.display_name(),
                role.faction(),
                player.self_heals_remaining,
                player.executions_remaining,
                role.display_name()
            ),
            None => "Your role has not been assigned yet.".into(),
        };
        return Ok(private(actor, &text));
    }
    if input == "/players" {
        let text = game
            .players
            .values()
            .map(|player| {
                format!(
                    "{} {} [{}{}]",
                    player.id,
                    player.name,
                    if player.alive { "alive" } else { "dead" },
                    if player.connected {
                        ""
                    } else {
                        ", disconnected"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Ok(private(actor, &text));
    }
    if input == "/graveyard" {
        let text = game
            .players
            .values()
            .filter(|player| !player.alive)
            .map(|player| {
                format!(
                    "{} {} ({})",
                    player.id,
                    player.name,
                    player
                        .current_role
                        .map(Role::display_name)
                        .unwrap_or("unassigned")
                )
            })
            .collect::<Vec<_>>();
        return Ok(private(
            actor,
            &if text.is_empty() {
                "Graveyard: (empty)".into()
            } else {
                format!("Graveyard: {}", text.join(", "))
            },
        ));
    }
    if input == "/history" {
        return Ok(private(
            actor,
            "Your terminal transcript is the ordered history of events authorized for you.",
        ));
    }
    if input == "/will show" {
        return Ok(private(
            actor,
            &format!("Will: {}", game.player(actor)?.will),
        ));
    }
    if input == "/will clear" {
        game.set_will(actor, "")?;
        return Ok(private(actor, "Will cleared."));
    }
    if let Some(text) = input.strip_prefix("/will set ") {
        game.set_will(actor, text)?;
        return Ok(private(actor, "Will replaced."));
    }
    if let Some(text) = input.strip_prefix("/will append ") {
        let mut will = game.player(actor)?.will.clone();
        if !will.is_empty() {
            will.push('\n');
        }
        will.push_str(text);
        game.set_will(actor, &will)?;
        return Ok(private(actor, "Will updated."));
    }
    if input == "/deathnote show" {
        return Ok(private(
            actor,
            &format!("Death note: {}", game.player(actor)?.death_note),
        ));
    }
    if input == "/deathnote clear" {
        game.set_death_note(actor, "")?;
        return Ok(private(actor, "Death note cleared."));
    }
    if let Some(text) = input.strip_prefix("/deathnote set ") {
        game.set_death_note(actor, text)?;
        return Ok(private(actor, "Death note replaced."));
    }
    if let Some(text) = input.strip_prefix("/deathnote append ") {
        let mut note = game.player(actor)?.death_note.clone();
        if !note.is_empty() {
            note.push('\n');
        }
        note.push_str(text);
        game.set_death_note(actor, &note)?;
        return Ok(private(actor, "Death note updated."));
    }
    if let Some(text) = input.strip_prefix("/say ") {
        return Ok(events(vec![game.say(actor, text, phase_id)?]));
    }
    if let Some(arguments) = input.strip_prefix("/whisper ") {
        let (target, text) = target_and_text(arguments)?;
        return Ok(events(game.whisper(actor, target, text, phase_id)?));
    }
    if let Some(text) = input.strip_prefix("/mafia ") {
        return Ok(events(vec![game.mafia_chat(actor, text, phase_id)?]));
    }
    if let Some(text) = input.strip_prefix("/dead ") {
        return Ok(events(vec![game.dead_chat(actor, text, phase_id)?]));
    }
    if let Some(text) = input.strip_prefix("/jailchat ") {
        return Ok(events(vec![game.jail_chat(actor, text, phase_id)?]));
    }
    if let Some(target) = input.strip_prefix("/jail ") {
        return Ok(events(vec![game.select_jail(
            actor,
            player_id(target)?,
            phase_id,
        )?]));
    }
    if input == "/execute" || input == "/spare" {
        let intent = if input == "/execute" {
            NightIntent::Execute
        } else {
            NightIntent::Spare
        };
        return Ok(events(game.select_night_intent(actor, intent, phase_id)?));
    }
    if let Some(target) = input.strip_prefix("/vote ") {
        return Ok(events(game.nominate(
            actor,
            player_id(target)?,
            phase_id,
        )?));
    }
    if input == "/unvote" {
        return Ok(events(vec![game.withdraw_nomination(actor, phase_id)?]));
    }
    if matches!(input, "/guilty" | "/innocent" | "/abstain") {
        let choice = match input {
            "/guilty" => JudgmentChoice::Guilty,
            "/innocent" => JudgmentChoice::Innocent,
            _ => JudgmentChoice::Abstain,
        };
        return Ok(events(vec![game.judge(actor, choice, phase_id)?]));
    }
    if input == "/cancel" {
        return Ok(events(vec![game.cancel_night_intent(actor, phase_id)?]));
    }
    if input == "/heal self" {
        return Ok(events(game.select_night_intent(
            actor,
            NightIntent::Heal(actor),
            phase_id,
        )?));
    }
    for (prefix, constructor) in [
        (
            "/investigate ",
            NightIntent::Investigate as fn(PlayerId) -> NightIntent,
        ),
        ("/heal ", NightIntent::Heal),
        ("/watch ", NightIntent::Watch),
        ("/block ", NightIntent::Block),
        ("/kill ", NightIntent::Kill),
        ("/frame ", NightIntent::Frame),
        ("/haunt ", NightIntent::Haunt),
    ] {
        if let Some(target) = input.strip_prefix(prefix) {
            return Ok(events(game.select_night_intent(
                actor,
                constructor(player_id(target)?),
                phase_id,
            )?));
        }
    }

    if !input.starts_with('/') {
        if !game.player(actor)?.alive {
            return Ok(events(vec![game.dead_chat(actor, input, phase_id)?]));
        }
        if game.phase.kind == PhaseKind::Night
            && game
                .jailed
                .is_some_and(|(jailor, prisoner)| actor == jailor || actor == prisoner)
        {
            return Ok(events(vec![game.jail_chat(actor, input, phase_id)?]));
        }
        if game.phase.kind == PhaseKind::Night
            && game.player(actor)?.faction == Some(Faction::Mafia)
        {
            return Ok(events(vec![game.mafia_chat(actor, input, phase_id)?]));
        }
        return Ok(events(vec![game.say(actor, input, phase_id)?]));
    }
    Err(GameError::InvalidText(format!("unknown command {input:?}")))
}

fn ensure_lobby_phase(game: &Game, supplied: u64) -> Result<(), GameError> {
    if supplied != game.phase.id {
        return Err(GameError::StalePhase {
            supplied,
            current: game.phase.id,
        });
    }
    if game.phase.kind != PhaseKind::Lobby {
        return Err(GameError::WrongPhase {
            expected: "lobby",
            actual: game.phase.kind,
        });
    }
    Ok(())
}

fn player_id(text: &str) -> Result<PlayerId, GameError> {
    let number = text
        .trim()
        .trim_start_matches('#')
        .parse::<u8>()
        .map_err(|_| GameError::InvalidTarget("expected a numeric player ID".into()))?;
    Ok(PlayerId(number))
}

fn target_and_text(arguments: &str) -> Result<(PlayerId, &str), GameError> {
    let (target, text) = arguments
        .split_once(' ')
        .filter(|(_, text)| !text.is_empty())
        .ok_or_else(|| GameError::InvalidText("target and message text are required".into()))?;
    Ok((player_id(target)?, text))
}

fn private(actor: PlayerId, text: &str) -> CommandOutput {
    events(vec![GameEvent {
        recipients: Recipients::only(actor),
        channel: "private_system",
        text: text.into(),
    }])
}

fn public(text: String) -> CommandOutput {
    events(vec![GameEvent {
        recipients: Recipients::All,
        channel: "public_system",
        text,
    }])
}

fn events(events: Vec<GameEvent>) -> CommandOutput {
    CommandOutput {
        events,
        quit: false,
    }
}

fn role_help(role: Role) -> &'static str {
    match role {
        Role::Sheriff => {
            "Sheriff — Town Investigative\nObjective: Help Town identify and eliminate the Mafia.\nAbility: Investigate one living player at night. Suspicious results can identify a Mafioso or Framer, but framing can create false evidence and some roles appear innocent.\nCommands: /investigate ID, /cancel"
        }
        Role::Doctor => {
            "Doctor — Town Protective\nObjective: Keep Town players alive while Town finds the Mafia.\nAbility: Heal one living player at night. You may heal yourself once per game.\nCommands: /heal ID, /heal self, /cancel"
        }
        Role::Lookout => {
            "Lookout — Town Investigative\nObjective: Track visits and compare them with player claims.\nAbility: Watch one other living player at night and learn up to three players who visited them.\nCommands: /watch ID, /cancel"
        }
        Role::TavernKeeper => {
            "Escort — Town Support\nObjective: Disrupt suspicious players without blocking important Town abilities.\nAbility: Block one living player's night ability. You are immune to ordinary role blocking.\nCommands: /block ID, /cancel"
        }
        Role::Jailor => {
            "Jailor — Town Killing\nObjective: Question suspicious players and execute confirmed threats.\nAbility: Select a prisoner during the day. At night, speak privately with the prisoner and choose whether to execute them. Night 1 executions are not allowed.\nCommands: /jail ID, /jailchat MESSAGE, /execute, /spare, /cancel"
        }
        Role::Godfather => {
            "Godfather — Mafia Killing\nObjective: Eliminate Town while hiding your identity.\nAbility: Choose the Mafia's target. If the Mafioso can act, they normally perform your ordered attack. You have Basic defense and appear innocent to the Sheriff.\nCommands: /kill ID, /cancel, /mafia MESSAGE, /deathnote set TEXT"
        }
        Role::Mafioso => {
            "Mafioso — Mafia Killing\nObjective: Eliminate Town while protecting your Mafia teammates.\nAbility: Perform the Mafia attack. A Godfather's valid order takes priority over your fallback target.\nCommands: /kill ID, /cancel, /mafia MESSAGE, /deathnote set TEXT"
        }
        Role::Framer => {
            "Framer — Mafia Deception\nObjective: Mislead Town investigations and help the Mafia survive.\nAbility: Frame one non-Mafia player at night so a Sheriff investigation reports them as suspicious.\nCommands: /frame ID, /cancel, /mafia MESSAGE"
        }
        Role::Jester => {
            "Jester — Neutral Evil\nObjective: Convince the Town to convict and publicly execute you.\nAbility: After your execution, haunt an eligible living player who voted guilty or abstained.\nCommands after execution: /haunt ID, /cancel"
        }
    }
}

fn full_role_help(role: Role) -> String {
    format!(
        "{}\nTarget IDs: run /players to see every player's ID, then use that number in commands such as /watch ID or /investigate ID.\nCommon commands: /role, /players, /say MESSAGE, /whisper ID MESSAGE, /will set TEXT, /will show, /help, /quit",
        role_help(role)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameConfig;
    use crate::roles::Preset;

    #[test]
    fn postgame_allows_reads_and_shared_chat_but_freezes_mutations_and_results() {
        let mut game =
            Game::with_seed(GameConfig::scenario(Preset::Demo5.roles().to_vec()), 1).unwrap();
        for uid in 1..=5 {
            let id = game.join(uid, &format!("Player{uid}")).unwrap();
            game.toggle_ready(id).unwrap();
        }
        game.start().unwrap();
        game.player_mut(PlayerId(1)).unwrap().alive = false;
        game.abort();
        let phase = game.phase.id;
        for actor in [PlayerId(1), PlayerId(2)] {
            for input in ["good game", "/say hello"] {
                let output = dispatch(&mut game, actor, phase, input).unwrap();
                assert_eq!(output.events[0].channel, "postgame");
                assert_eq!(output.events[0].recipients, Recipients::All);
            }
            for input in [
                "/role",
                "/players",
                "/graveyard",
                "/history",
                "/will show",
                "/deathnote show",
                "/help",
                "/help Doctor",
            ] {
                dispatch(&mut game, actor, phase, input).unwrap();
            }
            for input in [
                "/ready",
                "/vote 3",
                "/unvote",
                "/guilty",
                "/cancel",
                "/heal self",
                "/will set changed",
                "/deathnote set changed",
                "/mafia private",
                "/jail 3",
            ] {
                assert!(dispatch(&mut game, actor, phase, input).is_err(), "{input}");
            }
        }
        assert!(dispatch(&mut game, PlayerId(2), phase - 1, "late chat").is_err());
        assert!(game.set_will(PlayerId(2), "changed").is_err());
        assert!(game.set_death_note(PlayerId(4), "changed").is_err());
        let result = game.final_event();
        game.mark_disconnected(PlayerId(2)).unwrap();
        assert!(!game.player(PlayerId(2)).unwrap().pending_forfeit);
        assert!(game.apply_pending_forfeits().is_empty());
        assert_eq!(game.final_event(), result);
        assert_eq!(game.phase.id, phase);
    }

    #[test]
    fn parser_routes_ready_and_rejects_stale_phase() {
        let mut game = Game::with_seed(GameConfig::ordinary(Preset::Demo5), 1).unwrap();
        let player = game.join(1, "Alice").unwrap();
        dispatch(&mut game, player, 0, "/ready").unwrap();
        assert!(game.player(player).unwrap().ready);
        assert!(matches!(
            dispatch(&mut game, player, 99, "/ready"),
            Err(GameError::StalePhase { .. })
        ));
    }

    #[test]
    fn parser_never_treats_unknown_slash_command_as_chat() {
        let mut game = Game::with_seed(GameConfig::ordinary(Preset::Demo5), 1).unwrap();
        let player = game.join(1, "Alice").unwrap();
        assert!(matches!(
            dispatch(&mut game, player, 0, "/does-not-exist"),
            Err(GameError::InvalidText(_))
        ));
    }

    #[test]
    fn every_role_guide_explains_its_objective_and_commands() {
        for role in Role::ALL {
            let guide = full_role_help(role);
            assert!(guide.contains(role.display_name()), "{role:?}");
            assert!(guide.contains("Objective:"), "{role:?}");
            assert!(guide.contains("Commands"), "{role:?}");
            assert!(guide.contains('/'), "{role:?}");
            assert!(guide.contains("run /players"), "{role:?}");
        }
    }
}
