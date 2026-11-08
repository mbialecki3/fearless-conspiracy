# Fearless Conspiracy

Mitchell Bialecki - CSE 536, Project 1

A text-based game written in Rust for 3-9 players, inspired by Town of Salem.
Town tries to eliminate the Mafia, Mafia tries to eliminate the Town, and
the Jester tries to get voted out.

**The communications library is contained in one file: [`src/comm.rs`](src/comm.rs).**

## Setup and build

Use Linux (or Ubuntu under WSL), with everyone on the same machine.
From the project folder on Ubuntu, run:

```bash
./install
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

Run as your normal user. The installer uses `sudo` for system packages,
installs Rust, and builds the game. It needs internet access. If Rustup and
a C compiler/linker are already installed, use `cargo build --locked --bins`.
`rust-toolchain.toml` selects Rust 1.95.0.

## Run

Each player needs a separate non-root Linux account, different from the host.
For protected play, use a dedicated Ubuntu/systemd VM with cgroup v2.
Build as your normal user, then launch:

```bash
cargo build --locked --bins
sudo ./server --protected --players 5
```

Choose 3-9 players (default: 5). Startup creates `fearless_host` and
`fearless_p1` through `fearless_pN` with private groups. Read initial player
passwords using `sudo cat /var/lib/fearless-conspiracy/player-logins.txt` and
share each player's entry privately. Accounts persist and are reused without
resetting passwords; unrelated existing accounts are never modified. To use
existing accounts, pass `--server-user ACCOUNT` and a `--player ACCOUNT` for
each participant; these accounts need private primary groups and no sudo access.
Protected startup enables
VM-wide `hidepid=2`, limits each player's user slice to 25% of one CPU, 256 MiB
memory and 64 tasks, and blocks other players' cgroup statistics. It checks
isolation before starting the server without root or capabilities. These
restrictions remain until reboot; run clients from their normal login sessions.

Answer the setup questions; press Enter to accept a default. Choose manual
phases if you want the host to advance the game. In a separate terminal under
each player's account, run:

```bash
/run/fearless-conspiracy/fearless-conspiracy-client /run/fearless-conspiracy/game.sock "Alice"
```

Use different names and enter `/ready` in every client to start.
The protected launcher installs the client at the path above. For local play without OS restrictions, use
`./server --strict-users --shared-local` and `./client "Alice"`. To test several
terminals on one account, use `./server`; that mode allows the same UID.

Host commands: `status`, `advance` (manual mode), `abort`, and `shutdown`.

## Play

Everyone receives a secret role with its own abilities and goal. Use `/role`
to check your role and `/help ROLE` for its instructions. Players can tell the
truth or lie about their roles, so compare claims with night results and votes.
Use `/players` to find target IDs: `/investigate 2`, for example, targets #2.

### Goals

- **Town:** Eliminate every Mafia player while at least one Town player survives.
- **Mafia:** Eliminate every Town player while at least one Mafia player survives.
  Having equal numbers of Town and Mafia does not immediately end the match.
- **Jester (Neutral):** Convince the other players to publicly execute you.
  Being killed at night does not count. Your personal win is separate from
  the final Town/Mafia result.

### Phases

The game announces each phase and tells you which actions are available.
In automatic mode, timers move the game along. In manual mode, the host uses
`advance`; reaching the nomination threshold still starts a trial immediately.

1. **Lobby:** Join and enter `/ready`. The match starts when the selected
   number of players are connected and everyone is ready.
2. **Role Reveal:** Receive your role privately and read its help. Mafia
   members also learn their teammates' roles; the moderator can see all roles.
3. **Day 1 Introductions:** Talk using `/say MESSAGE` or plain text, introduce
   yourself, and make claims. There are no nominations or trials on Day 1;
   the game goes to the first night afterward.
4. **Night:** Choose your role's action, such as investigating, healing, or
   attacking. You can change your choice or use `/cancel` before night ends.
   Actions resolve together at the end of the phase, and public chat is closed.
5. **Dawn:** Read the public death announcements and your private night results.
   Deaths reveal roles and saved wills. Public chat stays closed until Discussion.
6. **Discussion:** Share evidence, question claims, and decide whom you suspect.
   Use public chat or `/whisper ID MESSAGE`; nominations open in the next phase.
7. **Nominations:** Use `/vote ID` to nominate a living player, change your
   target with another vote, or withdraw with `/unvote`. More than half of all
   living players must nominate the same person: with five alive, three votes
   are needed. You cannot nominate yourself. If the phase ends without enough
   votes for anyone, the game goes to night.
8. **Defense:** The nominated player goes on trial. Only that player can speak
   publicly, using `/say MESSAGE` or plain text to explain their actions and
   defend themselves.
9. **Judgment:** Living players other than the accused choose `/guilty`,
   `/innocent`, or `/abstain`. Not voting counts as abstaining. Choices are
   revealed when voting ends. Guilty must outnumber innocent to convict;
   a tie acquits. After acquittal, nominations resume with the remaining time,
   up to three trials that day; after the third acquittal, night begins.
10. **Last Words and Execution:** A convicted player gets one final chance
    to speak publicly, then is executed. Their role and will are published.
    Night begins afterward unless the match has ended.
11. **Postgame:** Once a win condition is met, the server reveals the final
    roles and results. Everyone can chat, including dead players, but gameplay
    actions are finished. Use `/quit` when you are ready to leave.

After the first night, the usual cycle is Dawn, Discussion, Nominations,
any trial, and Night again.

### Roles and actions

The player count determines which roles are included. Most abilities are used
at night; the Jailor chooses a prisoner during the day.

| Role | Command | What it does |
|---|---|---|
| Sheriff | `/investigate ID` | Check whether a player appears suspicious. A nonsuspicious result does not guarantee they are Town. |
| Doctor | `/heal ID` or `/heal self` | Protect a player from an ordinary attack. You can heal yourself once per game. |
| Lookout | `/watch ID` | Watch another player and learn up to three visitors to them. This does not tell you whether the watched player left home. |
| Escort | `/block ID` | Block a player's night ability. The Escort is immune to ordinary roleblocks. |
| Jailor | `/jail ID`, `/execute`, `/spare` | Choose a prisoner during the day, question them at night, and decide whether to execute. Executions are unavailable on Night 1. |
| Godfather | `/kill ID` | Choose the Mafia target; an available Mafioso normally carries out the attack. The Godfather appears innocent to the Sheriff. |
| Mafioso | `/kill ID` | Perform the Mafia attack. A valid Godfather order takes priority over your own target. |
| Framer | `/frame ID` | Make a non-Mafia player appear suspicious to the Sheriff. |
| Jester | `/haunt ID` | After being publicly executed, haunt an eligible living guilty or abstaining voter the following night. |

Protection, roleblocks, and framing can change the outcome of another player's
action. A protected player can survive an attack, and a framed Town player can
look suspicious. Read your private results and use `/help ROLE` for full details.

### Chat, wills, and leaving

Use `/say MESSAGE` or plain text for public chat during Introductions,
Discussion, and Nominations. During Defense and Last Words, only the accused
may speak publicly. Whispers are available during the ordinary daytime chat
phases; their contents are private, but everyone sees that a whisper happened.
At night, Mafia can use `/mafia MESSAGE`, and the Jailor and prisoner can use
`/jailchat MESSAGE`. Dead players can talk to one another with `/dead MESSAGE`
but cannot vote or take ordinary role actions.

Save notes with `/will set TEXT` and read them with `/will show`. Your will is
published when you die, so it can preserve investigation results or mislead
other players. You can use `/quit` at any time, but leaving during a match
counts as forfeiting rather than pausing your place in the game.

## Communications API

**File: `src/comm.rs`, imported as `fearless_conspiracy::comm`.**
It uses Unix-domain stream sockets, length-prefixed JSON messages, and
reader/writer threads. Group and public messages are sent to each recipient.

| Method | Purpose |
|---|---|
| `ServerTransport::bind(path, limits)` | Start listening. |
| `server.recv_timeout(duration)` | Receive a connection, message, or disconnection event. |
| `server.send_to(peer, message)` | Queue a message for one peer. |
| `server.send_group(peers, message)` | Queue a message for selected peers. |
| `server.broadcast(message)` | Queue a message for everyone. |
| `server.connected_peers()` | List peer IDs. |
| `server.disconnect(peer, reason)` | Close a connection. |
| `server.release_uid(uid)` | Free a lobby account reservation. |
| `server.shutdown_gracefully(reason, duration)` | Drain output and close connections. |
| `ClientTransport::connect(path, name, limits)` | Connect and complete the handshake. |
| `client.welcome()` | Read the handshake response. |
| `client.send(message)` | Queue a command. |
| `client.recv_timeout(duration)` | Receive a server message. |
| `client.close()` | Disconnect. |
| `write_message(writer, message)` | Write one framed JSON message. |
| `read_message(reader)` | Read one framed JSON message. |

The API uses `ClientMessage`, `ServerMessage`, `ServerEvent`, and `PeerId`.
`TransportLimits` configures queues, timeouts, permissions, and UID checks.
Errors use `CommError`; receive methods return `None` on timeout. Group sends
return `DeliveryReport`; shutdown returns `ShutdownReport`. Queued output
does not mean the recipient has read it.

## Files

| File | Purpose |
|---|---|
| **`src/comm.rs`** | **Entire communications library.** |
| `src/lib.rs` | Exports the modules. |
| `src/game.rs` | Game state, phases, voting, chat, and wins. |
| `src/roles.rs` | Roles, presets, and night actions. |
| `src/command.rs` | Player commands and help. |
| `src/rate_limit.rs` | Command limits and duplicate-request checks. |
| `src/server.rs` | Setup, host commands, and timers. |
| `src/bin/fearless-conspiracy-server.rs` | Moderator program. |
| `src/bin/fearless-conspiracy-client.rs` | Player program. |
| `src/bin/ipc-demo-server.rs`, `src/bin/ipc-demo-client.rs` | Examples of private, group, and broadcast delivery. |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | Dependencies and Rust version. |
| `install`, `server`, `client` | Setup and launch scripts. |
| `scripts/check.sh`, `scripts/smoke-game.sh` | Code checks and gameplay test. |
| `scripts/preflight-multi-user.sh` | Checks an optional group-based account setup. |
| `scripts/package-release.sh`, `scripts/package-submission.sh` | Package binaries or the class submission. |
| `scripts/build-report.py` | Generates the report PDF. |
| `docs/`, `ScreenshotsForPDF/`, `output/pdf/` | Report text, checklist, screenshots, and PDF. |

Tests live alongside the Rust code. Run them with `cargo test --locked`.
