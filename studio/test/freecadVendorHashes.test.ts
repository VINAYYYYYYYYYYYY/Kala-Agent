// FreeCAD wheel vendoring: reviewed SHA-256 pins + `pip download --require-hashes`.
//
// The staging helper downloads `pydantic` and its runtime dependencies for
// FreeCAD's own interpreter. `--require-hashes` is only meaningful when every
// pinned requirement carries its reviewed digests, so a missing/empty/malformed
// list must fail the build instead of silently allowing an arbitrary mirror to
// serve different bytes. These tests cover the pins file, the hash validation,
// the generated hashed requirements file and the actual pip argument list.

import { afterEach, describe, expect, it } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const require = createRequire(import.meta.url);
const vendor = require('../scripts/freecad-python-vendor.cjs') as {
  DEFAULT_PINS_FILE: string;
  readPins: (pinsFile?: string) => {
    requirements: string[];
    hashes: Record<string, string[]>;
  };
  validateRequirementHashes: (
    rawHashes: unknown,
    requirements: string[],
    pinsFile: string,
  ) => Record<string, string[]>;
  writeHashedRequirementsFile: (options: {
    requirements: string[];
    hashes: Record<string, string[]>;
    destination: string;
  }) => string;
  downloadWheels: (options: {
    requirements: string[];
    hashes?: Record<string, string[]>;
    pythonVersion: string;
    abi: string;
    wheelTag: string;
    destination: string;
    pipPython: string;
    env?: NodeJS.ProcessEnv;
    runner?: (
      command: string,
      args: string[],
      options?: unknown,
    ) => { status: number; stdout?: string; stderr?: string };
  }) => Array<{ name: string; sha256: string }>;
};

const temporaryDirectories: string[] = [];

function makeTemporaryDirectory(): string {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'kala-freecad-hashes-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(() => {
  while (temporaryDirectories.length > 0) {
    const directory = temporaryDirectories.pop();
    if (directory) fs.rmSync(directory, { recursive: true, force: true });
  }
});

const DIGEST = 'a'.repeat(64);
const OTHER_DIGEST = 'b'.repeat(64);

describe('freecad-python-deps.json', () => {
  it('pins a reviewed SHA-256 digest list for every requirement', () => {
    const pins = vendor.readPins();
    expect(pins.requirements.length).toBeGreaterThan(0);
    for (const requirement of pins.requirements) {
      const digests = pins.hashes[requirement]!;
      expect(digests, `hashes for ${requirement}`).toBeDefined();
      expect(digests.length).toBeGreaterThan(0);
      for (const digest of digests) expect(digest).toMatch(/^[0-9a-f]{64}$/);
    }
    // A compiled dependency ships one wheel per platform; the pins must cover
    // more than one artifact, not just the build host's.
    expect(Object.values(pins.hashes).some((digests) => digests.length > 1)).toBe(true);
  });

  it('refuses a pins file without the hashes object', () => {
    const directory = makeTemporaryDirectory();
    const pinsFile = path.join(directory, 'pins.json');
    fs.writeFileSync(
      pinsFile,
      JSON.stringify({ requirements: ['pydantic==2.13.5'] }),
      'utf8',
    );
    expect(() => vendor.readPins(pinsFile)).toThrow(/must carry a "hashes" object/);
  });

  it('refuses a pins file that leaves one requirement unhashed', () => {
    const directory = makeTemporaryDirectory();
    const pinsFile = path.join(directory, 'pins.json');
    fs.writeFileSync(
      pinsFile,
      JSON.stringify({
        requirements: ['pydantic==2.13.5', 'annotated-types==0.8.0'],
        hashes: { 'pydantic==2.13.5': [DIGEST] },
      }),
      'utf8',
    );
    expect(() => vendor.readPins(pinsFile)).toThrow(
      /no SHA-256 hashes for annotated-types==0\.8\.0/,
    );
  });
});

describe('validateRequirementHashes', () => {
  const requirements = ['pydantic==2.13.5'];

  it('copies the reviewed digests', () => {
    expect(
      vendor.validateRequirementHashes(
        { 'pydantic==2.13.5': [DIGEST, OTHER_DIGEST] },
        requirements,
        'pins.json',
      ),
    ).toEqual({ 'pydantic==2.13.5': [DIGEST, OTHER_DIGEST] });
  });

  it('refuses a non-object hash map', () => {
    expect(() => vendor.validateRequirementHashes([DIGEST], requirements, 'pins.json')).toThrow(
      /must carry a "hashes" object/,
    );
    expect(() => vendor.validateRequirementHashes(null, requirements, 'pins.json')).toThrow(
      /must carry a "hashes" object/,
    );
  });

  it('refuses an empty or missing digest list', () => {
    expect(() =>
      vendor.validateRequirementHashes({ 'pydantic==2.13.5': [] }, requirements, 'pins.json'),
    ).toThrow(/no SHA-256 hashes for pydantic==2\.13\.5/);
    expect(() => vendor.validateRequirementHashes({}, requirements, 'pins.json')).toThrow(
      /no SHA-256 hashes for pydantic==2\.13\.5/,
    );
  });

  it('refuses a malformed digest', () => {
    for (const digest of ['abc', DIGEST.toUpperCase(), `${DIGEST}0`, 42]) {
      expect(() =>
        vendor.validateRequirementHashes(
          { 'pydantic==2.13.5': [digest] },
          requirements,
          'pins.json',
        ),
      ).toThrow(/invalid SHA-256/);
    }
  });

  it('refuses hashes for requirements that are not pinned', () => {
    expect(() =>
      vendor.validateRequirementHashes(
        { 'pydantic==2.13.5': [DIGEST], 'urllib3==9.9.9': [DIGEST] },
        requirements,
        'pins.json',
      ),
    ).toThrow(/not pinned: urllib3==9\.9\.9/);
  });
});

