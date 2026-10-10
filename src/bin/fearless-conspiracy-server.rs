use fearless_conspiracy::comm::{
    ClientMessage, PeerId, ServerEvent, ServerMessage, ServerTransport, TransportLimits,
};
use fearless_conspiracy::command;
use fearless_conspiracy::game::{
    Game, GameConfig, GameEvent, MatchOutcome, Phase, PhaseKind, PlayerId, Recipients,
};
use fearless_conspiracy::rate_limit::{RateDecision, RateLimiter, RequestTracker, is_chat_command};
use fearless_conspiracy::server::{
    HostCommand, PhaseClock, SERVER_USAGE, ServerOptions, advance_boundary,
};
use std::collections::HashMap;
use std::env;
use std::io::{self, BufRead};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--help") {
        println!("{SERVER_USAGE}");
        return Ok(());
    }
    let options = ServerOptions::parse(&arguments)?;
    let path = &options.path;
    let preset = options.preset;
    let allow_same_uid = options.allow_same_uid;
    let socket_mode = if options.shared_local { 0o666 } else { 0o660 };
    let config = if options.scenario {
        GameConfig::scenario(preset.roles().to_vec())
    } else {
        GameConfig::ordinary(preset)
    };
    let mut game = Game::from_os_random(config)?;
    let limits = TransportLimits {
        require_unique_uid: !allow_same_uid,
        reject_server_uid: !allow_same_uid,
        socket_mode,
        ..TransportLimits::default()
    };
    let mut transport = ServerTransport::bind(path, limits)?;
    let mut peers = HashMap::<PeerId, PlayerId>::new();
    let mut player_peers = HashMap::<PlayerId, PeerId>::new();
    let mut sequences = HashMap::<PlayerId, u64>::new();
    let mut clock = if options.manual {
        PhaseClock::manual(options.durations)
    } else {
        PhaseClock::new(options.durations)
    };
    let (host_tx, host_rx) = mpsc::sync_channel(16);
    thread::Builder::new()
        .name("fearless-conspiracy-host-console".into())
        .spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if host_tx.send(line).is_err() {
                    break;
                }
            }
        })?;
    let server_started = Instant::now();
    let mut rate_limiter = RateLimiter::default();
    let mut request_tracker = RequestTracker::default();
    let mut final_announced = false;
    println!(
        "Fearless Conspiracy {} listening on {path}; waiting for {} players",
        preset.label(),
        game.config.expected_players()
    );
    println!("{}", options.announcement());

    loop {
        clock.sync(game.phase, Instant::now());
        if game.outcome != MatchOutcome::InProgress && !final_announced {
            deliver_events(
                &transport,
                &player_peers,
                &mut sequences,
                game.phase.id,
                vec![game.final_event(), GameEvent {
                    recipients: Recipients::All,
                    channel: "public_system",
                    text: "Postgame: everyone may chat using /say or bare text. Read commands remain available; gameplay is frozen. Use /quit to leave.".into(),
                }],
            );
            final_announced = true;
        }
        if final_announced && player_peers.is_empty() {
            transport.shutdown_gracefully("all players left postgame", Duration::from_secs(2));
            return Ok(());
        }
        if clock.expired(Instant::now()) {
            let events = advance_and_log(&mut game)?;
            deliver_events(
                &transport,
                &player_peers,
                &mut sequences,
                game.phase.id,
                events,
            );
            continue;
        }

        if let Ok(line) = host_rx.try_recv() {
            match HostCommand::parse(&line) {
                Ok(HostCommand::Help) => println!("{SERVER_USAGE}"),
                Ok(HostCommand::Status) => println!(
                    "Host status: day {} {:?}, phase {}, result {:?}, {} connected players",
                    game.phase.day,
                    game.phase.kind,
                    game.phase.id,
                    game.outcome,
                    player_peers.len()
                ),
                Ok(HostCommand::Advance) if options.manual => match advance_and_log(&mut game) {
                    Ok(events) => deliver_events(
                        &transport,
                        &player_peers,
                        &mut sequences,
                        game.phase.id,
                        events,
                    ),
                    Err(error) => eprintln!("Host: {error}"),
                },
                Ok(HostCommand::Advance) => eprintln!("Host: advance requires --manual"),
                Ok(HostCommand::Abort) => {
                    let events = game.abort();
                    deliver_events(
                        &transport,
                        &player_peers,
                        &mut sequences,
                        game.phase.id,
                        events,
                    );
                }
                Ok(HostCommand::Shutdown) => {
                    if !final_announced {
                        let mut events = game.abort();
                        events.push(game.final_event());
                        deliver_events(
                            &transport,
                            &player_peers,
                            &mut sequences,
                            game.phase.id,
                            events,
                        );
                    }
                    let report =
                        transport.shutdown_gracefully("host shutdown", Duration::from_secs(2));
                    if !report.failed.is_empty() {
                        eprintln!(
                            "Shutdown: {} peers disconnected before output could drain.",
                            report.failed.len()
                        );
                    }
                    return Ok(());
                }
                Err(error) => eprintln!("Host: {error}"),
            }
            continue;
        }
        let wait = clock.wait(Instant::now());
        let Some(event) = transport.recv_timeout(wait)? else {
            continue;
        };
        match event {
            ServerEvent::Connected {
                peer,
                credentials,
                name,
            } => match game.join(
                if allow_same_uid {
                    credentials.uid.saturating_add(peer.0 as u32)
                } else {
                    credentials.uid
                },
                &name,
            ) {
                Ok(player) => {
                    peers.insert(peer, player);
                    player_peers.insert(player, peer);
                    sequences.insert(player, 0);
                    println!("{player} {name} joined the lobby.");
                    print_lobby_readiness(&game);
                    let joined = vec![
                        GameEvent {
                            recipients: Recipients::only(player),
                            channel: "lobby",
                            text: options.announcement(),
                        },
                        GameEvent {
                            recipients: Recipients::only(player),
                            channel: "private_system",
                            text: format!(
                                "You are {player} {name}. Kernel credentials: uid={}, pid={}. Type /ready when prepared.",
                                credentials.uid, credentials.pid
                            ),
                        },
                        GameEvent {
                            recipients: Recipients::All,
                            channel: "lobby",
                            text: format!("{player} {name} joined the lobby."),
                        },
                    ];
                    deliver_events(
                        &transport,
                        &player_peers,
                        &mut sequences,
                        game.phase.id,
                        joined,
                    );
                }
                Err(error) => transport.disconnect(peer, error.to_string()),
            },
            ServerEvent::Message {
                peer,
                message:
                    ClientMessage::Command {
                        request_id,
                        phase_id,
                        text,
                    },
            } => {
                let Some(&player) = peers.get(&peer) else {
                    transport.disconnect(peer, "connection has no admitted player");
                    continue;
                };
                if !request_tracker.accept(player, request_id) {
                    let _ = transport.send_to(
                        peer,
                        ServerMessage::Reply {
                            request_id,
                            accepted: false,
                            text: "duplicate or old request ID".into(),
                        },
                    );
                    continue;
                }
                match rate_limiter.check(player, is_chat_command(&text), server_started.elapsed()) {
                    RateDecision::Allowed => {}
                    RateDecision::Rejected => {
                        let _ = transport.send_to(
                            peer,
                            ServerMessage::Reply {
                                request_id,
                                accepted: false,
                                text: "rate limit exceeded".into(),
                            },
                        );
                        continue;
                    }
                    RateDecision::Disconnect => {
                        transport
                            .disconnect(player_peers[&player], "repeated rate-limit violations");
                        continue;
                    }
                }
                match command::dispatch(&mut game, player, phase_id, &text) {
                    Ok(output) => {
                        log_chat_events(&output.events);
                        deliver_events(
                            &transport,
                            &player_peers,
                            &mut sequences,
                            game.phase.id,
                            output.events,
                        );
                        let _ = transport.send_to(
                            peer,
                            ServerMessage::Reply {
                                request_id,
                                accepted: true,
                                text: success_reply_text(&game, player, &text),
                            },
                        );
                        if text.trim() == "/ready" {
                            print_lobby_readiness(&game);
                        }
                        if output.quit {
                            transport.disconnect(peer, "requested by player");
                        } else if game.can_start() {
                            let events = game.start()?;
                            print_role_roster(&game);
                            deliver_events(
                                &transport,
                                &player_peers,
                                &mut sequences,
                                game.phase.id,
                                events,
                            );
                        }
                    }
                    Err(error) => {
                        let _ = transport.send_to(
                            peer,
                            ServerMessage::Reply {
                                request_id,
                                accepted: false,
                                text: error.to_string(),
                            },
                        );
                    }
                }
            }
            ServerEvent::Message { peer, .. } => {
                transport.disconnect(peer, "unexpected message type");
            }
            ServerEvent::Disconnected { peer, reason } => {
                if let Some(player) = peers.remove(&peer) {
                    rate_limiter.remove(player);
                    request_tracker.remove(player);
                    player_peers.remove(&player);
                    println!("{player} disconnected: {reason}");
                    if game.phase.kind == PhaseKind::Lobby {
                        if let Ok(uid) = game.leave_lobby(player) {
                            transport.release_uid(uid);
                        }
                        print_lobby_readiness(&game);
                    } else {
                        game.mark_disconnected(player)?;
                    }
                }
            }
        }
    }
}

