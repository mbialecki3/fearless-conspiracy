use std::collections::HashMap;
use std::env;
use std::time::{Duration, Instant};
use terminal_salem::comm::{
    ClientMessage, PeerId, ServerEvent, ServerMessage, ServerTransport, TransportLimits,
};
use terminal_salem::command;
use terminal_salem::game::{
    Game, GameConfig, GameEvent, MatchOutcome, PhaseKind, PlayerId, Recipients,
};
use terminal_salem::rate_limit::{RateDecision, RateLimiter, RequestTracker, is_chat_command};
use terminal_salem::roles::Preset;
use terminal_salem::server::{Durations, PhaseClock, advance_boundary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let path = arguments
        .first()
        .filter(|argument| !argument.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "/run/terminal-salem/game.sock".into());
    let preset = option_value(&arguments, "--preset")
        .map(parse_preset)
        .transpose()?
        .unwrap_or(Preset::Demo5);
    let scenario = arguments.iter().any(|argument| argument == "--scenario");
    let fast = arguments.iter().any(|argument| argument == "--fast");
    let allow_same_uid = arguments
        .iter()
        .any(|argument| argument == "--allow-same-uid");
    let config = if scenario {
        GameConfig::scenario(preset.roles().to_vec())
    } else {
        GameConfig::ordinary(preset)
    };
    let mut game = Game::from_os_random(config)?;
    let limits = TransportLimits {
        require_unique_uid: !allow_same_uid,
        reject_server_uid: !allow_same_uid,
        ..TransportLimits::default()
    };
    let mut transport = ServerTransport::bind(&path, limits)?;
    let durations = if fast {
        Durations::fast()
    } else {
        Durations::default()
    };
    let mut peers = HashMap::<PeerId, PlayerId>::new();
    let mut player_peers = HashMap::<PlayerId, PeerId>::new();
    let mut sequences = HashMap::<PlayerId, u64>::new();
    let mut clock = PhaseClock::new(durations);
    let server_started = Instant::now();
    let mut rate_limiter = RateLimiter::default();
    let mut request_tracker = RequestTracker::default();
    println!(
        "Terminal Salem {} listening on {path}; waiting for {} players",
        preset.label(),
        game.config.expected_players()
    );

    loop {
        clock.sync(game.phase, Instant::now());
        if game.outcome != MatchOutcome::InProgress {
            deliver_events(
                &transport,
                &player_peers,
                &mut sequences,
                game.phase.id,
                vec![game.final_event()],
            );
            std::thread::sleep(Duration::from_millis(100));
            return Ok(());
        }
        if clock.expired(Instant::now()) {
            let events = advance_boundary(&mut game)?;
            deliver_events(
                &transport,
                &player_peers,
                &mut sequences,
                game.phase.id,
                events,
            );
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
                    let joined = vec![
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
                                text: "accepted".into(),
                            },
                        );
                        if output.quit {
                            transport.disconnect(peer, "requested by player");
                        } else if game.can_start() {
                            let events = game.start()?;
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
                    } else {
                        game.mark_disconnected(player)?;
                    }
                }
            }
        }
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

fn option_value<'a>(arguments: &'a [String], option: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find(|pair| pair[0] == option)
        .map(|pair| pair[1].as_str())
}

fn parse_preset(text: &str) -> Result<Preset, String> {
    match text {
        "demo5" | "5" => Ok(Preset::Demo5),
        "standard7" | "7" => Ok(Preset::Standard7),
        "standard8" | "8" => Ok(Preset::Standard8),
        "full9" | "9" => Ok(Preset::Full9),
        _ => Err(format!(
            "unknown preset {text:?}; use demo5, standard7, standard8, or full9"
        )),
    }
}
