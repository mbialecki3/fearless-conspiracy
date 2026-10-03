# Terminal Salem

Terminal Salem is a local-terminal social deduction game and a Linux IPC
demonstration. One server and up to nine player processes communicate only over
pathname-based Unix domain stream sockets. The socket transport, framing,
connection threads, peer credentials, and delivery queues live in
`src/comm.rs`; game rules remain independent of transport code.

The authoritative implementation plan and fixed ruleset are in [`plan.md`](plan.md).

## Development setup (Ubuntu 26.04)

Rust is intentionally installed per user with `rustup`; game accounts never
need a compiler or administrator access. On a fresh development account:

```sh
sudo apt update
sudo apt install --yes build-essential curl pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
rustup show
cargo test
```

The repository pins Rust 1.95.0 in `rust-toolchain.toml`. The first Cargo build
needs internet access to download dependencies. No dependency is currently
vendored.

## Run the Milestone 1 IPC exchange

In one terminal:

```sh
cargo run --bin salem-server -- /tmp/terminal-salem-demo.sock
```

In another terminal:

```sh
cargo run --bin salem-client -- /tmp/terminal-salem-demo.sock Rowan
```

The client connects through `AF_UNIX`/`SOCK_STREAM`, writes a four-byte
big-endian length and JSON `hello` body, then reads a response framed the same
way. The server rejects an existing pathname instead of unlinking an arbitrary
file. It removes only the socket it successfully bound after the demo ends.

Run `cargo test` to exercise framing under partial reads and other malformed
input. See [`docs/ipc-architecture.md`](docs/ipc-architecture.md) for the
transport lifecycle and responsibility boundary.

The permanent multi-client communication demonstration is provided by
`ipc-demo-server` and `ipc-demo-client`. Follow
[`docs/multi-user-demo.md`](docs/multi-user-demo.md) to exercise authenticated
private, group, and broadcast delivery with three distinct Linux users.

Runtime account and socket-directory setup will be documented before the
multi-user integration milestone. Never put account passwords in this
repository.

## Milestones

1. Framed request/response across a Unix stream.
2. Concurrent authenticated clients with private, group, and broadcast delivery.
3. Complete five-player game (Sheriff, Doctor, Lookout, Mafioso, Jester).
4. Full nine-role ruleset and deterministic night-resolution scenarios.
5. Linux multi-user, privacy, failure, packaging, and report verification.
