//! All interprocess transport for Terminal Salem.
//!
//! Unix domain stream sockets carry bytes, not messages. This module gives the
//! byte stream application-level boundaries: a four-byte, big-endian length
//! followed by one UTF-8 JSON document. No socket operations belong in the game
//! rules modules.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Read, Write};

/// The largest accepted JSON payload. The four-byte header is not included.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug)]
pub enum CommError {
    Io(io::Error),
    EmptyFrame,
    FrameTooLarge { declared: usize, maximum: usize },
    InvalidJson(serde_json::Error),
}

impl fmt::Display for CommError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "socket I/O failed: {error}"),
            Self::EmptyFrame => write!(f, "empty frames are not valid messages"),
            Self::FrameTooLarge { declared, maximum } => {
                write!(f, "frame declares {declared} bytes; maximum is {maximum}")
            }
            Self::InvalidJson(error) => write!(f, "invalid JSON message: {error}"),
        }
    }
}

impl std::error::Error for CommError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidJson(error) => Some(error),
            Self::EmptyFrame | Self::FrameTooLarge { .. } => None,
        }
    }
}

impl From<io::Error> for CommError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for CommError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidJson(value)
    }
}

/// First-stage protocol messages sent by a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        version: u16,
        name: String,
    },
    Command {
        request_id: u64,
        phase_id: u64,
        text: String,
    },
    Close,
}

/// First-stage protocol messages sent by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        version: u16,
        player_id: u8,
        text: String,
    },
    Reply {
        request_id: u64,
        accepted: bool,
        text: String,
    },
    Event {
        sequence: u64,
        channel: String,
        text: String,
    },
    Error {
        text: String,
    },
    Closing {
        reason: String,
    },
}

/// Serialize and write one complete frame.
///
/// `write_all` is essential here: one `write` call is allowed to accept only
/// part of either the header or body on a stream socket.
pub fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), CommError>
where
    W: Write,
    T: Serialize,
{
    let body = serde_json::to_vec(message)?;
    if body.is_empty() {
        return Err(CommError::EmptyFrame);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared: body.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }

    let length = u32::try_from(body.len()).expect("64 KiB always fits in u32");
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Read and deserialize exactly one frame.
///
/// The declared length is checked before allocating the body buffer. Repeated
/// calls can safely read frames that arrived together because `read_exact`
/// consumes only the current header and body.
pub fn read_message<R, T>(reader: &mut R) -> Result<T, CommError>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let declared = u32::from_be_bytes(header) as usize;
    if declared == 0 {
        return Err(CommError::EmptyFrame);
    }
    if declared > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared,
            maximum: MAX_FRAME_BYTES,
        });
    }

    let mut body = vec![0_u8; declared];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::net::UnixStream;
    use std::thread;

    struct SmallChunks<R> {
        inner: R,
        limit: usize,
    }

    impl<R: Read> Read for SmallChunks<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.limit);
            self.inner.read(&mut buffer[..count])
        }
    }

    #[test]
    fn frame_round_trip_survives_split_header_and_body_reads() {
        let original = ClientMessage::Command {
            request_id: 19,
            phase_id: 4,
            text: "/investigate 3".into(),
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &original).unwrap();

        let mut split = SmallChunks {
            inner: Cursor::new(bytes),
            limit: 1,
        };
        let decoded: ClientMessage = read_message(&mut split).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn multiple_frames_can_arrive_in_one_stream_read() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &ClientMessage::Close).unwrap();
        write_message(
            &mut bytes,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Mira".into(),
            },
        )
        .unwrap();

        let mut cursor = Cursor::new(bytes);
        let first: ClientMessage = read_message(&mut cursor).unwrap();
        assert_eq!(first, ClientMessage::Close);
        let second: ClientMessage = read_message(&mut cursor).unwrap();
        assert_eq!(
            second,
            ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Mira".into()
            }
        );
    }

    #[test]
    fn oversized_length_is_rejected_before_body_read() {
        let declared = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(declared)).unwrap_err();
        assert!(matches!(error, CommError::FrameTooLarge { .. }));
    }

    #[test]
    fn invalid_json_is_rejected() {
        let mut bytes = Vec::from(8_u32.to_be_bytes());
        bytes.extend_from_slice(b"not-json");
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(bytes)).unwrap_err();
        assert!(matches!(error, CommError::InvalidJson(_)));
    }

    #[test]
    fn eof_mid_frame_is_reported() {
        let mut bytes = Vec::from(20_u32.to_be_bytes());
        bytes.extend_from_slice(b"short");
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(bytes)).unwrap_err();
        assert!(
            matches!(error, CommError::Io(ref io) if io.kind() == io::ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn real_unix_stream_pair_exchanges_a_framed_request_and_response() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let server_thread = thread::spawn(move || {
            let request: ClientMessage = read_message(&mut server).unwrap();
            assert!(matches!(request, ClientMessage::Hello { .. }));
            write_message(
                &mut server,
                &ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    player_id: 1,
                    text: "connected over AF_UNIX/SOCK_STREAM".into(),
                },
            )
            .unwrap();
        });

        write_message(
            &mut client,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Rowan".into(),
            },
        )
        .unwrap();
        let reply: ServerMessage = read_message(&mut client).unwrap();
        assert!(matches!(reply, ServerMessage::Welcome { player_id: 1, .. }));
        server_thread.join().unwrap();
    }
}
