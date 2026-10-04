# Build and install Sushi's parts: `just` lists the recipes.
#   tool     sushi (CLI + daemon) and sushi-hook
#   app      the Tauri desktop app (pet window, panel, tray)
#   plugin   the Noctalia plugin (Linux)

set shell := ["sh", "-cu"]

noctalia_dir := env_var_or_default("XDG_DATA_HOME", env_var("HOME") / ".local/share") / "noctalia-plugin-dev"

# List the recipes
default:
    @just --list --unsorted

# ── Tool ─────────────────────────────────────────────────────────────────────

# Build sushi and sushi-hook (release)
build:
    cargo build --release --bin sushi --bin sushi-hook

# Install sushi and sushi-hook into ~/.cargo/bin
install:
    cargo install --path . --force

# Connect an agent: claude, codex, copilot, antigravity, gemini, opencode, pi or all (`just connect claude no` only shows it)
connect agent="claude" write="yes":
    sushi install --agent {{agent}} {{ if write == "yes" { "--write" } else { "" } }}

# Restart the daemon (the systemd user service if enabled, else a background process)
restart:
    #!/bin/sh
    # Also stop a daemon started by hand: it would keep the socket and the new one could not start.
    if systemctl --user is-enabled sushi.service >/dev/null 2>&1; then
        systemctl --user stop sushi.service
        pkill -f '(^|/)sushi daemon$' || true
        sleep 0.3
        systemctl --user reset-failed sushi.service 2>/dev/null || true
        systemctl --user start sushi.service && echo "sushi.service restarted"
    else
        pkill -f '(^|/)sushi daemon$' || true
        sleep 0.3
        nohup sushi daemon >/tmp/sushi-daemon.log 2>&1 &
        echo "daemon started, log in /tmp/sushi-daemon.log"
    fi

# Install the tool and restart the daemon
update: install restart

# ── Desktop app ──────────────────────────────────────────────────────────────

# Build the desktop app (target/release/sushi-app)
app:
    cargo build --release -p sushi-app

# Build and start the desktop app
app-run:
    cargo run --release -p sushi-app

# Make the installers (.deb / .AppImage, .dmg, .msi): needs `cargo install tauri-cli --locked`
bundle:
    cd app/src-tauri && cargo tauri build

# Cross-compile the app + CLI .exe for Windows (not an installer — see `bundle-windows`)
app-windows:
    # Needs: rustup target add x86_64-pc-windows-gnu, and mingw-w64-gcc (pacman -S mingw-w64-gcc
    # / apt install mingw-w64).
    cargo build --release --target x86_64-pc-windows-gnu -p sushi-app
    cargo build --release --target x86_64-pc-windows-gnu --bin sushi --bin sushi-hook
    @echo "→ target/x86_64-pc-windows-gnu/release/{sushi-app,sushi,sushi-hook}.exe"
    @echo "  copy all three into the same folder on Windows and run sushi-app.exe"

# Cross-compile a real Windows installer (NSIS .exe) from here, CLI included
bundle-windows: app-windows
    # Needs everything app-windows does, plus `nsis` (`yay -S nsis` on Arch, `apt install nsis`
    # on Debian/Ubuntu): Tauri only auto-downloads its own NSIS toolchain when it is itself
    # running on Windows, so cross-compiling needs a system `makensis` on PATH instead.
    mkdir -p app/src-tauri/windows-bin
    cp target/x86_64-pc-windows-gnu/release/sushi.exe target/x86_64-pc-windows-gnu/release/sushi-hook.exe app/src-tauri/windows-bin/
    cd app/src-tauri && cargo tauri build --target x86_64-pc-windows-gnu --bundles nsis
    @echo "→ target/x86_64-pc-windows-gnu/release/bundle/nsis/*-setup.exe"

# Serve the interface with sample data, no Tauri or daemon (open /index.html?view=panel&mock=work)
ui-mock port="8000":
    python3 -m http.server {{port}} -d app/ui

# ── Noctalia plugin ──────────────────────────────────────────────────────────

# Link the plugin where Noctalia looks for local plugins and enable it
plugin-link:
    mkdir -p "{{noctalia_dir}}"
    ln -sfn "{{justfile_directory()}}/plugin" "{{noctalia_dir}}/sushi"
    noctalia msg plugins enable scanna/sushi

# Reload the plugin after editing it (Noctalia caches the scripts)
plugin-reload:
    noctalia msg plugins disable scanna/sushi
    noctalia msg plugins enable scanna/sushi

# ── Everything ───────────────────────────────────────────────────────────────

# Install the tool, restart the daemon and build the app
all: update app

# Tests, clippy, Luau, app scripts, icons and sounds
test:
    tests/run.sh

# Regenerate the icons and the sounds
assets:
    python3 tools/make_icon.py
    python3 tools/make_sounds.py

# Remove build products
clean:
    cargo clean
