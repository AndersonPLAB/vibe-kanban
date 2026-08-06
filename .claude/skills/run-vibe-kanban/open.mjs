#!/usr/bin/env node
// Desktop-icon entrypoint: open the workspaces board in the default browser.
//
//   node open.mjs [--root <repo>] [--log-dir <dir>] [--path /workspaces/board] [--no-open]
//
// Normally this starts nothing: it reads the port the dev server actually bound
// from .dev-ports.json and just opens the browser. If that server is not
// answering (no ports file, stale port, backend dead), it starts the driver
// detached, waits for it, and only then opens the browser — so the icon works
// with or without autostart.
//
// --no-open is the autostart path: same start-if-needed logic, no browser. It
// shares this file so the server is always launched the same way — detached,
// with stdio on a log file. Handing the dev script an inherited console stdin
// instead (a scheduled task's) kills vite seconds after it comes up.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, openSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i > 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};

const ROOT = path.resolve(arg('--root', path.resolve(import.meta.dirname, '../../..')));
const LOG_DIR = path.resolve(arg('--log-dir', os.tmpdir()));
const BOARD = arg('--path', '/workspaces/board');
const PORTS_FILE = path.join(ROOT, '.dev-ports.json');
// Sibling driver.mjs: the repo's when run from the repo, the installed copy when
// run from the launcher directory. Either way it targets ROOT via VK_ROOT.
const DRIVER = path.join(import.meta.dirname, 'driver.mjs');
const LOCK = path.join(LOG_DIR, 'vk-open.lock');
const READY_TIMEOUT_MS = Number(process.env.VK_READY_TIMEOUT_MS ?? 900_000);

const log = (msg) => console.log(`[${new Date().toISOString()}] ${msg}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The port drifts between runs (see SKILL.md), so re-read the file every poll —
// the driver rewrites it while starting.
const readPort = () => {
  try {
    return JSON.parse(readFileSync(PORTS_FILE, 'utf8')).frontend ?? null;
  } catch {
    return null;
  }
};

// A listening port is not enough: something else may hold it, or the backend may
// have died behind a live Vite. /api/health through the proxy proves the pair.
const alive = async (port) => {
  try {
    const res = await fetch(`http://localhost:${port}/api/health`, { signal: AbortSignal.timeout(2000) });
    return (await res.text()).includes('"success":true');
  } catch {
    return false;
  }
};

const NO_OPEN = process.argv.includes('--no-open');

const openBrowser = (url) => {
  if (NO_OPEN) {
    log(`ready at ${url} (not opening a browser)`);
    return;
  }
  log(`opening ${url}`);
  spawn('cmd', ['/c', 'start', '', url], { detached: true, stdio: 'ignore', windowsHide: true }).unref();
};

// Two drivers at once is the documented 0xc0000142 failure: the second one's
// sweep kills the first mid-build. So one cold start at a time.
const lockHeld = () => {
  if (!existsSync(LOCK)) return false;
  const pid = Number(readFileSync(LOCK, 'utf8').trim());
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false; // stale lock from a crashed run
  }
};

const startDriver = () => {
  const out = openSync(path.join(LOG_DIR, 'vk-server.log'), 'a');
  const child = spawn(process.execPath, [DRIVER], {
    cwd: ROOT,
    env: { ...process.env, VK_ROOT: ROOT },
    detached: true, // outlive this process: the icon exits, the server keeps running
    stdio: ['ignore', out, out],
    windowsHide: true,
  });
  child.unref();
  log(`started driver (pid ${child.pid}), log: ${path.join(LOG_DIR, 'vk-server.log')}`);
};

mkdirSync(LOG_DIR, { recursive: true });

const port = readPort();
if (port && (await alive(port))) {
  openBrowser(`http://localhost:${port}${BOARD}`);
  process.exit(0);
}

log(`no server on ${port ? `:${port}` : 'record'} — cold start`);
if (lockHeld()) {
  log('another cold start is already in progress; exiting');
  process.exit(0);
}
writeFileSync(LOCK, String(process.pid));
process.on('exit', () => { try { rmSync(LOCK); } catch {} });

startDriver();

const deadline = Date.now() + READY_TIMEOUT_MS;
while (Date.now() < deadline) {
  await sleep(2000);
  const p = readPort();
  if (p && (await alive(p))) {
    openBrowser(`http://localhost:${p}${BOARD}`);
    process.exit(0);
  }
}

log(`server did not come up within ${READY_TIMEOUT_MS / 1000}s — see vk-server.log`);
process.exit(1);
