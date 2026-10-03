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

#[derive(Clone, Copy)]
struct Durations {
    role_reveal: Duration,
    day_one: Duration,
    night: Duration,
    dawn: Duration,
    discussion: Duration,
    nominations: Duration,
    defense: Duration,
    judgment: Duration,
    last_words: Duration,
}

impl Durations {
    fn standard() -> Self {
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

    fn fast() -> Self {
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

    fn for_phase(self, phase: PhaseKind) -> Option<Duration> {
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
        Durations::standard()
    };
    let mut peers = HashMap::<PeerId, PlayerId>::new();
    let mut player_peers = HashMap::<PlayerId, PeerId>::new();
    let mut sequences = HashMap::<PlayerId, u64>::new();
    let mut observed_phase = game.phase.id;
    let mut deadline = None;
    let server_started = Instant::now();
    let mut rate_limiter = RateLimiter::default();
    let mut request_tracker = RequestTracker::default();
    println!(
        "Terminal Salem {} listening on {path}; waiting for {} players",
        preset.label(),
        game.config.expected_players()
    );

    loop {
        if game.phase.id != observed_phase {
            observed_phase = game.phase.id;
            deadline = durations
                .for_phase(game.phase.kind)
                .map(|duration| Instant::now() + duration);
        }
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
        if deadline.is_some_and(|time| Instant::now() >= time) {
            let mut events = game.apply_pending_forfeits();
            if game.outcome == MatchOutcome::InProgress {
                let phase_id = game.phase.id;
                match game.phase.kind {
                    PhaseKind::Night => events.extend(game.resolve_night()?.events),
                    PhaseKind::Judgment => events.extend(game.resolve_judgment(phase_id)?.1),
                    _ => events.extend(game.advance_phase(phase_id)?),
                }
            }
            deliver_events(
                &transport,
                &player_peers,
                &mut sequences,
                game.phase.id,
                events,
            );
            continue;
        }

        let wait = deadline
            .and_then(|time| time.checked_duration_since(Instant::now()))
            .unwrap_or(Duration::from_millis(100))
            .min(Duration::from_millis(100));
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
