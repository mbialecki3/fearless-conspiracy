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

## Run the game

The default is a timed, randomized Demo 5 match on the protected runtime
socket:

```sh
sudo -u salem_host salem-server /run/terminal-salem/game.sock --preset demo5
sudo -u salem_p1 salem-client /run/terminal-salem/game.sock Alice
```

Start one client per distinct player account, then enter `/ready` in each.
Available presets are `demo5`, `standard7`, `standard8`, and `full9`. Add
`--scenario` to assign roles in the public preset order for a visibly labeled
test/screenshot run. `--fast --allow-same-uid` is only for automated local
smoke tests; it shortens phase timers and disables the separate-UID check.

The client attaches the latest server-issued phase ID to every state-changing
command. The server rejects stale commands, routes hidden events only to their
authorized sockets, and advances authoritative timers without waiting for idle
players. Type `/help` for the command summary.

All nine planned roles are implemented: Sheriff, Doctor, Lookout, Tavern
Keeper, Jailor, Godfather, Mafioso, Framer, and Jester. Night resolution follows
the frozen-snapshot pipeline in `plan.md`, including jail access, block
immunity, one Mafia attack, persistent frames, simultaneous lethal effects,
Jester haunt, and post-death-batch promotion. `cargo test` contains deterministic
cases for every worked scenario and role interaction.

Interactive clients use `rustyline`'s external printer so an incoming socket
event does not destroy a partially typed command. Pass `--plain` for redirected
input and automated transcripts.

## Source map and verification

- `src/comm.rs`: every socket operation, JSON framing, peer credentials,
  connection threads, deadlines, bounded queues, and fan-out.
- `src/game.rs`: authoritative match/player/phase state, chat permissions,
  trials, wills, forfeits, victory, and personal outcomes.
- `src/roles.rs`: role attributes, intentions, visits, attacks, deterministic
  night resolution, and promotions.
- `src/command.rs`: terminal command parsing and dispatch into the pure engine.
- `src/rate_limit.rs`: rolling command/chat limits and replay protection.
- `src/bin/salem-server.rs` and `salem-client.rs`: process wiring and terminal UI.

Run `scripts/check.sh` for the complete compiler/test gate and
`scripts/smoke-game.sh` for a five-client end-to-end process test. The detailed
coverage map is in [`docs/testing.md`](docs/testing.md). Peer feedback and the
PDF still require real classmates and screenshots; templates are provided in
[`docs/peer-feedback-template.md`](docs/peer-feedback-template.md) and
[`docs/report-outline.md`](docs/report-outline.md) without fabricated evidence.

Runtime account and socket-directory setup will be documented before the
multi-user integration milestone. Never put account passwords in this
repository.

## Milestones

1. Framed request/response across a Unix stream.
2. Concurrent authenticated clients with private, group, and broadcast delivery.
3. Complete five-player game (Sheriff, Doctor, Lookout, Mafioso, Jester).
4. Full nine-role ruleset and deterministic night-resolution scenarios.
5. Linux multi-user, privacy, failure, packaging, and report verification.