fn success_reply_text(game: &Game, player: PlayerId, command: &str) -> String {
    if command.trim() == "/ready" {
        let status = game
            .player(player)
            .map(|player| if player.ready { "ready" } else { "not ready" })
            .unwrap_or("unknown");
        format!("Ready status confirmed: {status}.")
    } else if is_chat_command(command) {
        "Message delivered.".into()
    } else {
        "accepted".into()
    }
}

fn print_lobby_readiness(game: &Game) {
    if game.phase.kind != PhaseKind::Lobby {
        return;
    }
    println!("Lobby readiness:");
    if game.players.is_empty() {
        println!("  (no connected players)");
        return;
    }
    for player in game.players.values() {
        println!(
            "  {} {}: {}",
            player.id,
            player.name,
            if player.ready { "READY" } else { "not ready" }
        );
    }
}

fn log_chat_events(events: &[GameEvent]) {
    for event in events {
        if matches!(
            event.channel,
            "public" | "whisper" | "mafia" | "jail" | "dead" | "postgame"
        ) {
            let safe_text = event.text.replace('\r', "\\r").replace('\n', "\\n");
            println!("[CHAT:{}] {safe_text}", event.channel.to_uppercase());
        }
    }
}

fn advance_and_log(
    game: &mut Game,
) -> Result<Vec<GameEvent>, fearless_conspiracy::game::GameError> {
    let previous = game.phase;
    let events = advance_boundary(game)?;
    println!(
        "Scene: {} -> {}",
        scene_label(previous),
        scene_label(game.phase)
    );
    Ok(events)
}

