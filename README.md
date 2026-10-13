# Fearless Conspiracy

Fearless Conspiracy is a text-based social-deduction game for three to nine
players. One terminal runs the moderator server, and every player runs a client
process from a separate terminal.

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

The launcher asks how many players you want (3–9), whether the host should
advance phases manually, whether roles should be predictable, which automatic
timer speed to use, and whether other local Linux accounts can connect. Press
Enter to accept each default. It shows the role lineup before opening the lobby.
Predictable roles follow player ID order; otherwise the lineup is shuffled.

To use the questions with command-line defaults, run `./server --setup` with
any additional options. For example, `./server --setup --shared-local` defaults
to allowing connections from other local accounts.

For multi user setup, log each player into a different Linux user
account and start the server with shared local socket access:

```bash
./server --shared-local
```

Start one client from each players terminal using

```bash
./client "Name"
```

Enter `/ready` in every client. The game starts when the selected number of
players are connected and everyone is ready.

To skip the questions, supply game options. For example, for six players with
predictable roles and manual phase control, use:

```bash
./server --players 6 --scenario --manual
```

Existing scenario and preset commands still work. `./server --players 5` starts
with the previous defaults. With redirected input, bare `./server` also keeps
the previous defaults and skips setup; use `--setup` to explicitly read answers.

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

Choose any player count from 3 through 9 with `--players NUMBER`. The equivalent
presets are `demo3`, `standard4`, `demo5`, `standard6`, `standard7`, `standard8`,
and `full9`. The default remains five players.

| Players | Preset | Roles |
|---|---|---|
| 3 | `demo3` | Sheriff, Doctor, Mafioso |
| 4 | `standard4` | Sheriff, Doctor, Lookout, Mafioso |
| 6 | `standard6` | Sheriff, Doctor, Lookout, Escort, Mafioso, Jester |

Three-player games are quick demos where one death can decide the match;
four players give more room for discussion. For example, run `./server --players 3`
or `./server --players 4`, or select either count in the interactive setup.
