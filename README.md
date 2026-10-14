# Fearless Conspiracy

Fearless Conspiracy is a text-based social-deduction game for three to nine
players. One terminal runs the moderator server, and every player runs a client
process from a separate terminal.

## Install

### Download binaries

Download **fearless-conspiracy-0.1.0-linux-x86_64.tar.gz** from the
[GitHub release](https://github.com/mbialecki3/fearless-conspiracy/releases/latest).
This package runs on 64-bit x86 Linux, including Ubuntu under WSL, without
installing Rust or compiling anything.

Extract the download and start the moderator in a terminal:

```bash
tar -xzf fearless-conspiracy-0.1.0-linux-x86_64.tar.gz
cd fearless-conspiracy-0.1.0-linux-x86_64
./server
```

In a separate terminal for each player, enter the same extracted folder and run:

```bash
./client "Alice"
```

Use a different name for each player, then enter `/ready` in every client.
All players must run on the same Linux system (or the same WSL distribution/VM),
because the game uses local Unix-domain sockets. It does not connect different
computers over the network. For multiple Linux accounts, use `./server --shared-local`.

### Build from source on Ubuntu

After downloading or cloning the project, enter its folder and run:

```bash
cd Project1_CSE536
./install
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

Run `./install` as your normal user. It uses `sudo` to install Ubuntu packages
(`build-essential`, `ca-certificates`, `curl`, and `pkg-config`), installs Rustup
if needed, downloads the project's Rust toolchain, and builds the game. Internet
access is required. Existing Rustup installations are reused.

The `export` command makes Rust available in your current terminal. For a new
installation, Rustup also configures future shells. You can rerun `./install`
if installation is interrupted.

### Build from source on other Linux distributions

1. Use your distribution's package manager to install a C compiler and linker
   (GCC or Clang), Make, `curl`, `pkg-config`, and CA certificates. On Ubuntu,
   you can install these manually with:

   ```bash
   sudo apt update
   sudo apt install --yes build-essential ca-certificates curl pkg-config
   ```

2. Install Rust using the [official Rustup installer](https://rust-lang.org/tools/install/)
   if you do not already have Rustup:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
   export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
   ```

3. Enter the project folder, select the toolchain, and build:

   ```bash
   cd Project1_CSE536
   rustup show
   cargo build --locked --bins
   ```

The checked-in `rust-toolchain.toml` selects Rust 1.95.0, Rustfmt, and Clippy;
Rustup installs missing components for the project. See the
[Rustup toolchain documentation](https://rust-lang.github.io/rustup/overrides.html#the-toolchain-file)
for how this works.

The game requires Linux because it uses Linux Unix-socket peer credentials.
On Windows, use Ubuntu under WSL; on macOS, use a Linux VM. Then follow the
instructions above inside that Linux environment.

## Run

Start the server in one terminal:

```bash
./server
```

Every launch of `./server` asks how many players you want (3–9), whether the host
should advance phases manually, whether roles should be predictable, which
automatic timer speed to use, and whether other local Linux accounts can
connect. Press Enter to explicitly accept a displayed default, or type your
choice. It shows the role lineup before opening the lobby.
Predictable roles follow player ID order; otherwise the lineup is shuffled.

Setup always runs, even when you supply options or redirect input. The server
waits for answers before opening the lobby; if input closes before setup is
complete, it exits without starting a game. Command-line options only supply
the displayed defaults for the questions; they do not skip setup. `--setup`
is still accepted for compatibility but is no longer needed.

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

You can optionally suggest answers with game options. For example, this shows
six players, predictable roles, and manual phase control as the setup defaults:

```bash
./server --players 6 --scenario --manual
```

The questions still appear, and you must answer or press Enter for each one.

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
four players give more room for discussion. Run `./server` and select either
count in the setup questions.