fn print_role_roster(game: &Game) {
    println!("Game started. Player roles:");
    for player in game.players.values() {
        let role = player
            .current_role
            .map(|role| role.display_name())
            .unwrap_or("Unassigned");
        println!("  {} {}: {role}", player.id, player.name);
    }
    println!("Current scene: {}", scene_label(game.phase));
}

fn scene_label(phase: Phase) -> String {
    let name = match phase.kind {
        PhaseKind::Lobby => "Lobby",
        PhaseKind::RoleReveal => "Role Reveal",
        PhaseKind::DayOne => "Day 1 Introductions",
        PhaseKind::Night => "Night",
        PhaseKind::Dawn => "Dawn",
        PhaseKind::Discussion => "Discussion",
        PhaseKind::Nominations => "Nominations",
        PhaseKind::Defense => "Defense",
        PhaseKind::Judgment => "Judgment",
        PhaseKind::LastWords => "Last Words",
        PhaseKind::Ended => "Game Ended",
    };
    match phase.kind {
        PhaseKind::Lobby | PhaseKind::Ended => name.into(),
        _ => format!("{name} (Day {})", phase.day),
    }
}

fn deliver_events(
    transport: &ServerTransport,
    player_peers: &HashMap<PlayerId, PeerId>,
    sequences: &mut HashMap<PlayerId, u64>,
    phase_id: u64,
    events: Vec<GameEvent>,
) {
    for event in events {
        let recipients: Vec<_> = match event.recipients {
            Recipients::All => player_peers.keys().copied().collect(),
            Recipients::Players(players) => players.into_iter().collect(),
        };
        for player in recipients {
            let Some(&peer) = player_peers.get(&player) else {
                continue;
            };
            let sequence = sequences.entry(player).or_default();
            *sequence += 1;
            let _ = transport.send_to(
                peer,
                ServerMessage::Event {
                    sequence: *sequence,
                    phase_id,
                    channel: event.channel.into(),
                    text: event.text.clone(),
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fearless_conspiracy::roles::Preset;

    #[test]
    fn only_ready_and_chat_receive_visible_success_text() {
        let mut game = Game::with_seed(GameConfig::ordinary(Preset::Demo5), 1).unwrap();
        let player = game.join(1001, "Alice").unwrap();

        assert_eq!(success_reply_text(&game, player, "/role"), "accepted");
        assert_eq!(
            success_reply_text(&game, player, "hello everyone"),
            "Message delivered."
        );

        game.toggle_ready(player).unwrap();
        assert_eq!(
            success_reply_text(&game, player, "/ready"),
            "Ready status confirmed: ready."
        );
    }
}
