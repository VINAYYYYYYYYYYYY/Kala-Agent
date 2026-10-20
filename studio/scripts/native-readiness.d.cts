export interface CapabilityRow {
  id: string;
  requiredByPython: boolean;
  presentInNative: boolean;
}

export interface PendingFeature {
  id: string;
  label: string;
}

export interface ReadinessReport {
  schemaVersion: number;
  kind: string;
  generatedAt: string;
  ready: boolean;
  python: {
    root: string;
    sourceSha256: string;
    fileCount: number;
    routeMethodCount: number;
    routeMethods: string[];
    internalRouteMethodCount: number;
    internalRouteMethods: string[];
  };
  native: {
    root: string;
    sourceSha256: string;
    fileCount: number;
    routeMethodCount: number;
    routeMethods: string[];
    rustNativeRouteMethodCount: number;
    rustNativeRouteMethods: string[];
    proxiedRouteMethodCount: number;
    proxiedRouteMethods: string[];
  };
  missingRouteMethods: string[];
  extraNativeRouteMethods: string[];
  capabilities: CapabilityRow[];
  missingCapabilities: string[];
  pendingFeatures: PendingFeature[];
  summary: string;
}

export interface ReadinessSnapshot {
  ready: boolean;
  pythonSourceSha256: string | null;
  nativeSourceSha256: string | null;
  pythonRouteMethodCount: number;
  nativeRouteMethodCount: number;
  missingRouteMethods: string[];
  missingCapabilities: string[];
  pendingFeatures: PendingFeature[];
  summary: string;
  error?: string;
}

export function normalizeRoutePath(routePath: string): string;
export function routeKey(method: string, routePath: string): string;
export interface StudioSourceAnalysis {
  ready: boolean;
  pythonRouteMethods: string[];
  internalRouteMethods: string[];
  internalRouteMethodCount: number;
  nativeRouteMethods: string[];
  rustNativeRouteMethods: string[];
  rustNativeRouteMethodCount: number;
  proxiedRouteMethods: string[];
  proxiedRouteMethodCount: number;
  missingRouteMethods: string[];
  extraNativeRouteMethods: string[];
  capabilities: CapabilityRow[];
  missingCapabilities: string[];
  pendingFeatures: PendingFeature[];
}

export function parsePythonRoutes(source: string): string[];
export function parsePythonRouteSurface(source: string): {
  publicRoutes: string[];
  internalRoutes: string[];
};
export function parseRustRoutes(source: string): string[];
export function parseRustRouteSurface(source: string): {
  routeMethods: string[];
  rustNativeRouteMethods: string[];
  proxiedRouteMethods: string[];
};
export function analyzeStudioSources(options: {
  pythonSource: string;
  nativeSource: string;
}): StudioSourceAnalysis;
export function checkStudioReadiness(options?: {
  pythonStudioRoot?: string;
  nativeCrateRoot?: string;
  generatedAt?: string;
}): ReadinessReport;
export function readinessSnapshot(report: ReadinessReport): ReadinessSnapshot;
export function formatReadinessReport(report: ReadinessReport): string;
export function writeReport(report: ReadinessReport, outputPath: string): string;
export function parseArguments(argv: string[]): {
  reportOnly: boolean;
  json: boolean;
  output: string | null;
  help: boolean;
};
export function main(argv?: string[]): number;

export const STUDIO_ROOT: string;
export const REPO_ROOT: string;
export const PYTHON_STUDIO_ROOT: string;
export const NATIVE_CRATE_ROOT: string;
export const SCHEMA_VERSION: number;
export const KIND: string;
