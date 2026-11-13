# Fearless Conspiracy

A text-based Rust game for 3-9 players, inspired by Town of Salem. Town tries to
find the Mafia, the Mafia tries to eliminate Town, and the Jester wants to get
executed. Talk, make claims, and work out who's lying.

## Setup

Use a dedicated Ubuntu VM with systemd and cgroup v2. Everyone plays on the same
Linux system. Cgroup v2 is built into Linux, so there's no separate package for it.

Download and extract the [prebuilt Linux x86-64 release](https://github.com/mbialecki3/fearless-conspiracy/releases/latest).
It includes the binaries, so you don't need Rust. Install the supporting tools:

```bash
sudo apt-get update
sudo apt-get install --yes python3 acl util-linux passwd
```

If you cloned the source instead, run `./install` as your normal user. It installs
the dependencies and Rust, then builds the game. Follow its printed PATH command
if needed.

## Start a game

Protected mode is on by default:

```bash
sudo ./server --players 5
```

Choose 3-9 players; the default is five. Answer the setup questions, keeping
predictable roles off for a normal game. Manual mode lets the host advance phases.

Startup creates `fearless_host` and `fearless_p1` through `fearless_pN`.
Read the player passwords in another moderator terminal:

```bash
sudo cat /var/lib/fearless-conspiracy/player-logins.txt
```

Give each player only their own login. Each player logs into their assigned
account normally and runs:

```bash
/run/fearless-conspiracy/fearless-conspiracy-client /run/fearless-conspiracy/game.sock "Alice"
```

The lobby waits until everyone connects and enters `/ready`. Keep the moderator
console private because it shows the roles.

For testing several terminals on your own account without the protections:

```bash
./server --unprotected --players 5
./client "Alice" --plain
```

Run more clients in separate terminals. Plain `./server` without sudo shows an
error instead of silently switching modes.

## Playing

Day 1 is introductions, then the first night. At night, choose your role's action.
Actions resolve together. During the day, discuss the results and nominate a
suspect. More than half the living players must nominate the same person.

The accused gets a defense before everyone else votes. Guilty must outnumber
innocent to convict; ties acquit. Executions include last words, a role reveal,
and the player's saved will. The game tells you which phase you're in.

Town wins when all Mafia are gone. Mafia wins when all Town are gone; equal
numbers alone don't end the game. The Jester wins by being publicly executed.

## Helpful commands

| Command | Use |
|---|---|
| `/ready` | Ready up in the lobby. |
| `/role`, `/help ROLE` | Check your role and its abilities. Try `/help Sheriff`. |
| `/players` | Find target IDs. `/investigate 2` targets player #2. |
| `/say MESSAGE` or plain text | Talk publicly when the phase allows it. |
| `/whisper ID MESSAGE` | Private daytime message. Others see that a whisper happened. |
| `/vote ID`, `/unvote` | Nominate someone or withdraw your nomination. |
| `/guilty`, `/innocent`, `/abstain` | Vote during judgment. The accused can't vote. |
| `/will set TEXT`, `/will show` | Save or read your will. It's revealed when you die. |
| `/cancel` | Clear your night choice. |
| `/mafia`, `/jailchat`, `/dead` followed by a message | Use the chat available to your role or state. |
| `/quit` | Leave. Leaving mid-game counts as forfeiting. |

Role actions include `/investigate`, `/heal`, `/watch`, `/block`, `/kill`, and
`/frame`, followed by an ID. The Jailor uses `/jail ID`, `/execute`, and `/spare`.
An executed Jester can use `/haunt ID`. Check `/help ROLE` for the details.

Host commands: `status`, `advance` in manual mode, `abort`, and `shutdown`.

## Security and cleanup

- Separate player accounts and limited user slices keep players inside their
  assigned permissions and resource limits.
- `hidepid=2` hides other accounts' process/thread metadata. It blocks the tested
  leak where thread activity exposed Mafia-chat recipients. Other players'
  cgroup statistics are restricted too.
- Each player gets 25% of one CPU, a 192 MiB memory soft limit, a 256 MiB hard
  limit, no swap, and 64 tasks.
- Sudo is used for setup and cleanup. The game server runs without root.
  Startup stops if a required protection can't be verified.
- Shutdown, Ctrl+C, and handled termination signals restore the previous settings
  and remove temporary player accounts. Existing accounts passed with `--player`
  are kept. The server account and staged binaries stay for reuse.

After a forced kill, the next protected launch tries to recover. To recover
without starting a game:

```bash
sudo ./server --restore
```

## Code and checks

The entire communications library is **[`src/comm.rs`](src/comm.rs)**. It uses local
Unix stream sockets, length-prefixed JSON, and reader/writer threads. Its API has
`bind`/`connect`, `send`/`recv_timeout`, `send_to`, `send_group`, `broadcast`,
`close`, and `shutdown_gracefully`.

Rules are in `src/game.rs` and `src/roles.rs`, commands in `src/command.rs`, host
setup/timers in `src/server.rs`, and programs in `src/bin/`.
`scripts/protected-server.py` handles isolation and cleanup. The Cargo files
cover dependencies and the Rust toolchain.

Run `bash scripts/check.sh` for checks or `bash scripts/smoke-game.sh` for a quick
game. Use `python3 scripts/run-proc-visibility-vm.py --help` for the VM security
tests. `scripts/package-release.sh VERSION` builds the binary release package.
