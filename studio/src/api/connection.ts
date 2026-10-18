// Connection bootstrap.
//
// Desktop mode: the Electron preload script exposes a single
// `window.kalaStudio.getConnection()` that returns `{baseUrl, token}` to
// the trusted top-level frame only. We never expose a generic IPC channel.
//
// Browser mode: the operator fills an in-page form (default origin from
// `window.location.origin`, token in a password input). The bootstrap
// rejects any origin that is not an exact-match loopback URL, refuses URL
// credentials or query strings, and never persists the token.
//
// Nothing in this module writes to disk, sends telemetry, or reads anything
// beyond the typed bridge.

import type { StudioConnection } from '../types/studio';

export interface KalaStudioBridge {
  getConnection(): Promise<{ baseUrl: string; token: string }>;
}

declare global {
  interface Window {
    kalaStudio?: KalaStudioBridge;
  }
}

export interface ConnectionFormValues {
  baseUrl: string;
  token: string;
}

export interface ConnectionFormProps {
  defaultBaseUrl: string;
  onSubmit(values: ConnectionFormValues): void;
  onCancel?(): void;
}

export async function resolveConnectionFromBridge(): Promise<StudioConnection> {
  if (typeof window === 'undefined') {
    throw new Error('studio connection can only be resolved in a browser context');
  }
  const bridge = window.kalaStudio;
  if (!bridge || typeof bridge.getConnection !== 'function') {
    throw new Error('desktop bridge unavailable');
  }
  const result = await bridge.getConnection();
  if (!result || typeof result.baseUrl !== 'string' || typeof result.token !== 'string') {
    throw new Error('desktop bridge returned a malformed connection');
  }
  return validateConnection(result.baseUrl, result.token, 'desktop');
}

export function validateConnection(rawBaseUrl: string, token: string, mode: StudioConnection['mode']): StudioConnection {
  const baseUrl = normalizeLoopbackOrigin(rawBaseUrl);
  if (!baseUrl) {
    throw new Error('origin must be a loopback URL (http://127.0.0.1:<port>, http://[::1]:<port>, or http://localhost:<port>) with no credentials or query string');
  }
  if (typeof token !== 'string' || token.length < 16) {
    throw new Error('token must be a string of at least 16 characters');
  }
  return { baseUrl, token, mode };
}

export function normalizeLoopbackOrigin(input: string): string | null {
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return null;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
  if (url.username || url.password) return null;
  if (url.search || url.hash) return null;
  const host = url.hostname.toLowerCase();
  const isLoopback = host === '127.0.0.1' || host === 'localhost' || host === '::1' || host === '[::1]';
  if (!isLoopback) return null;
  if (url.port === '') return null;
  if (!/^\d+$/.test(url.port)) return null;
  // Origin is scheme + host + port; never the path.
  return `${url.protocol}//${url.host}`;
}

export function currentOriginOrEmpty(): string {
  if (typeof window === 'undefined') return '';
  try {
    const url = new URL(window.location.href);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return '';
    if (!url.port) return '';
    return `${url.protocol}//${url.host}`;
  } catch {
    return '';
  }
}