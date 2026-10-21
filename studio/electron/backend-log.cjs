// Kala Studio — bounded backend log and startup failure dialog.
//
// The native backend is a child process of the Electron main process. Its
// stdout/stderr used to be discarded, so a packaged launch that failed
// (missing binary, spawn error, early exit, health timeout) showed nothing at
// all and the user had no way to report what happened.
//
// This module is the ONLY writer of that log. The backend never writes it,
// never knows where it is, and never receives a path to it.
//
//   * `RotatingLog` appends the child's output to `<logs>/backend.log` and
//     rotates it: the current file plus four archives, each at most 1 MiB,
//     so the log can never grow past ~5 MiB.
//   * `scrubSecrets` removes the bearer token (and anything shaped like a
//     token or an `Authorization` header) from every chunk before it is
//     written, so a backend that echoes its environment cannot leak the
//     token into a file the user may attach to a bug report.
//   * `formatStartupFailure` builds the plain-text body of the startup error
//     dialog: what failed, where the log is, and the last log lines.
//
// Nothing here ever sees a prompt, a user message or an API key: the backend
// prints error kinds and names only, and this module never adds anything of
// its own to the log.

'use strict';

const fs = require('node:fs');
const path = require('node:path');

// 1 MiB per file, five files in total (the current one plus four archives).
const DEFAULT_MAX_BYTES = 1024 * 1024;
const DEFAULT_MAX_FILES = 5;
const DEFAULT_NAME = 'backend.log';
const REDACTION = '[redacted]';
// How many lines are kept in memory for the error dialog.
const DEFAULT_RECENT_LINES = 200;
// How many lines the dialog shows.
const DIALOG_TAIL_LINES = 20;

/**
 * Remove secrets from a chunk of backend output.
 *
 * `secrets` holds literal values (the bearer token) that must never reach the
 * log or the dialog. The generic patterns cover the shapes a backend or a
 * library might echo: an environment assignment and an `Authorization` header.
 */
function scrubSecrets(text, secrets = []) {
  let scrubbed = String(text ?? '');
  const literals = [...new Set(
    secrets.filter((value) => typeof value === 'string' && value.length >= 8),
  )].sort((left, right) => right.length - left.length);
  for (const secret of literals) {
    scrubbed = scrubbed.split(secret).join(REDACTION);
  }
  scrubbed = scrubbed.replace(
    /(KALA_STUDIO_TOKEN\s*[=:]\s*)(\S+)/gi,
    (_match, prefix) => `${prefix}${REDACTION}`,
  );
  scrubbed = scrubbed.replace(
    /(Authorization\s*:\s*Bearer\s+)(\S+)/gi,
    (_match, prefix) => `${prefix}${REDACTION}`,
  );
  return scrubbed;
}

/**
 * A bounded, rotating text log.
 *
 * The log directory and the file name are chosen by the caller (Electron's
 * `app.getPath('logs')` in production). Rotation keeps at most `maxFiles`
 * files: `backend.log`, `backend.log.1` … `backend.log.4`.
 */
class RotatingLog {
  constructor({
    dir,
    name = DEFAULT_NAME,
    maxBytes = DEFAULT_MAX_BYTES,
    maxFiles = DEFAULT_MAX_FILES,
    secrets = [],
    recentLines = DEFAULT_RECENT_LINES,
  } = {}) {
    if (!dir) throw new Error('RotatingLog requires a directory');
    this.dir = path.resolve(dir);
    this.name = name;
    this.maxBytes = Math.max(1024, Number(maxBytes) || DEFAULT_MAX_BYTES);
    this.maxFiles = Math.max(1, Number(maxFiles) || DEFAULT_MAX_FILES);
    this.secrets = [...secrets];
    this.recentLines = Math.max(1, Number(recentLines) || DEFAULT_RECENT_LINES);
    this.recent = [];
    this.bytes = 0;
    this.failed = null;
    try {
      fs.mkdirSync(this.dir, { recursive: true });
      this.bytes = this.currentSize();
    } catch (error) {
      this.failed = error;
    }
  }

  /** The absolute path of the file that is currently being written. */
  get filePath() {
    return path.join(this.dir, this.name);
  }

  /** The absolute paths of every file this log owns, oldest archive last. */
  get filePaths() {
    const files = [this.filePath];
    for (let index = 1; index < this.maxFiles; index += 1) {
      files.push(`${this.filePath}.${index}`);
    }
    return files;
  }

