use std::env;
use std::io::{self, BufRead};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use terminal_salem::comm::{ClientMessage, ClientTransport, TransportLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let path = arguments
        .next()
        .unwrap_or_else(|| "/run/terminal-salem/game.sock".into());
    let name = arguments.next().unwrap_or_else(|| "Player".into());
    let client = ClientTransport::connect(path, name, TransportLimits::default())?;
    println!("{:?}", client.welcome());
    println!("Enter a demonstration command; EOF exits.");

    let (line_tx, line_rx) = mpsc::channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            if line_tx.send(line).is_err() {
                return;
            }
        }
    });

    let mut request_id = 1_u64;
    loop {
        if let Some(message) = client.recv_timeout(Duration::from_millis(50))? {
            println!("{message:?}");
        }
        match line_rx.try_recv() {
            Ok(Ok(text)) => {
                let quitting = text == "/quit";
                client.send(ClientMessage::Command {
                    request_id,
                    phase_id: 0,
                    text,
                })?;
                request_id += 1;
                if quitting {
                    thread::sleep(Duration::from_millis(100));
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
