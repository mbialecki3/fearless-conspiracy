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

Runtime account and socket-directory setup will be documented before the
multi-user integration milestone. Never put account passwords in this
repository.

## Milestones

1. Framed request/response across a Unix stream.
2. Concurrent authenticated clients with private, group, and broadcast delivery.
3. Complete five-player game (Sheriff, Doctor, Lookout, Mafioso, Jester).
4. Full nine-role ruleset and deterministic night-resolution scenarios.
5. Linux multi-user, privacy, failure, packaging, and report verification.

