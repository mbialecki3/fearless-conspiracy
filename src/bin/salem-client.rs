use std::env;
use std::os::unix::net::UnixStream;
use terminal_salem::comm::{
    ClientMessage, PROTOCOL_VERSION, ServerMessage, read_message, write_message,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/terminal-salem-demo.sock".into());
    let name = env::args().nth(2).unwrap_or_else(|| "Player".into());

    let mut stream = UnixStream::connect(&path)?;
    write_message(
        &mut stream,
        &ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            name,
        },
    )?;
    let reply: ServerMessage = read_message(&mut stream)?;
    println!("{reply:?}");
    Ok(())
}