describe('writeHashedRequirementsFile', () => {
  it('writes one --hash flag per reviewed digest', () => {
    const directory = makeTemporaryDirectory();
    const file = vendor.writeHashedRequirementsFile({
      requirements: ['pydantic==2.13.5', 'pydantic-core==2.46.5'],
      hashes: {
        'pydantic==2.13.5': [DIGEST],
        'pydantic-core==2.46.5': [DIGEST, OTHER_DIGEST],
      },
      destination: directory,
    });
    const lines = fs.readFileSync(file, 'utf8').trimEnd().split('\n');
    expect(lines).toEqual([
      `pydantic==2.13.5 --hash=sha256:${DIGEST}`,
      `pydantic-core==2.46.5 --hash=sha256:${DIGEST} --hash=sha256:${OTHER_DIGEST}`,
    ]);
  });

  it('refuses to write a requirements file without hashes', () => {
    const directory = makeTemporaryDirectory();
    expect(() =>
      vendor.writeHashedRequirementsFile({
        requirements: ['pydantic==2.13.5'],
        hashes: undefined as unknown as Record<string, string[]>,
        destination: directory,
      }),
    ).toThrow(/requires the pinned SHA-256 hashes/);
    expect(() =>
      vendor.writeHashedRequirementsFile({
        requirements: ['pydantic==2.13.5'],
        hashes: {},
        destination: directory,
      }),
    ).toThrow(/no reviewed SHA-256 hashes for pydantic==2\.13\.5/);
  });
});

describe('downloadWheels', () => {
  const requirements = ['pydantic==2.13.5'];
  const hashes = { 'pydantic==2.13.5': [DIGEST, OTHER_DIGEST] };

  it('runs pip in hash-checking mode against the hashed requirements file', () => {
    const destination = makeTemporaryDirectory();
    const calls: Array<{ command: string; args: string[]; options?: unknown }> = [];
    const runner = (
      command: string,
      args: string[],
      options?: unknown,
    ): { status: number; stdout: string; stderr: string } => {
      calls.push({ command, args, options });
      const dest = args[args.indexOf('--dest') + 1];
      fs.writeFileSync(path.join(dest as string, 'pydantic-2.13.5-py3-none-any.whl'), 'wheel');
      return { status: 0, stdout: '', stderr: '' };
    };

    const wheels = vendor.downloadWheels({
      requirements,
      hashes,
      pythonVersion: '3.12',
      abi: 'cp312',
      wheelTag: 'win_amd64',
      destination,
      pipPython: 'python',
      env: {},
      runner,
    });

    expect(calls).toHaveLength(1);
    const [call] = calls;
    expect(call?.command).toBe('python');
    const args = call?.args ?? [];
    expect(args).toContain('download');
    expect(args).toContain('--require-hashes');
    expect(args).toContain('--only-binary=:all:');
    expect(args).toContain('--no-input');
    expect(args).toContain('--disable-pip-version-check');
    expect(args).toContain('cp312');
    expect(args).toContain('win_amd64');

    const requirementsFile = args[args.indexOf('-r') + 1] as string;
    const contents = fs.readFileSync(requirementsFile, 'utf8');
    expect(contents).toContain(`--hash=sha256:${DIGEST}`);
    expect(contents).toContain(`--hash=sha256:${OTHER_DIGEST}`);

    expect(wheels).toHaveLength(1);
    expect(wheels[0]?.name).toBe('pydantic-2.13.5-py3-none-any.whl');
    expect(wheels[0]?.sha256).toMatch(/^[0-9a-f]{64}$/);
  });

  it('refuses to invoke pip when the reviewed hashes are missing', () => {
    const destination = makeTemporaryDirectory();
    let invoked = false;
    expect(() =>
      vendor.downloadWheels({
        requirements,
        pythonVersion: '3.12',
        abi: 'cp312',
        wheelTag: 'win_amd64',
        destination,
        pipPython: 'python',
        env: {},
        runner: () => {
          invoked = true;
          return { status: 0, stdout: '', stderr: '' };
        },
      }),
    ).toThrow(/requires the pinned SHA-256 hashes/);
    expect(invoked).toBe(false);
  });

  it('surfaces a non-zero pip exit', () => {
    const destination = makeTemporaryDirectory();
    expect(() =>
      vendor.downloadWheels({
        requirements,
        hashes,
        pythonVersion: '3.12',
        abi: 'cp312',
        wheelTag: 'win_amd64',
        destination,
        pipPython: 'python',
        env: {},
        runner: () => ({ status: 1, stdout: '', stderr: 'no matching distribution' }),
      }),
    ).toThrow(/failed \(exit 1\): no matching distribution/);
  });
});