  currentSize() {
    try {
      return fs.statSync(this.filePath).size;
    } catch {
      return 0;
    }
  }

  /** Rotate the log so the current file starts empty again. */
  rotate() {
    const files = this.filePaths;
    // The oldest archive falls off the end.
    try {
      fs.rmSync(files[files.length - 1], { force: true });
    } catch {
      // ignore
    }
    for (let index = files.length - 2; index >= 1; index -= 1) {
      try {
        fs.renameSync(files[index], files[index + 1]);
      } catch {
        // ignore
      }
    }
    try {
      fs.renameSync(files[0], files[1]);
    } catch {
      // ignore
    }
    this.bytes = 0;
  }

  /**
   * Append a chunk of text. The chunk is scrubbed first, so the token can
   * never be written even if the backend echoes its own environment.
   */
  write(chunk) {
    const text = scrubSecrets(chunk, this.secrets);
    if (!text) return;
    const lines = text.split(/\r?\n/);
    // A trailing newline ends the last line; it does not start a new one.
    if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
    for (const line of lines) {
      this.recent.push(line);
    }
    while (this.recent.length > this.recentLines) this.recent.shift();
    if (this.failed) return;
    try {
      const size = Buffer.byteLength(text, 'utf8');
      if (this.bytes > 0 && this.bytes + size > this.maxBytes) this.rotate();
      fs.appendFileSync(this.filePath, text, { mode: 0o600 });
      this.bytes += size;
    } catch (error) {
      this.failed = error;
    }
  }

  /** The last `count` lines that were written, newest last. */
  tail(count = DIALOG_TAIL_LINES) {
    const wanted = Math.max(0, Number(count) || 0);
    if (wanted === 0) return [];
    return this.recent.slice(Math.max(0, this.recent.length - wanted));
  }

  /** The files this log owns that exist on disk, newest first. */
  existingFiles() {
    const files = [];
    for (const candidate of this.filePaths) {
      try {
        if (fs.statSync(candidate).isFile()) files.push(candidate);
      } catch {
        // ignore
      }
    }
    return files;
  }
}

/**
 * Build the plain-text body of the startup failure dialog.
 *
 * The body names the failure, the log file and the last log lines. It never
 * contains the token: the log is scrubbed on the way in and the reason text
 * comes from the Studio, not from the backend.
 */
function formatStartupFailure({ reason, logPath, lines = [], tailLines = DIALOG_TAIL_LINES } = {}) {
  const parts = [];
  const text = String(reason ?? '').trim();
  parts.push(text.length > 0 ? text : 'Kala Studio could not start.');
  if (logPath) {
    parts.push('');
    parts.push(`Log file: ${logPath}`);
  }
  const shown = lines.slice(Math.max(0, lines.length - Math.max(0, tailLines)));
  if (shown.length > 0) {
    parts.push('');
    parts.push(`Last ${shown.length} log line${shown.length === 1 ? '' : 's'}:`);
    for (const line of shown) parts.push(line);
  }
  return parts.join('\n');
}

/**
 * Show a startup failure to the user.
 *
 * `dialog` is injectable so the path can be tested without Electron. The
 * function never throws: a headless machine without a dialog must still be
 * able to report the failure on stderr.
 */
function reportStartupFailure({
  dialog,
  logger = null,
  title = 'Kala Studio could not start',
  reason,
  tailLines = DIALOG_TAIL_LINES,
  console: consoleLike = console,
} = {}) {
  const logPath = logger && logger.filePath ? logger.filePath : null;
  const lines = logger && typeof logger.tail === 'function' ? logger.tail(tailLines) : [];
  const body = formatStartupFailure({ reason, logPath, lines, tailLines });
  try {
    if (dialog && typeof dialog.showErrorBox === 'function') {
      dialog.showErrorBox(title, body);
    }
  } catch {
    // A dialog that refuses to open must not take the app down.
  }
  try {
    if (consoleLike && typeof consoleLike.error === 'function') {
      consoleLike.error(`[kala-studio] ${title}\n${body}`);
    }
  } catch {
    // ignore
  }
  return body;
}

module.exports = {
  DEFAULT_MAX_BYTES,
  DEFAULT_MAX_FILES,
  DEFAULT_NAME,
  DIALOG_TAIL_LINES,
  REDACTION,
  RotatingLog,
  formatStartupFailure,
  reportStartupFailure,
  scrubSecrets,
};
