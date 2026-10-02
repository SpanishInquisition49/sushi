<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/horizontal-reversed.svg">
    <img src="assets/logo/horizontal.svg" alt="Sushi" height="96">
  </picture>
</p>

# Sushi

An animated pet that keeps an eye on your coding agents inspired by [coucou](https://github.com/Louis-CFM/coucou). It runs as a **desktop app on macOS, Windows
and Linux** (any desktop environment) and as a plugin for the [Noctalia](https://docs.noctalia.dev) shell (v5).
It watches **Claude Code**, **Codex CLI**, **GitHub Copilot CLI**, **opencode** and **pi**, side by side.

- **Live**: every session, step by step. What the agent reads, edits and runs, as a rail of steps
  (✓ done, ◌ running, ✗ failed) and a viewer with the **diff of the file being edited** (line numbers,
  red/green lines, syntax colors, typed out as it happens), the terminal of a command, or the file it read.
  When a turn finishes the pet does a happy little jump.
- **Approve from the notch**: permission requests pop up with a preview of what the agent wants to do and
  **Allow / Deny** buttons. One click, back to work. With Claude Code, when it **asks you a question**
  (`AskUserQuestion`) its options show up as buttons and your choice goes straight back; a
  **plan** to approve (`ExitPlanMode`) is shown for reading and approved in the terminal, the only
  place that dialog exists.
- **Ask anything**: a built-in chat tab that runs one of your agents headless with no tools (Claude Code, Copilot, pi or
  Codex, your choice in the plugin settings).
- **Usage** (Claude Code only): plan limits (5-hour and weekly), context window trend, tokens today.
- **A pet with a personality**: 23 characters (nigiri, maki, ramen, bao, dango, sake…), breathing, blinking,
  eyes that glance at whatever you hover, 30+ emotes and idle quirks, a nap after a while, a greeting
  on launch and 28 little sounds.

## Supported agents

| | Claude Code | Codex CLI | GitHub Copilot | opencode | pi |
|---|:-:|:-:|:-:|:-:|:-:|
| Sessions, steps, diffs, command output | ✓ | ✓ | ✓ | ✓ | ✓ |
| Allow / Deny from the notch | ✓ | ✓ | ✓ | ✓ | opt-in (`SUSHI_PI_APPROVE=1`) |
| Questions and plans | ✓ | – | – | – | – |
| Context size and tokens | ✓ | – | – | – | – |
| Plan limits (5 h / weekly) | ✓ | – | – | – | – |
| Built-in chat | ✓ | ✓ (read-only sandbox, not tool-free) | ✓ | – | ✓ |
| Connected through | hooks in `~/.claude/settings.json` | hooks in `~/.codex/hooks.json` | hook file `~/.copilot/hooks/sushi.json` | plugin in `~/.config/opencode/plugins/` | extension in `~/.pi/agent/extensions/` |

What has been checked against the real thing:

- **Claude Code**: the reference.
- **GitHub Copilot CLI** (1.0.89): the hook payloads in `src/agent/copilot.rs` were recorded from the real CLI;
  Allow and Deny from the notch were run end to end (the allowed command ran, the denied one did not), and so
  was the chat. Copilot gives no id for tool calls, so a step is matched to its result by tool name.
- **pi**: the extension follows the docs shipped with pi 0.85 and was exercised against a stand-in for pi's API,
  not inside pi. The chat parser follows pi's JSON mode docs (the model configured here was offline).
- **Codex** (hooks and `codex exec --json` as documented; the `apply_patch` shape is a best reading of the docs)
  and **opencode** (plugin API as documented) have only been tested against their documented payloads.

Expect to adjust a field or two if a version differs: please open an issue with what it sent.

## How it works

```
Claude Code / Codex / Copilot ──hook JSON──▶ sushi-hook --agent <id> ─┐
opencode / pi ──plugin (integrations/*.ts) spawns the hook──┤
                                                            ▼ unix socket
                                                  sushi daemon (Rust): one adapter per agent
                                                            │ state.json (runtime dir)
              Noctalia plugin (Luau): bar widget + panel ◀──┤
              Desktop app (Tauri): pet window + panel ◀─────┘  (asks the daemon over the same socket)
                       Allow / Deny ──▶ sushi approve|deny ──▶ answers the hook (or the plugin)
```

Each agent has an adapter (`src/agent/`) that turns its events and tool calls into one neutral shape
(a session, a step, a diff...) and turns your Allow / Deny back into what the agent expects.

The hook never blocks an agent: if the daemon is not running it exits at once. Permission requests
and questions wait for your answer (30 s by default, `SUSHI_PERMISSION_TIMEOUT_SECS`), then the agent
falls back to its own prompt. That prompt is on screen the whole time, so you can always answer in
the terminal instead; if you do, the card in the notch disappears by itself.

## Install

### Requirements

- Linux, macOS or Windows 10+ for the daemon, the hook and `sushi install` (they use a Unix domain
  socket, which Windows 10 supports). The Noctalia plugin additionally needs Linux with
  [Noctalia](https://docs.noctalia.dev) v5 (plugin API 21) running.
- Where things live: Linux uses `$XDG_RUNTIME_DIR`; macOS `$TMPDIR/sushi-<uid>`; Windows
  `%LOCALAPPDATA%\sushi\run`. Config and cache follow `XDG_*`, `%APPDATA%` and `%LOCALAPPDATA%`.
- Plan limits: `curl` (built into Windows 10+ and macOS). On macOS the Claude login is read from the Keychain.
- Autostart the daemon: `contrib/sushi.service` (systemd), `contrib/io.sushi.daemon.plist` (macOS
  launchd), or on Windows `schtasks /Create /SC ONLOGON /TN Sushi /TR "%USERPROFILE%\.cargo\bin\sushi.exe daemon"`.
- At least one of: [Claude Code](https://claude.com/claude-code), [Codex CLI](https://github.com/openai/codex),
  [GitHub Copilot CLI](https://github.com/features/copilot/cli), [opencode](https://opencode.ai), [pi](https://pi.dev)
- Rust (`cargo`) to build the tool; `curl` (plan limits) and `systemd` (optional, for the user service)

### 1. The tool (`sushi` + `sushi-hook`)

```sh
git clone <repo-url> sushi && cd sushi
cargo install --path .             # installs sushi and sushi-hook into ~/.cargo/bin
```

Make sure `~/.cargo/bin` is in your `PATH`. Then connect the agents you use (without `--write` it only
shows what it would do; whatever it replaces is backed up as `<file>.bak-sushi-<time>`):

```sh
sushi install --agent claude --write     # hooks in ~/.claude/settings.json
sushi install --agent codex --write      # hooks in ~/.codex/hooks.json
sushi install --agent copilot --write    # hook file ~/.copilot/hooks/sushi.json
sushi install --agent opencode --write   # plugin in ~/.config/opencode/plugins/sushi.ts
sushi install --agent pi --write         # extension in ~/.pi/agent/extensions/sushi.ts
sushi install --agent all --write        # all of the above
```

(`sushi install-hooks --write` still works and means `--agent claude`.) It is safe to run again.
Agents read their hooks and plugins when a session starts: sessions that are already open need a restart.

pi has no permission prompt of its own, so Sushi only watches it; to allow or deny every tool call
from the notch, start pi with `SUSHI_PI_APPROVE=1`.

### 2. The daemon

Try it in a terminal first:

```sh
sushi daemon
```

or run it at login as a user service (recommended):

```sh
mkdir -p ~/.config/systemd/user && cp contrib/sushi.service ~/.config/systemd/user/
systemctl --user enable --now sushi.service
systemctl --user status sushi.service      # check that it is running
```

The service expects the binary in `~/.cargo/bin/sushi`; edit `ExecStart` if you installed it elsewhere.

### 3. The desktop app (macOS, Windows, Linux)

`app/` is a [Tauri](https://tauri.app) app: a small always-on-top **pet window** (click it to open the
**panel**, drag it to move it, right click to pet it) and a **tray icon** to show or hide things and quit. It is
the same pet and panel as the Noctalia plugin (Live, Chat, Usage, Allow / Deny, sounds), plus a Settings tab.
It starts the daemon for you if none is running, so steps 1 and 2 are all it needs.

```sh
cargo run -p sushi-app --release        # try it
cargo install tauri-cli --locked        # once, to make an installer
cd app/src-tauri && cargo tauri build   # .app / .dmg, .msi / .exe, .deb / .AppImage
```

- **Linux** needs the web view libraries (Debian/Ubuntu: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libgtk-3-dev`;
  Arch: `webkit2gtk-4.1 libayatana-appindicator`). The always-on-top pet window and a transparent background need
  X11 or a compositor that allows them; on GNOME / Wayland the window behaves like a normal one, and the tray
  icon needs the AppIndicator extension.
- **macOS** hides the Dock icon (it is a menu bar app). Run `tools/make_icon.py` and `cargo tauri icon` for an `.icns`.
- **Windows** 10 or later (the daemon uses Unix sockets, which Windows supports from build 1803).
- The app finds the `sushi` binary next to itself or in your `PATH`, so keep `cargo install --path .` from step 1.
- Settings (character, sounds, fidgets, nap delay, what the pet window shows, chat agent and model) are in the
  panel's gear tab and stored in `sushi/app.json` under your config folder.
- To try the interface without Tauri or a daemon, serve `app/ui` (`python3 -m http.server -d app/ui`) and open
  `/index.html?view=panel&mock=work` (also `view=pet`, `mock=ask|idle|down`, `tab=chat|usage|settings`).

### 4. The Noctalia plugin (Linux only)

The plugin lives in `plugin/`. Link it where Noctalia looks for local plugins and enable it:

```sh
mkdir -p ~/.local/share/noctalia-plugin-dev
ln -s "$PWD/plugin" ~/.local/share/noctalia-plugin-dev/sushi
noctalia msg plugins enable scanna/sushi
```

Then add the widget to a bar (the center group of the bar is the notch):

```sh
noctalia msg settings-open-widget <bar-name>    # add "Sushi" (scanna/sushi:pet)
```

If the plugin does not find the tool, set the path to the `sushi` binary in the plugin's settings
(default `~/.cargo/bin/sushi`). Start a session of any connected agent and the pet shows up with it.

After editing the plugin's code, reload it: `noctalia msg plugins disable scanna/sushi`
then `enable` (Noctalia caches the scripts).

## Using it

| Where | What |
|---|---|
| Bar widget (Noctalia) / pet window (app) | the pet, the turn timer, files changed, the 5-hour plan usage; the tooltip has the full progress of the active session. Left click opens the panel, right click pets it. |
| Panel · Live | steps on the left, viewer on the right. Click a step to look at it, click a session to pin it. |
| Panel · Chat | type and press Enter. Stop and "new conversation" buttons at the bottom. |
| Panel · Usage | limits, context trend, tokens. |
| Keybinding | `noctalia msg plugin scanna/sushi:state all tab chat` then `noctalia msg panel-open scanna/sushi:panel` opens the panel on a tab (`live`, `chat`, `usage`). |

## Settings

In Noctalia's plugin settings: **character**, idle quirks, nap delay, **sounds**, widget details,
close the panel after Allow / Deny, **chat agent** and chat model, and the path to the `sushi` binary.

The daemon reads an optional `~/.config/sushi/config.json`:

```json
{
  "show_code": true,
  "context_window": 200000,
  "context_windows": { "claude-sonnet-5": 1000000 },
  "chat_agent": "claude",
  "chat_model": "sonnet",
  "chat_models": { "copilot": "auto" },
  "claude_path": "claude",
  "agent_paths": { "pi": "/opt/pi/bin/pi" }
}
```

`chat_agent` is the agent the chat talks to (`claude`, `copilot`, `pi` or `codex`; the plugin setting wins),
`chat_model` is the model for Claude Code and `chat_models` the one per other agent (empty: the agent's default).
`agent_paths` says where an executable is when it is not on the `PATH`. A conversation is with one agent:
switching starts a new one.

Set `SUSHI_NO_LIMITS=1` to stop the daemon from asking for your plan limits.

## Privacy and costs

- **Code in the viewer**: diffs, command output and file excerpts are published in
  `$XDG_RUNTIME_DIR/sushi/state.json`, readable only by you and cleared on logout. Set
  `"show_code": false` to publish titles only.
- **Plan limits** use the OAuth token Claude Code stores in `~/.claude/.credentials.json`, sent only to
  `api.anthropic.com` (through `curl`, never on a command line). The endpoint is undocumented and may change.
- **The chat** runs the chosen agent headless with no tools (Claude Code: `--tools ""` and no MCP servers;
  Copilot: no tools available; pi: `--no-tools` and no extensions), using that agent's own login. It counts
  against its plan like any other session (a Copilot chat message is one premium request) and keeps its
  history in `~/.cache/sushi/`. Codex has no tool-free mode: its chat runs in a read-only sandbox, so it could
  still read files.

## Sounds

The 28 sounds in `plugin/sounds/` are synthesized by `tools/make_sounds.py` (standard library only).
Run it again to regenerate them; `--check` only verifies them.

## Development

```sh
tests/run.sh      # everything below, in one go
```

It runs `cargo test` (unit tests, plus end-to-end tests that start a real daemon and the real hook in a
throwaway directory and walk through Allow, Deny, questions, plans and killed hooks), `cargo clippy`, the
syntax and `noctalia plugins lint` checks of the Luau scripts, the Luau tests in `tests/lua/` (they run
the plugin's scripts against stand-ins for Noctalia's `ui`, `noctalia` and `panel`, so `luajit` is
needed) and the sound check.

The Luau code is written to run in Noctalia's plugin runtime; the UI is plain flex boxes (no canvas),
so every character is a tree of rounded boxes moved with computed spacers.
