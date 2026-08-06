---
name: run-vibe-kanban
description: Run, start, launch, build, smoke-test or screenshot the vibe-kanban dev server (React + Vite frontend, Rust/axum backend) on Windows. Use when asked to run the app, bring up the dev server, verify a change in the real app, or get the dev URL.
---

# Run vibe-kanban (Windows)

Vibe Kanban is a Vite/React frontend plus a Rust (axum + sqlx/SQLite) backend, both
started by one `pnpm run dev`. On Windows that script needs a bash shell and libclang,
and it **auto-allocates its ports** — so the URL must be read from the log, never assumed.

Drive it with the committed driver. All paths below are relative to the repo root.

```bash
node .claude/skills/run-vibe-kanban/driver.mjs
```

It runs preflight → sweeps stale dev processes → starts the dev server → waits for both
ports → smoke-tests the API through the Vite proxy → prints the URL, then stays up.
Verified working 2026-08-05 (Windows 11, Node 26.7, pnpm 10.13, Rust nightly-2025-12-04).

## Modes

| Command | What it does |
|---|---|
| `node .claude/skills/run-vibe-kanban/driver.mjs --check` | Preflight only (~instant). Catches the CRLF trap *before* a 3-minute build. |
| `node .claude/skills/run-vibe-kanban/driver.mjs --smoke` | Start, verify, tear down, exit 0/1. Use in CI-ish checks. |
| `node .claude/skills/run-vibe-kanban/driver.mjs` | Start, verify, print URL, stay up. Ctrl-C stops the whole tree. |
| `node .claude/skills/run-vibe-kanban/driver.mjs --kill` | Sweep orphaned dev processes from earlier runs. |
| `node .claude/skills/run-vibe-kanban/open.mjs` | Open `/workspaces/board` in a browser, starting the server only if it is not answering. What the desktop icon runs — see Autostart below. |

Successful output ends with the line that matters:

```
preflight: migrations LF ✓ | libclang ✓ | bash ✓
smoke: frontend 200 | backend :3003 200 | proxied /api/health ok
==> OPEN: http://localhost:3002  (backend :3003)
```

**Read the port off that line.** It is not stable: observed 3000/3002 and 3002/3003 across
runs on the same machine.

## Autostart + desktop icon

Two human entry points, one install, one start path:

```
  logon (+1 min delay)                     double-click "Vibe Kanban"
          │                                            │
  Task Scheduler: VibeKanbanDev              Desktop\Vibe Kanban.lnk (vk.ico)
          │                                            │
  wscript //nologo autostart.vbs             wscript //nologo open.vbs
          │            WScript.Shell.Run(..., 0, False) = no window, no wait
          └──────────────┬─────────────────────────────┘
                         ▼
              node open.mjs [--no-open]
                         │
          read .dev-ports.json → GET /api/health
                         │
            ┌────────────┴────────────┐
        answering                 dead / stale / no file
            │                          │
            │                 node driver.mjs (detached, stdio → vk-server.log)
            │                    sweeps orphans → pnpm run dev → waits for
            │                    both ports → smoke-tests the Vite proxy
            │                          │
            │                 poll .dev-ports.json again (the port changes)
            └────────────┬─────────────┘
                         ▼
        start "" http://localhost:<port>/workspaces/board
              (skipped entirely under --no-open)
```

The icon starts nothing when the server is already up (~120 ms to browser), and
cold-starts it when it is not (~7 s warm, minutes on a cold cargo cache) — so it
works with or without the autostart task. `--no-open` is the same path without
the browser, which is all the logon task is.

Install, and re-install after pulling changes to the driver or `open.mjs`:

```powershell
powershell -ExecutionPolicy Bypass -File .claude/skills/run-vibe-kanban/install-autostart.ps1 -RepoRoot C:\lab\vibe-kanban
```

| Path | What |
|---|---|
| `%USERPROFILE%\.vibe-kanban\launcher\` | everything the installer generates (`-InstallDir` to move it) |
| `…\launcher\autostart.vbs`, `open.vbs` | hidden launchers; the only place the repo path is baked in |
| `…\launcher\open.mjs`, `driver.mjs` | **copies**, so a deleted worktree or a branch switch cannot break the shortcut. `VK_ROOT` / `--root` points them back at the repo |
| `…\launcher\vk.ico` | 256px "VK" mark, BMP payload (see Gotchas) |
| `…\launcher\autostart.log`, `open.log`, `vk-server.log` | logon run, icon run, dev server output |
| `Desktop\Vibe Kanban.lnk` | `wscript.exe //nologo open.vbs` |
| Task `VibeKanbanDev` | at logon, +1 min, no window, `IgnoreNew`, no execution time limit |

