use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, ExternalPrinter as _};
use std::env;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;
use terminal_salem::comm::{ClientMessage, ClientTransport, ServerMessage, TransportLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let positional: Vec<_> = arguments
        .iter()
        .filter(|argument| !argument.starts_with("--"))
        .collect();
    let path = positional
        .first()
        .map(|argument| argument.as_str())
        .unwrap_or("/run/terminal-salem/game.sock");
    let name = positional
        .get(1)
        .ok_or("usage: salem-client [SOCKET] NAME [--plain]")?;
    let client = ClientTransport::connect(path, name.as_str(), TransportLimits::default())?;
    println!("[IPC] {:?}", client.welcome());
    if arguments.iter().any(|argument| argument == "--plain") {
        run_plain(client)
    } else {
        run_prompt_safe(client)
    }
}

fn run_plain(client: ClientTransport) -> Result<(), Box<dyn std::error::Error>> {
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
        match client.recv_timeout(Duration::from_millis(50)) {
            Ok(Some(message)) => {
                if let Some(new_phase) = message_phase(&message) {
                    phase_id = new_phase;
                }
                println!("{}", render_message(message));
            }
            Ok(None) => {}
            Err(error) => {
                println!("[IPC CLOSED] {error}");
                return Ok(());
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

enum PromptCommand {
    Line(String),
    Close,
}

fn run_prompt_safe(client: ClientTransport) -> Result<(), Box<dyn std::error::Error>> {
    let mut editor = DefaultEditor::new()?;
    let mut printer = editor.create_external_printer()?;
    let phase_id = Arc::new(AtomicU64::new(0));
    let network_phase = Arc::clone(&phase_id);
    let (command_tx, command_rx) = mpsc::channel();
    let network = thread::spawn(move || {
        let mut request_id = 1_u64;
        loop {
            match command_rx.try_recv() {
                Ok(PromptCommand::Line(text)) => {
                    if client
                        .send(ClientMessage::Command {
                            request_id,
                            phase_id: network_phase.load(Ordering::Acquire),
                            text,
                        })
                        .is_err()
                    {
                        return;
                    }
                    request_id += 1;
                }
                Ok(PromptCommand::Close) | Err(mpsc::TryRecvError::Disconnected) => {
                    client.close();
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            match client.recv_timeout(Duration::from_millis(50)) {
                Ok(Some(message)) => {
                    if let Some(new_phase) = message_phase(&message) {
                        network_phase.store(new_phase, Ordering::Release);
                    }
                    let closing = matches!(message, ServerMessage::Closing { .. });
                    if printer.print(render_message(message)).is_err() || closing {
                        return;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = printer.print(format!("[IPC CLOSED] {error}"));
                    return;
                }
            }
        }
    });

    loop {
        match editor.readline("> ") {
            Ok(line) => {
                editor.add_history_entry(line.as_str())?;
                let quitting = line.trim() == "/quit";
                command_tx.send(PromptCommand::Line(line))?;
                if quitting {
                    thread::sleep(Duration::from_millis(200));
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => {
                let _ = command_tx.send(PromptCommand::Close);
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    drop(command_tx);
    let _ = network.join();
    Ok(())
}

fn message_phase(message: &ServerMessage) -> Option<u64> {
    match message {
        ServerMessage::Event { phase_id, .. } => Some(*phase_id),
        _ => None,
    }
}

fn render_message(message: ServerMessage) -> String {
    match message {
        ServerMessage::Event { channel, text, .. } => {
            format!("[{}] {text}", channel.to_uppercase())
        }
        ServerMessage::Reply { accepted, text, .. } => {
            format!("[{}] {text}", if accepted { "OK" } else { "REJECTED" })
        }
        ServerMessage::Closing { reason } => format!("[CLOSED] {reason}"),
        other => format!("[SYSTEM] {other:?}"),
    }
}
