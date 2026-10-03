use crate::game::{Game, GameError, GameEvent, JudgmentChoice, PhaseKind, PlayerId, Recipients};
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
    if input == "/help" {
        return Ok(private(
            actor,
            "Commands: /ready, /role, /players, /say, /whisper, /vote, /unvote, /guilty, /innocent, /abstain, role night commands, /cancel, /will, /deathnote, /dead, /mafia, /quit.",
        ));
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
                "You are the {} ({:?}). Self-heals: {}. Executions: {}.",
                role.display_name(),
                role.faction(),
                player.self_heals_remaining,
                player.executions_remaining
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameConfig;
    use crate::roles::Preset;

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
}
