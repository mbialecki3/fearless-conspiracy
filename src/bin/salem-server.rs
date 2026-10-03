use std::env;
use std::time::Duration;
use terminal_salem::comm::{ServerEvent, ServerMessage, ServerTransport, TransportLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/terminal-salem-demo.sock".into());
    let limits = TransportLimits {
        reject_server_uid: false,
        require_unique_uid: false,
        ..TransportLimits::default()
    };
    let mut server = ServerTransport::bind(&path, limits)?;
    println!("listening on {path} (AF_UNIX/SOCK_STREAM)");
    if let Some(ServerEvent::Connected { peer, name, .. }) =
        server.recv_timeout(Duration::from_secs(30))?
    {
        server.send_to(
            peer,
            ServerMessage::Event {
                sequence: 1,
                channel: "private".into(),
                text: format!("Welcome, {name}. Concurrent framed IPC is working."),
            },
        )?;
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
