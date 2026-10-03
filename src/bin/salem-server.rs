use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use terminal_salem::comm::{
    ClientMessage, PROTOCOL_VERSION, ServerMessage, read_message, write_message,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/terminal-salem-demo.sock".into());
    let path = Path::new(&path);
    if path.exists() {
        return Err(format!(
            "{} already exists; verify and remove a stale socket yourself",
            path.display()
        )
        .into());
    }

    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    println!("listening on {} (AF_UNIX/SOCK_STREAM)", path.display());

    let result = serve_one(&listener);
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        eprintln!("warning: could not remove {}: {error}", path.display());
    }
    result
}

fn serve_one(listener: &UnixListener) -> Result<(), Box<dyn std::error::Error>> {
    let (mut stream, _) = listener.accept()?;
    let request: ClientMessage = read_message(&mut stream)?;
    match request {
        ClientMessage::Hello { version, name } if version == PROTOCOL_VERSION => {
            write_message(
                &mut stream,
                &ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    player_id: 1,
                    text: format!("Welcome, {name}. Framed IPC is working."),
                },
            )?;
        }
        ClientMessage::Hello { version, .. } => {
            write_message(
                &mut stream,
                &ServerMessage::Error {
                    text: format!(
                        "protocol version {version} is incompatible with {PROTOCOL_VERSION}"
                    ),
                },
            )?;
        }
        _ => {
            write_message(
                &mut stream,
                &ServerMessage::Error {
                    text: "the first message must be hello".into(),
                },
            )?;
        }
    }
    Ok(())
}