Scheduled task over `shell:startup`: it survives startup-folder policies, can be
delayed off the boot storm, and is inspectable (`Get-ScheduledTaskInfo VibeKanbanDev`
→ `LastTaskResult 0`). Uninstall:

```powershell
Unregister-ScheduledTask -TaskName VibeKanbanDev -Confirm:$false
Remove-Item "$([Environment]::GetFolderPath('Desktop'))\Vibe Kanban.lnk", "$env:USERPROFILE\.vibe-kanban\launcher" -Recurse
```

Verified end to end 2026-08-06: `Start-ScheduledTask` → ready on :3002 in 23 s with
zero visible windows and alive after a 2-minute soak; click with the server up →
board in 120 ms, same PIDs; `--kill` + deleted `.dev-ports.json` → click → server
back on :3000 (ports drifted) and the board open 6.5 s later.

## Prerequisites

One-time, in this order. Rust comes from `rust-toolchain.toml` (nightly-2025-12-04) —
rustup switches automatically, so a "stable" install is not what gets used.

```bash
pnpm i
cargo install cargo-watch
cargo install sqlx-cli --no-default-features --features sqlite,rustls
```

LLVM is required and **not mentioned in the README** — `sqlx`'s `sqlite-preupdate-hook`
feature forces `buildtime_bindgen` in `libsqlite3-sys`, which needs `libclang.dll`. The
project's own CI installs `clang libclang-dev llvm` (`.github/workflows/pre-release.yml`).

```bash
winget install LLVM.LLVM --accept-package-agreements --accept-source-agreements --disable-interactivity
```

The winget installer does not add LLVM to PATH, so persist the location once:

```powershell
[Environment]::SetEnvironmentVariable('LIBCLANG_PATH','C:\Program Files\LLVM\bin','User')
```

The driver also passes `LIBCLANG_PATH` itself, so it works even in a shell that
predates that variable.

## Build times

First backend build ~3 min (longer on a truly cold cargo cache — it fetches git
dependencies including `codex` and a `ts-rs` fork). Warm rebuild ~28s. The driver
waits up to 900s; raise with `VK_READY_TIMEOUT_MS`.

## Run (human path)

From **Git Bash** (fails in PowerShell/cmd — see Gotchas):

```bash
pnpm run dev
```

This gives you raw interleaved output and no port summary, which is why the driver
exists. To get the port, grep the log for `Main server on :` or read `.dev-ports.json`.

## Verify visually

The driver only proves HTTP works. To confirm the UI renders, point a browser tool
(Playwright MCP `browser_navigate` + `browser_take_screenshot`, or any browser) at the
frontend URL. `/` redirects to `/onboarding`; you should see the pixel "VIBE-KANBAN"
wordmark with three columns — Coding Agent / Code Editor / Notification Sound — and a
`--dangerously-skip-permissions` warning banner. A blank frame means the backend died
after the smoke check; check the log.

## Gotchas

- **The dev script is bash, not cmd.** It starts with `export FRONTEND_PORT=$(...)`, so
  running `pnpm run dev` from PowerShell/cmd dies instantly with `'export' não é
  reconhecido` / `is not recognized`. The driver sets `npm_config_script_shell` to Git
  Bash. Repo `.npmrc` does not set it.
- **A bash script-shell is not enough — bash's own coreutils must be on PATH.**
  pnpm's `.bin` shims are `/bin/sh` scripts that call `dirname`, `sed` and `uname`.
  Launched from a Git Bash session those are inherited on PATH and everything
  works; from PowerShell, cmd or Task Scheduler they are not, `basedir` comes out
  empty, and the run dies on `Cannot find module 'C:\concurrently\dist\bin\concurrently.js'`
  — a path that looks like a corrupt install and is really a missing `dirname`.
  The driver now prepends `C:\Program Files\Git\usr\bin` to PATH itself.
- **Vite dies if the dev script inherits a scheduled task's stdin.** An autostart
  that ran `driver.mjs` straight from `cmd` under `wscript` came up, smoke-tested
  green, then lost vite ~25 s later (`ELIFECYCLE Command failed with exit code 1`)
  while the Rust server stayed orphaned. Both entry points now go through
  `open.mjs`, which spawns the driver detached with `stdio: ['ignore', log, log]`.
