// Tests for the bounded backend log and the startup failure dialog.
//
// The packaged app used to discard the backend child's stdout/stderr and to
// exit silently when the backend never became ready. These tests cover the
// three things that must never regress:
//
//   1. the log is bounded (it rotates and never grows without limit),
//   2. the bearer token never reaches the log or the dialog, even when the
//      backend echoes its own environment,
//   3. a startup failure is shown to the user with the log path and the last
//      log lines.
//
// Electron itself is not loaded: `main.cjs` is imported in a plain Node test
// and every Electron surface it needs (the log directory, the error dialog)
// is injected as a mock, exactly like the shipped main process passes them.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

type Main = typeof import('../electron/main.cjs');

let mainModule: Main;
const temporaryDirectories: string[] = [];
const TOKEN = 'unit-test-bearer-token-0123456789abcdef';

function makeTemporaryDirectory(): string {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-backend-log-'));
  temporaryDirectories.push(directory);
  return directory;
}

async function waitFor(predicate: () => boolean, timeoutMs = 10_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error('the condition was not met in time');
}

beforeEach(async () => {
  const mod = (await import('../electron/main.cjs' as string)) as { default?: Main } & Main;
  mainModule = (mod.default ?? mod) as Main;
});

afterEach(() => {
  vi.restoreAllMocks();
  for (const directory of temporaryDirectories.splice(0)) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

describe('the bounded backend log', () => {
  it('rotates instead of growing without limit', () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({
      dir: directory,
      maxBytes: 1024,
      maxFiles: 5,
    });

    for (let index = 0; index < 40; index += 1) {
      log.write(`${String(index).padStart(4, '0')} ${'x'.repeat(200)}\n`);
    }

    const files = log.existingFiles();
    expect(files.length).toBeGreaterThan(1);
    // Five files in total: the current one plus four archives.
    expect(files.length).toBeLessThanOrEqual(5);
    for (const file of files) {
      expect(fs.statSync(file).size).toBeLessThanOrEqual(1024);
    }
    // The newest lines survive the rotation.
    expect(fs.readFileSync(log.filePath, 'utf8')).toContain('0039');
  });

  it('keeps the last lines in memory for the error dialog', () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({ dir: directory });
    for (let index = 0; index < 30; index += 1) {
      log.write(`line ${index}\n`);
    }
    const tail = log.tail(20);
    expect(tail).toHaveLength(20);
    expect(tail[0]).toBe('line 10');
    expect(tail[19]).toBe('line 29');
  });

  it('never writes the bearer token, even when the backend echoes it', () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({ dir: directory, secrets: [TOKEN] });
    log.write(`starting with KALA_STUDIO_TOKEN=${TOKEN}\n`);
    log.write(`the raw token is ${TOKEN}\n`);
    log.write('Authorization: Bearer another-secret-value\n');

    const written = fs.readFileSync(log.filePath, 'utf8');
    expect(written).not.toContain(TOKEN);
    expect(written).not.toContain('another-secret-value');
    expect(written).toContain('[redacted]');
    expect(log.tail(10).join('\n')).not.toContain(TOKEN);
  });

  it('scrubs an environment assignment that names the token', () => {
    const scrubbed = mainModule.scrubSecrets(`KALA_STUDIO_TOKEN=${TOKEN}`, [TOKEN]);
    expect(scrubbed).not.toContain(TOKEN);
    expect(scrubbed).toContain('KALA_STUDIO_TOKEN=');
  });

  it('captures the backend child output and scrubs the token out of it', async () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({ dir: directory, secrets: [TOKEN] });
    const backend = new mainModule.BackendProcess({
      command: process.execPath,
      args: ['-e', 'console.error("the backend said hello"); console.log(process.env.KALA_STUDIO_TOKEN);'],
      cwd: process.cwd(),
      env: { ...process.env, KALA_STUDIO_TOKEN: TOKEN },
      port: 4321,
      token: TOKEN,
      logger: log,
    });
    backend.start();
    await waitFor(() => {
      try {
        return fs.readFileSync(log.filePath, 'utf8').includes('the backend said hello');
      } catch {
        return false;
      }
    });

    const written = fs.readFileSync(log.filePath, 'utf8');
    expect(written).toContain('the backend said hello');
    expect(written).not.toContain(TOKEN);
  });
});

describe('the startup failure dialog', () => {
  it('shows the reason, the log path and the last log lines', () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({ dir: directory, secrets: [TOKEN] });
    log.write('the backend binary is missing\n');
    log.write(`KALA_STUDIO_TOKEN=${TOKEN}\n`);

    const showErrorBox = vi.fn();
    const body = mainModule.reportStartupFailure({
      dialog: { showErrorBox },
      logger: log,
      title: 'Kala Studio could not start',
      reason: 'The backend program was not found.',
    });

    expect(showErrorBox).toHaveBeenCalledTimes(1);
    const [title, content] = showErrorBox.mock.calls[0] as [string, string];
    expect(title).toBe('Kala Studio could not start');
    expect(content).toContain('The backend program was not found.');
    expect(content).toContain(log.filePath);
    expect(content).toContain('the backend binary is missing');
    // The token never reaches the dialog.
    expect(content).not.toContain(TOKEN);
    expect(body).toBe(content);
  });

  it('survives a machine that cannot show a dialog', () => {
    const directory = makeTemporaryDirectory();
    const log = new mainModule.RotatingLog({ dir: directory });
    const body = mainModule.reportStartupFailure({
      dialog: {
        showErrorBox: () => {
          throw new Error('no display');
        },
      },
      logger: log,
      reason: 'The backend never became ready.',
      console: { error: () => undefined },
    });
    expect(body).toContain('The backend never became ready.');
    expect(body).toContain(log.filePath);
  });

  it('puts the log file under the Electron logs directory', () => {
    const electronApp = { getPath: (name: string) => path.join('/tmp/kala-electron-mock', name) };
    expect(mainModule.resolveLogsDirectory(electronApp)).toBe(path.join('/tmp/kala-electron-mock', 'logs'));
    expect(mainModule.resolveLogsDirectory(null)).toContain(path.join('studio', 'logs'));
  });
});
