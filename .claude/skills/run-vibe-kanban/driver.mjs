#!/usr/bin/env node
// Driver for running vibe-kanban's dev server on Windows.
//
//   node .claude/skills/run-vibe-kanban/driver.mjs --check   preflight only, no build
//   node .claude/skills/run-vibe-kanban/driver.mjs --smoke   start, verify, tear down
//   node .claude/skills/run-vibe-kanban/driver.mjs           start, verify, stay up
//
// Why this exists: `pnpm run dev` needs a bash script-shell + LIBCLANG_PATH on
// Windows, and it auto-allocates ports that shift between runs — so the URL has
// to be parsed from the log, never assumed.

import { spawn, execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname, '../../..');
const BASH = 'C:\\Program Files\\Git\\usr\\bin\\bash.exe';
const LLVM_BIN = 'C:\\Program Files\\LLVM\\bin';
const MIGRATIONS = path.join(ROOT, 'crates/db/migrations');
const READY_TIMEOUT_MS = Number(process.env.VK_READY_TIMEOUT_MS ?? 900_000);

// Startup failures that are cryptic enough to be worth translating.
const KNOWN_FAILURES = [
  [/Migrate\(VersionMismatch/, 'Migration checksum mismatch — migration .sql files have CRLF endings.\n  Fix: git config core.autocrlf false && git rm --cached -rq . && git reset --hard'],
  [/Unable to find libclang/, `libclang.dll not found — sqlx's sqlite-preupdate-hook needs bindgen.\n  Fix: winget install LLVM.LLVM, then set LIBCLANG_PATH=${LLVM_BIN}`],
  [/'export'|n.o . reconhecido|is not recognized as an internal/, 'The dev script ran under cmd.exe, not bash.\n  Fix: run this driver (it sets npm_config_script_shell), or launch from Git Bash.'],
  [/error: could not compile|error\[E\d+\]/, 'Rust compile error — see the log above.'],
  [/EADDRINUSE|Address already in use/, 'Port collision. Stop the other dev server, or delete .dev-ports.json.'],
];

const fail = (msg) => { console.error(`\n[FAIL] ${msg}`); process.exit(1); };

const ps = (script) => execFileSync('powershell', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8' });

// Ctrl-C / SIGTERM on Windows kills only the top shim: cargo-watch, cargo and
// vite survive, keep holding ports, and the next run silently shifts to port+1.
// So kill by tree, and sweep leftovers from earlier runs before starting.
function sweepStale() {
  const matched = ps(
    `Get-CimInstance Win32_Process -Filter "Name='node.exe' OR Name='cargo.exe' OR Name='cargo-watch.exe' OR Name='server.exe'" |` +
    ` Where-Object { $_.ProcessId -ne ${process.pid} -and (` +
    `   $_.CommandLine -like '*watch -w crates*' -or` +
    `   ($_.CommandLine -like '*vibe-kanban*' -and ($_.CommandLine -like '*concurrently*' -or $_.CommandLine -like '*vite*'))` +
    ` ) } | Select-Object -ExpandProperty ProcessId`,
  ).trim().split(/\s+/).filter(Boolean);

  for (const pid of matched) killTree(pid);
  if (matched.length) console.log(`swept ${matched.length} stale dev process(es) from earlier runs`);
  return matched.length;
}

function killTree(pid) {
  try { execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' }); } catch { /* already gone */ }
}

function preflight() {
  const problems = [];

  // The CRLF trap: sqlx checksums migration file *bytes*, so CRLF breaks all of
  // them at once. Cheap to detect here, 3+ minutes of build to discover otherwise.
  const crlf = readdirSync(MIGRATIONS)
    .filter((f) => f.endsWith('.sql'))
    .filter((f) => readFileSync(path.join(MIGRATIONS, f)).includes('\r\n'));
  if (crlf.length) {
    problems.push(
      `${crlf.length}/${readdirSync(MIGRATIONS).filter((f) => f.endsWith('.sql')).length} migrations have CRLF endings — the backend will crash with Migrate(VersionMismatch).\n` +
      `      Fix: git config core.autocrlf false && git rm --cached -rq . && git reset --hard`,
    );
  }

  const libclang = existsSync(path.join(process.env.LIBCLANG_PATH || LLVM_BIN, 'libclang.dll'));
  if (!libclang) problems.push(`libclang.dll not found (looked in ${process.env.LIBCLANG_PATH || LLVM_BIN}).\n      Fix: winget install LLVM.LLVM`);

  if (!existsSync(BASH)) problems.push(`Git Bash not found at ${BASH} — the dev script needs a POSIX shell.`);

  console.log(`preflight: migrations ${crlf.length ? 'CRLF ✗' : 'LF ✓'} | libclang ${libclang ? '✓' : '✗'} | bash ${existsSync(BASH) ? '✓' : '✗'}`);
  return problems;
}

function start() {
  const child = spawn('pnpm', ['run', 'dev'], {
    cwd: ROOT,
    shell: true, // pnpm is a .cmd shim on Windows
    env: {
      ...process.env,
      npm_config_script_shell: BASH, // the dev script is bash: `export VAR=$(...)`
      LIBCLANG_PATH: process.env.LIBCLANG_PATH || LLVM_BIN,
    },
  });

  const ports = {};
  let settled = false;

  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      if (!settled) reject(new Error(`not ready after ${READY_TIMEOUT_MS / 1000}s (cold Rust build can be slow — raise VK_READY_TIMEOUT_MS)`));
    }, READY_TIMEOUT_MS);

    const onChunk = (buf) => {
      const raw = buf.toString();
      process.stdout.write(raw);
      // Vite colorizes its output, and the port sits *behind* an escape code
      // (`localhost:\x1b[1m3000`) — so strip ANSI before matching anything.
      const text = raw.replace(/\x1b\[[0-9;]*m/g, '');

      // Ports are auto-allocated and shift between runs — parse, never assume.
      ports.backend ??= text.match(/Main server on :(\d+)/)?.[1];
      ports.frontend ??= text.match(/Local:\s+http:\/\/localhost:(\d+)/)?.[1];

      for (const [re, diagnosis] of KNOWN_FAILURES) {
        if (re.test(text) && !settled) {
          settled = true;
          clearTimeout(timer);
          reject(new Error(diagnosis));
          return;
        }
      }

      if (ports.backend && ports.frontend && !settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ child, ports });
      }
    };

    child.stdout.on('data', onChunk);
    child.stderr.on('data', onChunk);
    child.on('exit', (code) => {
      if (!settled) { settled = true; clearTimeout(timer); reject(new Error(`dev server exited with code ${code} before becoming ready`)); }
    });
  });
}

