// Tests for the connection bootstrap helpers.
//
// `normalizeLoopbackOrigin` must:
//   - accept loopback URLs with a port
//   - refuse URLs with credentials, query strings, or hashes
//   - refuse non-loopback hostnames
//   - normalise to scheme://host (no path, no trailing slash)
//
// `validateConnection` composes the origin + token validation that the
// in-page form runs before handing the connection to the store.

import { describe, expect, it } from 'vitest';
import { normalizeLoopbackOrigin, validateConnection } from '../src/api/connection';

describe('normalizeLoopbackOrigin', () => {
  it('accepts 127.0.0.1 with a port', () => {
    expect(normalizeLoopbackOrigin('http://127.0.0.1:8765')).toBe('http://127.0.0.1:8765');
  });
  it('accepts localhost with a port', () => {
    expect(normalizeLoopbackOrigin('http://localhost:8765')).toBe('http://localhost:8765');
  });
  it('accepts IPv6 loopback', () => {
    expect(normalizeLoopbackOrigin('http://[::1]:8765')).toBe('http://[::1]:8765');
  });
  it('strips a trailing slash and path', () => {
    expect(normalizeLoopbackOrigin('http://127.0.0.1:8765/')).toBe('http://127.0.0.1:8765');
    expect(normalizeLoopbackOrigin('http://127.0.0.1:8765/api/studio/health')).toBe('http://127.0.0.1:8765');
  });
  it('refuses a URL without a port', () => {
    expect(normalizeLoopbackOrigin('http://127.0.0.1')).toBeNull();
  });
  it('refuses a non-loopback host', () => {
    expect(normalizeLoopbackOrigin('http://example.com:8765')).toBeNull();
  });
  it('refuses a URL with credentials', () => {
    expect(normalizeLoopbackOrigin('http://user:pass@127.0.0.1:8765')).toBeNull();
  });
  it('refuses a URL with a query string', () => {
    expect(normalizeLoopbackOrigin('http://127.0.0.1:8765?token=secret')).toBeNull();
  });
  it('refuses a URL with a hash', () => {
    expect(normalizeLoopbackOrigin('http://127.0.0.1:8765#x')).toBeNull();
  });
  it('refuses a non-http(s) scheme', () => {
    expect(normalizeLoopbackOrigin('file:///etc/passwd')).toBeNull();
    expect(normalizeLoopbackOrigin('ws://127.0.0.1:8765')).toBeNull();
  });
});

describe('validateConnection', () => {
  it('returns a usable StudioConnection for a valid pair', () => {
    const c = validateConnection('http://127.0.0.1:8765', 'a'.repeat(20), 'desktop');
    expect(c.baseUrl).toBe('http://127.0.0.1:8765');
    expect(c.mode).toBe('desktop');
  });

  it('rejects short tokens', () => {
    expect(() => validateConnection('http://127.0.0.1:8765', 'short', 'desktop')).toThrow();
  });

  it('rejects non-loopback origins', () => {
    expect(() => validateConnection('http://example.com', 'a'.repeat(20), 'desktop')).toThrow();
  });
});

// (no additional helpers)