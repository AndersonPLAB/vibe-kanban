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

Successful output ends with the line that matters:

```
preflight: migrations LF ✓ | libclang ✓ | bash ✓
smoke: frontend 200 | backend :3003 200 | proxied /api/health ok
==> OPEN: http://localhost:3002  (backend :3003)
```

**Read the port off that line.** It is not stable: observed 3000/3002 and 3002/3003 across
runs on the same machine.

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

`dev_assets/` is gitignored and disposable — it is re-seeded from `dev_assets_seed/` on
start, so deleting it is a safe reset for DB/config weirdness.