// The check that matters: hitting /api/health *through* the Vite proxy proves the
// frontend is actually wired to the port the backend really bound.
async function smoke({ frontend, backend }) {
  const get = async (url) => {
    const res = await fetch(url);
    return { status: res.status, body: await res.text() };
  };

  const html = await get(`http://localhost:${frontend}/`);
  if (html.status !== 200) throw new Error(`frontend returned HTTP ${html.status}`);

  const direct = await get(`http://127.0.0.1:${backend}/api/health`);
  if (direct.status !== 200) throw new Error(`backend :${backend} returned HTTP ${direct.status}`);

  const proxied = await get(`http://localhost:${frontend}/api/health`);
  if (!proxied.body.includes('"success":true')) {
    throw new Error(`proxied /api/health failed (HTTP ${proxied.status}): ${proxied.body.slice(0, 200)}\n  The Vite proxy is not pointing at the port the backend bound.`);
  }

  console.log(`\nsmoke: frontend ${html.status} | backend :${backend} ${direct.status} | proxied /api/health ok`);
}

const mode = process.argv[2] ?? '--serve';

if (mode === '--kill') { console.log(`swept ${sweepStale()} process(es)`); process.exit(0); }

const problems = preflight();
if (problems.length) fail(`preflight failed:\n  - ${problems.join('\n  - ')}`);
if (mode === '--check') { console.log('preflight ok'); process.exit(0); }

sweepStale();

let handle;
try {
  handle = await start();
  await smoke(handle.ports);
} catch (err) {
  if (handle) killTree(handle.child.pid);
  else sweepStale();
  fail(err.message);
}

console.log(`\n==> OPEN: http://localhost:${handle.ports.frontend}  (backend :${handle.ports.backend})`);

if (mode === '--smoke') {
  killTree(handle.child.pid);
  sweepStale();
  process.exit(0);
}

console.log('Dev server running. Ctrl-C to stop.');
process.on('SIGINT', () => { killTree(handle.child.pid); sweepStale(); process.exit(0); });
