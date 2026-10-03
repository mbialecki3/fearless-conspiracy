use std::env;
use std::io::{self, BufRead};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use terminal_salem::comm::{ClientMessage, ClientTransport, ServerMessage, TransportLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/run/terminal-salem/game.sock".into());
    let name = env::args()
        .nth(2)
        .ok_or("usage: salem-client [SOCKET] NAME")?;
    let client = ClientTransport::connect(path, name, TransportLimits::default())?;
    println!("[IPC] {:?}", client.welcome());

    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if line_tx.send(line).is_err() {
                return;
            }
        }
    });

    let mut request_id = 1_u64;
    let mut phase_id = 0_u64;
    loop {
        let message = match client.recv_timeout(Duration::from_millis(50)) {
            Ok(message) => message,
            Err(error) => {
                println!("[IPC CLOSED] {error}");
                return Ok(());
            }
        };
        if let Some(message) = message {
            match message {
                ServerMessage::Event {
                    phase_id: event_phase,
                    channel,
                    text,
                    ..
                } => {
                    phase_id = event_phase;
                    println!("[{}] {text}", channel.to_uppercase());
                }
                ServerMessage::Reply { accepted, text, .. } => {
                    println!("[{}] {text}", if accepted { "OK" } else { "REJECTED" });
                }
                ServerMessage::Closing { reason } => {
                    println!("[CLOSED] {reason}");
                    return Ok(());
                }
                other => println!("[SYSTEM] {other:?}"),
            }
        }
        match line_rx.try_recv() {
            Ok(Ok(text)) => {
                let quitting = text.trim() == "/quit";
                client.send(ClientMessage::Command {
                    request_id,
                    phase_id,
                    text,
                })?;
                request_id += 1;
                if quitting {
                    thread::sleep(Duration::from_millis(150));
                    return Ok(());
                }
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(mpsc::TryRecvError::Disconnected) => {
                client.close();
                return Ok(());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}
