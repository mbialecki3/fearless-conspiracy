# Terminal Salem

Terminal Salem is a text-based social deduction game for five to nine players.
One terminal runs the server and each player uses a separate client terminal.

## Install

On Ubuntu:

```bash
sudo apt update
sudo apt install --yes build-essential curl pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
cd Project1_CSE536
cargo build
```

## Run the game

Start the server in one terminal:

```bash
./server
```

Start one client in each of five other terminals:

```bash
./client "Player 10"
./client "Player 11"
./client "Player 12"
./client "Player 13"
./client "Player 14"
```

Enter `/ready` in every client. The game starts when everyone is ready.

Running `./client` without a name asks for one. The server and clients use
`/tmp/terminal-salem.sock` automatically.

For predictable roles and manual phase control, use:

```bash
./server --scenario --manual
```

In the server terminal, use `status` to show the phase, `advance` to move to the
next phase, and `shutdown` to stop the server.

## How to play

- **Town** wins by eliminating every Mafia player.
- **Mafia** wins by deceiving and eliminating the Town.
- **Jester** wins by convincing the players to execute the Jester.

Players receive secret roles. At night, players use their role abilities. At
dawn, results and deaths are announced. During the day, players discuss their
evidence and nominate someone for trial. The accused gives a defense, then the
other living players vote guilty, innocent, or abstain.

Players are targeted by their displayed game IDs, such as `#1` or `#4`, not by
their client names.

### Common commands

```text
/help                    Show available commands.
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
