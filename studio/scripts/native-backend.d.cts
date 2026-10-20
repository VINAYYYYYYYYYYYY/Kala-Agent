import type { ReadinessSnapshot } from './native-readiness.cjs';

export interface NativePaths {
  studioRoot: string;
  stageDir: string;
  binaryName: string;
  binaryPath: string;
  staticDir: string;
  indexFile: string;
  metadataPath: string;
  libDir: string;
  adapterDir: string;
}

export interface BundledSharedLibrary {
  name: string;
  source: string;
  requiredBy: string;
  bytes: number;
  sha256: string;
}

export interface FreeCadVendorWheel {
  name: string;
  bytes: number;
  sha256: string;
}

export interface FreeCadVendorAbi {
  python: string;
  abi: string;
  wheelPlatform: string;
  wheels: FreeCadVendorWheel[];
  packages: string[];
  prunedBytecodeEntries?: number;
  fileCount: number;
  sourceDigest: string;
}

export interface FreeCadVendorManifest {
  schemaVersion: number;
  kind?: string;
  generatedBy?: string;
  generatedAt?: string;
  layout?: string;
  platform?: string;
  wheelPlatform?: string;
  supportedAbis: string[];
  freecad?: {
    executable: string | null;
    version: string | null;
    python: string | null;
    abi: string | null;
    soabi?: string | null;
    cacheTag?: string | null;
    platform?: string | null;
    detected: boolean;
    abiSupported?: boolean | null;
  };
  abis: Record<string, FreeCadVendorAbi>;
  requirements?: string[];
  kalaAgent?: { path: string; source: string; files: number; treeSha256: string };
  verification?: { pydantic: string; protocol: string; python?: string; cacheTag?: string } | null;
  verificationSkippedReason?: string | null;
  detectedUnsupported?: {
    executable: string;
    version: string | null;
    python: string;
    abi: string;
  } | null;
  fileCount?: number;
  sourceDigest?: string;
}

export interface BundledFreeCadVendor {
  state: 'vendored' | 'absent';
  manifest?: string;
  layout?: string;
  supportedAbis?: string[];
  wheelPlatform?: string;
  freecad?: FreeCadVendorManifest['freecad'];
  requirements?: string[];
  abis?: Record<string, {
    python: string;
    wheels: FreeCadVendorWheel[];
    packages: string[];
    fileCount: number;
    sourceDigest: string;
  }>;
  fileCount?: number;
  sourceDigest?: string;
  verification?: FreeCadVendorManifest['verification'];
  verificationSkippedReason?: string | null;
}

export interface BundledFreeCadAdapter {
  path: string;
  srcPath: string;
  modPath: string;
  fileCount: number;
  pythonFileCount: number;
  treeSha256: string;
  vendor?: BundledFreeCadVendor;
}

export interface NativeBuildMetadata {
  schemaVersion: number;
  stageOwner: string;
  stageDir: string;
  managedBy: string;
  package: string;
  packageVersion: string;
  profile: string;
  features: string | null;
  nativeStep: boolean;
  target: string | null;
  platform: NodeJS.Platform;
  binary: string;
  binaryBytes: number;
  binarySha256: string;
  sourceBinarySha256?: string;
  staticDistSha256: string;
  sharedLibraryMode?: 'ldd' | 'explicit' | 'none';
  sharedLibraries?: BundledSharedLibrary[];
  sharedLibraryMissing?: Array<{ name: string; requiredBy: string; reason?: string }>;
  rpath?: string | null;
  patchelf?: string | null;
  freeCadAdapter?: BundledFreeCadAdapter;
  sourceParity: ReadinessSnapshot;
  cargo: string | null;
  rustc: string | null;
  gitCommit: string | null;
  createdBy: string;
}

export interface FreeCadVendorOptions {
  mode?: 'skip' | 'auto' | 'require';
  pinsFile?: string;
  executable?: string;
  platform?: NodeJS.Platform;
  env?: NodeJS.ProcessEnv;
  pipPython?: string;
  pythonVersion?: string;
  abi?: string;
  wheelTag?: string;
  verify?: boolean;
}

export interface StageOptions {
  stageDir?: string;
  distDir?: string;
  binaryPath?: string;
  target?: string;
  platform?: NodeJS.Platform;
  noDefaultFeatures?: boolean;
  features?: string;
  studioRoot?: string;
  backendRoot?: string;
  targetDir?: string;
  repoRoot?: string;
  occtLibDir?: string;
  freecadVendor?: FreeCadVendorOptions;
  stdio?: 'inherit' | 'ignore';
  onBeforeInstall?: (context: StageInstallContext) => void | Promise<void>;
  onBeforeBackupRemoval?: (context: StageBackupContext) => void | Promise<void>;
}

export interface StageState {
  exists: boolean;
  identity: string | null;
  stateSha256: string | null;
  nonempty: boolean;
}

export interface StageInstallContext {
  destination: string;
  candidate: string;
  stagePaths: NativePaths;
  originalState: StageState;
}

export interface StageBackupContext {
  destination: string;
  backup: string;
  stagePaths: NativePaths;
}

export interface StageResult {
  paths: NativePaths;
  metadata: NativeBuildMetadata;
}

