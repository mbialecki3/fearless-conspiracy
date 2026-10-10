use fearless_conspiracy::comm::{ClientMessage, ClientTransport, ServerMessage, TransportLimits};
use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, ExternalPrinter as _};
use std::env;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    let positional: Vec<_> = arguments
        .iter()
        .filter(|argument| !argument.starts_with("--"))
        .collect();
    let path = positional
        .first()
        .map(|argument| argument.as_str())
        .unwrap_or("/run/fearless-conspiracy/game.sock");
    let name = positional
        .get(1)
        .ok_or("usage: fearless-conspiracy-client [SOCKET] NAME [--plain]")?;
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
                if let Some(rendered) = render_message(message) {
                    println!("{rendered}");
                }
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
                if let Err(error) = client.send(ClientMessage::Command {
                    request_id,
                    phase_id,
                    text,
                }) {
                    println!("[ERROR] command was not sent: {error}");
                    return Ok(());
                }
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
                    if let Err(error) = client.send(ClientMessage::Command {
                        request_id,
                        phase_id: network_phase.load(Ordering::Acquire),
                        text,
                    }) {
                        let _ = printer.print(format!("[ERROR] command was not sent: {error}"));
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
                    if render_message(message)
                        .is_some_and(|rendered| printer.print(rendered).is_err())
                        || closing
                    {
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

fn render_message(message: ServerMessage) -> Option<String> {
    match message {
        ServerMessage::Event { channel, text, .. } => {
            Some(format!("[{}] {text}", channel.to_uppercase()))
        }
        ServerMessage::Reply {
            accepted: true,
            text,
            ..
        } if text == "accepted" => None,
        ServerMessage::Reply { accepted, text, .. } => Some(format!(
            "[{}] {text}",
            if accepted { "OK" } else { "ERROR" }
        )),
        ServerMessage::Closing { reason } => Some(format!("[CLOSED] {reason}")),
        other => Some(format!("[SYSTEM] {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routine_success_replies_are_silent() {
        assert_eq!(
            render_message(ServerMessage::Reply {
                request_id: 1,
                accepted: true,
                text: "accepted".into(),
            }),
            None
        );
    }

    #[test]
    fn useful_confirmations_and_errors_are_visible() {
        assert_eq!(
            render_message(ServerMessage::Reply {
                request_id: 1,
                accepted: true,
                text: "Ready status confirmed: ready.".into(),
            }),
            Some("[OK] Ready status confirmed: ready.".into())
        );
        assert_eq!(
            render_message(ServerMessage::Reply {
                request_id: 2,
                accepted: false,
                text: "wrong phase".into(),
            }),
            Some("[ERROR] wrong phase".into())
        );
    }
}
