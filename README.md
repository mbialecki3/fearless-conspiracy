# Fearless Conspiracy

Fearless Conspiracy is a text-based social-deduction game for five to nine
players. One terminal runs the moderator server, and every player runs a client
process from a separate terminal.

> **Disclaimer:** Fearless Conspiracy is an independent student project based on
> and inspired by social-deduction games such as Mafia, Town of Salem, and
> Werewolf. It is not affiliated with, endorsed by, or associated with the
> creators, publishers, or rights holders of those games.

## Install

On Ubuntu, install Rust and build the project:

```bash
sudo apt update
sudo apt install --yes build-essential curl pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
cd Project1_CSE536
cargo build
```

## Run

Start the server in one terminal:

```bash
./server
```

For the course's multi-user setup, log each player into a different Linux user
account and start the server with shared local socket access:

```bash
./server --shared-local
```

This makes the Unix socket accessible to local users on the machine. Use it
only on a trusted system. The normal `./server` command restricts access to the
server owner and group.

Start one client from each player's terminal:

```bash
./client "Player 10"
./client "Player 11"
./client "Player 12"
./client "Player 13"
./client "Player 14"
```

Enter `/ready` in every client. A five-player game starts when all five players
are connected and ready.

Running `./client` without a name asks for one. The server and clients use
`/tmp/fearless-conspiracy.sock` automatically.

If a previous server crashed and left that socket behind, the next `./server`
launch verifies that no server is listening and removes the stale socket
automatically. A running server is never replaced.

For predictable roles and manual phase control, use:

```bash
./server --scenario --manual
```

In the server terminal, use `status` to show the current game state, `advance`
to move to the next phase, `abort` to cancel the current match, and `shutdown`
to stop the server cleanly.

## How to play

- **Town** wins by eliminating every Mafia player.
- **Mafia** wins by deceiving and eliminating the Town.
- **Jester** wins by convincing the players to execute the Jester.

Players receive secret roles. At night, players privately choose role actions.
At dawn, deaths and other public results are announced. During the day, players
discuss their evidence and nominate someone for trial. The accused gives a
defense, and the other living players vote guilty, innocent, or abstain.

Players are targeted by their displayed game IDs, such as `#1` or `#4`, not by
their client names.

### Common commands

```text
/help                    Show available commands.
/help ROLE               Explain a role and its commands.
/role                    Show your role.
/players                 Show player IDs and status.
/say MESSAGE             Send a public message.
/whisper ID MESSAGE      Send a private message.
/vote ID                 Nominate a player for trial.
/unvote                  Remove your nomination.
/guilty                  Vote to execute the accused.
/innocent                Vote to acquit the accused.
/abstain                 Abstain from judgment.
/will set TEXT           Set your last will.
/cancel                  Cancel your night action.
/quit                    Leave the game.
```

Plain text is sent to the appropriate current chat. It is public during normal
daytime discussion and becomes Mafia or jail chat at night when applicable.

### Role commands

| Role | Command | Ability |
|---|---|---|
| Sheriff | `/investigate ID` | Check whether someone appears suspicious. |
| Doctor | `/heal ID` or `/heal self` | Protect someone from an attack. |
| Lookout | `/watch ID` | See who visited someone. |
| Escort | `/block ID` | Stop someone's night ability. |
| Jailor | `/jail ID`, `/execute`, `/spare` | Detain and question someone. |
| Godfather | `/kill ID` | Choose the Mafia target. |
| Mafioso | `/kill ID` | Perform the Mafia attack. |
| Framer | `/frame ID` | Make someone appear suspicious. |
| Jester | `/haunt ID` | Haunt an eligible voter after execution. |

Mafia players can use `/mafia MESSAGE`, the Jailor and prisoner can use
`/jailchat MESSAGE`, and dead players can use `/dead MESSAGE`.

For larger games, start the server with `--preset standard7`, `--preset
standard8`, or `--preset full9`.
