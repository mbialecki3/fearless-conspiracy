use std::env;
use std::time::Duration;
use terminal_salem::comm::{ClientTransport, TransportLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/terminal-salem-demo.sock".into());
    let name = env::args().nth(2).unwrap_or_else(|| "Player".into());

    let limits = TransportLimits {
        reject_server_uid: false,
        require_unique_uid: false,
        ..TransportLimits::default()
    };
    let client = ClientTransport::connect(&path, name, limits)?;
    println!("{:?}", client.welcome());
    if let Some(message) = client.recv_timeout(Duration::from_secs(2))? {
        println!("{message:?}");
    }
    Ok(())
}
