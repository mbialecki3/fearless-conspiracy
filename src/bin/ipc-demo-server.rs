use std::collections::HashMap;
use std::env;
use std::time::Duration;
use terminal_salem::comm::{
    ClientMessage, PeerId, ServerEvent, ServerMessage, ServerTransport, TransportLimits,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let path = arguments
        .next()
        .unwrap_or_else(|| "/run/terminal-salem/game.sock".into());
    let options: Vec<_> = arguments.collect();
    let allow_same_uid = options
        .iter()
        .any(|argument| argument == "--allow-same-uid");
    let exit_after = options
        .windows(2)
        .find(|pair| pair[0] == "--exit-after")
        .and_then(|pair| pair[1].parse::<usize>().ok());
    let limits = TransportLimits {
        reject_server_uid: !allow_same_uid,
        require_unique_uid: !allow_same_uid,
        ..TransportLimits::default()
    };
    let mut transport = ServerTransport::bind(&path, limits)?;
    let mut names = HashMap::new();
    let mut total_connected = 0_usize;
    let mut sequence = 1_u64;
    println!("IPC demonstration listening on {path}");
    println!("commands: /private ID TEXT, /group ID,ID TEXT, /broadcast TEXT, /quit");

    loop {
        let Some(event) = transport.recv_timeout(Duration::from_millis(250))? else {
            continue;
        };
        match event {
            ServerEvent::Connected {
                peer,
                credentials,
                name,
            } => {
                println!(
                    "peer {} joined as {name:?} (uid={}, gid={}, pid={})",
                    peer.0, credentials.uid, credentials.gid, credentials.pid
                );
                total_connected += 1;
                names.insert(peer, name.clone());
                transport.send_to(
                    peer,
                    ServerMessage::Event {
                        sequence,
                        channel: "private".into(),
                        text: format!("Your authenticated transport peer ID is {}.", peer.0),
                    },
                )?;
                sequence += 1;
                transport.broadcast(ServerMessage::Event {
                    sequence,
                    channel: "public".into(),
                    text: format!("{name} joined as peer {}.", peer.0),
                });
                sequence += 1;
            }
            ServerEvent::Message {
                peer,
                message:
                    ClientMessage::Command {
                        request_id, text, ..
                    },
            } => {
                let result = route_command(&transport, &names, peer, &text, sequence);
                sequence += 1;
                let (accepted, response) = match result {
                    Ok(()) => (true, "message queued".to_owned()),
                    Err(error) => (false, error),
                };
                let _ = transport.send_to(
                    peer,
                    ServerMessage::Reply {
                        request_id,
                        accepted,
                        text: response,
                    },
                );
            }
            ServerEvent::Message { peer, .. } => {
                transport.disconnect(peer, "only command messages are accepted after hello");
            }
            ServerEvent::Disconnected { peer, reason } => {
                let name = names.remove(&peer).unwrap_or_else(|| "unknown".into());
                println!("peer {} ({name}) disconnected: {reason}", peer.0);
                transport.broadcast(ServerMessage::Event {
                    sequence,
                    channel: "public".into(),
                    text: format!("{name} disconnected."),
                });
                sequence += 1;
                if exit_after
                    .is_some_and(|expected| total_connected >= expected && names.is_empty())
                {
                    println!("all {total_connected} demonstration clients disconnected");
                    return Ok(());
                }
            }
        }
    }
}

fn route_command(
    transport: &ServerTransport,
    names: &HashMap<PeerId, String>,
    sender: PeerId,
    text: &str,
    sequence: u64,
) -> Result<(), String> {
    if text == "/quit" {
        transport.disconnect(sender, "requested by client");
        return Ok(());
    }
    if let Some(body) = text.strip_prefix("/broadcast ") {
        transport.broadcast(event(sequence, "public", sender, body));
        return Ok(());
    }
    if let Some(arguments) = text.strip_prefix("/private ") {
        let (target, body) = split_target(arguments)?;
        let target = parse_peer(target, names)?;
        transport
            .send_to(target, event(sequence, "private", sender, body))
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    if let Some(arguments) = text.strip_prefix("/group ") {
        let (targets, body) = split_target(arguments)?;
        let recipients = targets
            .split(',')
            .map(|target| parse_peer(target, names))
            .collect::<Result<Vec<_>, _>>()?;
        let report = transport.send_group(recipients, event(sequence, "group", sender, body));
        if report.failed.is_empty() {
            return Ok(());
        }
        return Err(format!("failed recipients: {:?}", report.failed));
    }
    Err("use /private ID TEXT, /group ID,ID TEXT, /broadcast TEXT, or /quit".into())
}

fn split_target(arguments: &str) -> Result<(&str, &str), String> {
    arguments
        .split_once(' ')
        .filter(|(target, body)| !target.is_empty() && !body.is_empty())
        .ok_or_else(|| "a target list and message body are required".into())
}

fn parse_peer(target: &str, names: &HashMap<PeerId, String>) -> Result<PeerId, String> {
    let number = target
        .parse::<u64>()
        .map_err(|_| format!("{target:?} is not a numeric peer ID"))?;
    let peer = PeerId(number);
    names
        .contains_key(&peer)
        .then_some(peer)
        .ok_or_else(|| format!("peer {number} is not connected"))
}

fn event(sequence: u64, channel: &str, sender: PeerId, body: &str) -> ServerMessage {
    ServerMessage::Event {
        sequence,
        channel: channel.into(),
        text: format!("peer {}: {body}", sender.0),
    }
}