export function nativeBinaryName(options?: { platform?: NodeJS.Platform; target?: string }): string;
export function cargoTargetDirectory(options?: { backendRoot?: string; targetDir?: string }): string;
export function cargoBinaryPath(options?: {
  backendRoot?: string;
  targetDir?: string;
  target?: string;
  profile?: string;
}): string;
export function buildCargoArguments(options?: StageOptions): string[];
export function nativePaths(options?: {
  studioRoot?: string;
  stageDir?: string;
  platform?: NodeJS.Platform;
  target?: string;
}): NativePaths;
export function assertSafeStageDir(
  stageDir: string,
  options?: { studioRoot?: string; repoRoot?: string },
): string;
export function assertNoSymlinkAncestors(target: string, label: string): void;
export function assertSourceTreeNoSymlinks(root: string, label: string): string;
export function assertRegularSourceFile(filePath: string, label: string): string;
export function directoryStateDigest(root: string): string;
export function captureStageState(stageDir: string): StageState;
export function sameStageState(left: StageState, right: StageState): boolean;
export function assertStageStateUnchanged(
  original: StageState,
  current: StageState,
  destination: string,
): void;
export function fileSha256(filePath: string): Promise<string>;
export function treeSha256(root: string): Promise<string>;
export function binaryContainsNativeStep(filePath: string): boolean;
export function validateNativeTree(
  paths: NativePaths,
  options?: { requireBundles?: boolean },
): true;
export function verifyStageIntegrity(
  paths: NativePaths,
  options?: { requireBundles?: boolean },
): Promise<NativeBuildMetadata>;
export function stageFreeCadAdapter(options: {
  repoRoot?: string;
  destination: string;
  vendor?: FreeCadVendorOptions;
}): {
  root: string;
  sourceRoot: string;
  modRoot: string;
  fileCount: number;
  pythonFileCount: number;
  vendor: FreeCadVendorManifest | null;
};
export function validateBundledFreeCadAdapter(paths: NativePaths): {
  root: string;
  fileCount: number;
  pythonFileCount: number;
  vendor: FreeCadVendorManifest | null;
};
export function validateFreeCadVendorManifest(adapterRoot: string): FreeCadVendorManifest | null;
export function isFreeCadVendorPath(relative: string): boolean;
export function assertFreeCadAdapterBundled(paths: NativePaths): {
  root: string;
  fileCount: number;
  pythonFileCount: number;
  vendor: FreeCadVendorManifest | null;
};
export function parseLddOutput(output: string): {
  entries: Array<{ name: string; path: string }>;
  missing: string[];
};
export function isOcctSharedLibraryName(name: string): boolean;
export function isSystemLibraryPath(path: string): boolean;
export function collectLddSharedLibraries(binaryPath: string): Promise<{
  mode: 'ldd';
  libraries: Array<{ name: string; sourcePath: string; requiredBy: string }>;
  missing: Array<{ name: string; requiredBy: string; reason?: string }>;
  lddStatus: number | null;
  note: string | null;
}>;
export function listExplicitLibraries(directory: string, platform: NodeJS.Platform): {
  mode: 'explicit';
  libraries: Array<{ name: string; sourcePath: string; requiredBy: string }>;
  missing: never[];
  lddStatus: null;
  note: null;
};
export function stageSharedLibraries(options: {
  binaryPath: string;
  destination: string;
  platform?: NodeJS.Platform;
  occtLibDir?: string;
}): Promise<{
  mode: 'ldd' | 'explicit' | 'none';
  libraries: BundledSharedLibrary[];
  missing: Array<{ name: string; requiredBy: string; reason?: string }>;
  targetDir: string | null;
  lddStatus?: number | null;
  note?: string | null;
}>;
export function patchElfRpaths(options: {
  binaryPath: string;
  libDir: string;
  libraries: BundledSharedLibrary[];
  platform?: NodeJS.Platform;
}): {
  patchelf: string | null;
  binaryRpath: string | null;
  libraryRpath: string | null;
  note: string;
};
export function assertStageSourceParity(
  metadata: NativeBuildMetadata,
  currentReport: import('./native-readiness.cjs').ReadinessReport,
): ReadinessSnapshot;
export function assertStageUiMatches(
  metadata: NativeBuildMetadata,
  options?: { distDir?: string },
): Promise<string>;
export function validateStageOwnership(
  stageDir: string,
  options?: { studioRoot?: string },
): NativeBuildMetadata;
export function currentSourceParity(): ReadinessSnapshot;
export function stageNativeBackend(options?: StageOptions): Promise<StageResult>;
export function buildAndStageNative(options?: StageOptions): Promise<StageResult>;
export function parseArguments(argv: string[]): {
  noBuild: boolean;
  stageDir: string;
  distDir: string;
  target?: string;
  noDefaultFeatures: boolean;
  features?: string;
  binaryPath?: string;
  occtLibDir?: string;
  vendorFreeCad: boolean;
  requireFreeCadVendor: boolean;
  freeCadPython?: string;
  freeCadPythonVersion?: string;
  freeCadAbi?: string;
  vendorPython?: string;
  help: boolean;
};
export function main(argv?: string[]): Promise<number>;

export const STUDIO_ROOT: string;
export const REPO_ROOT: string;
export const BACKEND_ROOT: string;
export const DEFAULT_STAGE_DIR: string;
export const DEFAULT_DIST_DIR: string;
export const METADATA_NAME: string;
export const STATIC_DIR_NAME: string;
export const SHARED_LIB_DIR_NAME: string;
export const ADAPTER_DIR_NAME: string;
export const FREECAD_SITE_PACKAGES_PREFIX: string;
export const FREECAD_PYTHON_ABIS: string[];
export const STAGE_OWNER: string;
export const HELPER_ID: string;
export const ORIGIN_LIB_RPATH: string;
export const ORIGIN_RPATH: string;
export const FREECAD_VENDOR_MANIFEST: string;
