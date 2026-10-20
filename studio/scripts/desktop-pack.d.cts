export interface BuilderInvocation {
  argv: string[];
  dir: boolean;
  win?: string[] | boolean;
  mac?: string[] | boolean;
  linux?: string[] | boolean;
  targets: Array<{ platform: string; type: string }>;
  unpackedOnly: boolean;
}

export interface PackagingDecision {
  dirMode: boolean;
  classification: 'development-unpacked-artifact' | 'release';
  releaseEligible: boolean;
  warning: string | null;
}

export function sanitizePublishArguments(argv: string[]): string[];
export function normalizeBuilderInvocation(argv: string[]): BuilderInvocation;
export function isDevelopmentDirectoryBuild(argv: string[]): boolean;
export function decidePackaging(options: {
  argv: string[];
  report: { ready: boolean; missingRouteMethods?: string[]; missingCapabilities?: string[] } | null;
}): PackagingDecision;
export function writeReadinessArtifact(
  report: unknown,
  decision: PackagingDecision,
): string;
export function main(argv?: string[]): Promise<number>;

export const READINESS_REPORT: string;