- **Ports are auto-allocated and drift upward.** `scripts/setup-dev-environment.js`
  probes for free ports and writes `.dev-ports.json`. If anything still holds 3000/3001
  it silently shifts to the next free pair. Never hardcode 3000/3001.
- **Killing the dev server on Windows leaves orphans.** Ctrl-C / killing the top process
  leaves `cargo-watch`, `cargo` and `vite` alive, still holding ports — which is what
  makes ports drift. Three generations (15 processes) accumulated in one session before
  this was caught. The driver kills by tree (`taskkill /T /F`) and sweeps leftovers
  before every start; `--kill` does it on demand.
- **CRLF silently breaks all DB migrations.** With global `core.autocrlf=true` and no
  `.gitattributes` in this repo, the migration `.sql` files check out as CRLF. sqlx
  checksums migration file *bytes* (SHA-384), so every checksum mismatches and the
  backend exits with `Migrate(VersionMismatch(20250617183714))` — an error that points
  at the database, not at line endings. `--check` catches it in milliseconds.
- **`Recommended executor: CLAUDE_CODE` in the log is informational**, not an error.
- **A fresh dev env has zero registered repos, and a workspace without a repo is
  structurally broken.** The seeded `dev_assets` DB registers no repository, and
  attaching one is what materialises the git worktree (`container_ref`). Until a
  workspace has ≥1 repo, `git/status`, the diff WS and agent dispatch all fail with
  `Container(Other(Workspace has no repositories configured))` — a 500 the UI shows as
  the generic "An internal error occurred". Register a repo before exercising any
  workspace flow:

  ```bash
  curl -s -X POST -H "Origin: http://localhost:$FRONTEND_PORT" -H "Content-Type: application/json" \
    -d '{"path":"/abs/path/to/repo","display_name":"my-repo"}' \
    http://localhost:$BACKEND_PORT/api/repos
  ```

  `target_branch` must name a branch that **exists** — `""` is rejected with
  `Branch '' does not exist in repository`. Resolve it from the repo's
  `default_target_branch`, else the `is_current` entry of
  `GET /api/repos/{id}/branches`.

## Troubleshooting

Errors actually hit on this machine, and what fixed them:

| Symptom | Fix |
|---|---|
| `'export' não é reconhecido como um comando interno` | Ran under cmd.exe. Use the driver, or run from Git Bash. |
| `Unable to find libclang: "couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll']"` | Install LLVM (above) and set `LIBCLANG_PATH`. |
| `Error: Deployment(Sqlx(Migrate(VersionMismatch(20250617183714))))` | CRLF migrations. `git config core.autocrlf false && git rm --cached -rq . && git reset --hard`, then confirm with `--check`. |
| `server.exe (exit code: 0xc0000142, STATUS_DLL_INIT_FAILED)` | An orphaned `cargo-watch` rebuilt and relaunched the server while its files were changing. Run `--kill` first. |
| Driver hangs, no `==> OPEN` line | Something never printed a port. Read the task log directly; a cold cargo build genuinely takes minutes before any port appears. |
| `Failed to parse config: unknown variant 'light'` | Harmless. Seed `config.json` uses a lowercase theme the v7 schema rejects; the app falls back to defaults and continues. |
| Tailwind `content option ... is missing or empty` warning | Harmless; styles render correctly. |
| UI shows `An internal error occurred` on first prompt; log says `Container(Other(Workspace has no repositories configured))` | The workspace has no repo, so no worktree. Attach one: `POST /api/workspaces/{id}/repos` with `{"repo_id","target_branch"}` (a real branch name). |
| `Branch '' does not exist in repository '<name>'` | `target_branch` was empty. Use the repo's `default_target_branch`, or the `is_current` branch from `GET /api/repos/{id}/branches`. |
| Icon looks blank in Explorer, or `[System.Drawing.Icon]::ToBitmap()` throws `Intervalo solicitado ultrapassa o fim da matriz` | PNG-compressed `.ico` payload. Legal since Vista, but System.Drawing and some shell extensions cannot decode it — the installer writes a 32bpp BMP payload instead. |
| Task runs, `LastTaskResult 0`, nothing starts | Read `launcher\autostart.log`: the task only launches `wscript`, so every real error lands there, not in Task Scheduler. |
| `cargo install sqlx-cli` fails: `requires rustc 1.94.0 or newer` | Pin to the version matching the project's sqlx (0.8.6): `cargo install sqlx-cli@0.8.6 --no-default-features --features sqlite,rustls`. |

`dev_assets/` is gitignored and disposable — it is re-seeded from `dev_assets_seed/` on
start, so deleting it is a safe reset for DB/config weirdness.
