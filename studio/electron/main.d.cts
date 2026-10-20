// Type declarations for the Electron main process module. The runtime
// module is CommonJS and side-effect-free on require: the actual startup
// lives in `electron/bootstrap.cjs`, which calls the exported
// `bootstrap()` function. `main.cjs` never starts anything at import time.

import type { EventEmitter } from 'node:events';
import type { ChildProcess } from 'node:child_process';

export interface BackendInvocation {
  command: string;
  args: string[];
  cwd: string;
  env: NodeJS.ProcessEnv;
  port: number;
  token: string;
}

export interface BackendProcessOptions {
  command: string;
  args: string[];
  cwd: string;
  env: NodeJS.ProcessEnv;
  port: number;
  token: string;
}

export type BackendMode =
  | 'packaged-native'
  | 'native'
  | 'python-override'
  | 'python-reference'
  | 'invalid';

export interface ResolvedBackend {
  command: string;
  args: string[];
  cwd: string;
  env: NodeJS.ProcessEnv;
  exists: boolean;
  mode: BackendMode;
  instructions: string;
}

export interface ResolveBackendOptions {
  isPackaged?: boolean;
  platform?: NodeJS.Platform;
  resourcesPath?: string;
  repoRoot?: string;
  devBackend?: string;
  python?: string;
}

export interface BackendExitInfo {
  code: number | null;
  signal: NodeJS.Signals | null;
}

export interface BackendProcessInstance {
  start(): void;
  waitForHealth(timeoutMs: number): Promise<void>;
  shutdown(): void;
  readonly error: Error | null;
  readonly exited: boolean;
  readonly ready: boolean;
  readonly expected: boolean;
  readonly exitCode: number | null;
  readonly exitSignal: NodeJS.Signals | null;
  onUnexpectedExit: ((info: BackendExitInfo) => void) | null;
}

export interface FakeWin {
  webContents: {
    mainFrame: { url: string };
    isDestroyed(): boolean;
  };
}

export interface FakeEvent {
  sender: { isDestroyed(): boolean };
  senderFrame: { url: string };
}

export function bootstrap(): Promise<void>;
export function buildBackendInvocation(options: BackendProcessOptions): BackendInvocation;
export function resolveBackendCommand(options?: ResolveBackendOptions): ResolvedBackend;
export function packagedBackendEnvironment(options?: {
  resourcesPath: string;
  environ?: NodeJS.ProcessEnv;
  platform?: NodeJS.Platform;
}): NodeJS.ProcessEnv;
export function validateIpcSender(event: FakeEvent, win: FakeWin, trustedOrigin: string): void;
export function pickFreePort(): Promise<number>;
export function generateToken(): string;
export const BackendProcess: new (options: BackendProcessOptions) => BackendProcessInstance;
export function attachSecurityPolicy(win: unknown, trustedOrigin: string): void;
export function registerIpc(win: unknown, trustedOrigin: string, baseUrl: string, token: string): void;

export const TRUSTED_HOST: string;
export const PRELOAD_PATH: string;
export const REPO_ROOT: string;
export const NATIVE_STAGE_DIR: string;
export const DEV_BACKEND_ENV: string;

// Mark these as ambient so unused-variable analysis does not flag them
// when the consumer imports only the helpers.
declare const __unused__: EventEmitter;
declare const __child__: ChildProcess;
void __unused__;
void __child__;