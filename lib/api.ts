/**
 * Bridge to the Rust engine.
 *
 * Every call is a Tauri command — the frontend performs no network I/O of its
 * own, which is what lets the app ship with a strict CSP and no network
 * permissions in the webview.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AdjacentSubnet,
  AssistConfig,
  AssistEvent,
  AssistSettings,
  AutoRepeatState,
  CaseSummary,
  DiscoveredNetwork,
  DoctorReport,
  Detection,
  Location,
  NetworkList,
  NetworkProfile,
  UnifiConfig,
  UpdateInfo,
  UpdatePreferences,
  PortInfo,
  PortProfile,
  ScanDiff,
  ScanEvent,
  ScanSnapshot,
  ScanStatus,
  ScanTarget,
  SnapshotSummary,
  TroubleshootingCase,
} from "./types";

/** Next's dev server renders once on the server during export; guard against that. */
export const isDesktop = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface StartScanRequest {
  extraRanges?: string[];
  portProfile?: PortProfile;
  includeDiscoveredSubnets?: boolean;
}

export async function getStatus(): Promise<ScanStatus> {
  return invoke<ScanStatus>("get_status");
}

export async function startScan(request: StartScanRequest = {}): Promise<string> {
  return invoke<string>("start_scan", { request });
}

export async function cancelScan(): Promise<boolean> {
  return invoke<boolean>("cancel_scan");
}

export async function runDoctor(force = false): Promise<DoctorReport> {
  return invoke<DoctorReport>("run_doctor", { force });
}

export async function listSnapshots(limit = 50): Promise<SnapshotSummary[]> {
  return invoke<SnapshotSummary[]>("list_snapshots", { limit });
}

export interface SnapshotWithDiff {
  snapshot: ScanSnapshot;
  diff?: ScanDiff;
}

export async function getSnapshot(
  id: string,
  diff?: "previous" | string,
): Promise<SnapshotWithDiff> {
  return invoke<SnapshotWithDiff>("get_snapshot", { id, diff: diff ?? null });
}

export async function deleteSnapshot(id: string): Promise<boolean> {
  return invoke<boolean>("delete_snapshot", { id });
}

export async function deepScanHost(ip: string): Promise<PortInfo[]> {
  return invoke<PortInfo[]>("deep_scan_host", { ip });
}

export async function setAutoRepeat(
  enabled: boolean,
  intervalMinutes: number,
): Promise<AutoRepeatState> {
  return invoke<AutoRepeatState>("set_auto_repeat", { enabled, intervalMinutes });
}

export async function exportSnapshot(
  id: string,
  format: "json" | "csv",
  path: string,
): Promise<string> {
  return invoke<string>("export_snapshot", { id, format, path });
}

export async function getDataDir(): Promise<string> {
  return invoke<string>("get_data_dir");
}

export async function getAppVersion(): Promise<string> {
  return invoke<string>("get_app_version");
}

/** Subscribes to live scan progress. Returns an unsubscribe function. */
export async function onScanEvent(
  handler: (event: ScanEvent) => void,
): Promise<UnlistenFn> {
  return listen<ScanEvent>("scan://progress", (message) => handler(message.payload));
}

/* -------------------------------------------------------------------- update */

export async function checkForUpdate(force = false): Promise<UpdateInfo | null> {
  return invoke<UpdateInfo | null>("check_for_update", { force });
}

export async function getUpdatePreferences(): Promise<UpdatePreferences> {
  return invoke<UpdatePreferences>("get_update_preferences");
}

export async function skipUpdateVersion(version: string): Promise<void> {
  return invoke("skip_update_version", { version });
}

export async function setUpdateChecksEnabled(enabled: boolean): Promise<void> {
  return invoke("set_update_checks_enabled", { enabled });
}

/* --------------------------------------------------------------------- UniFi */

export async function getUnifiConfig(): Promise<UnifiConfig | null> {
  return invoke<UnifiConfig | null>("get_unifi_config");
}

/**
 * Saves settings. The password is optional so host/site can be edited without
 * re-typing it, and it is never read back to the frontend.
 */
export async function saveUnifiConfig(
  config: UnifiConfig,
  password?: string,
): Promise<void> {
  return invoke("save_unifi_config", { config, password: password ?? null });
}

export async function clearUnifiConfig(): Promise<void> {
  return invoke("clear_unifi_config");
}

/** Whether one diagnostic can run against this controller, and what is missing. */
export interface DiagnosticCoverage {
  finding: string;
  endpoint: string;
  available: boolean;
  missing: string[];
}

/**
 * Asks the controller which diagnostics it can actually support.
 *
 * UniFi releases differ in what they report, so a detector can be correct and
 * still never fire — which makes an empty result ambiguous until this is
 * answered. `saveTo` writes the raw payloads to that path; opt-in, because they
 * contain MACs, hostnames and SSIDs.
 */
export async function checkControllerFields(saveTo?: string): Promise<DiagnosticCoverage[]> {
  return invoke<DiagnosticCoverage[]>("check_controller_fields", {
    saveTo: saveTo ?? null,
  });
}

export async function testUnifiConnection(
  config: UnifiConfig,
  password?: string,
): Promise<string> {
  return invoke<string>("test_unifi_connection", { config, password: password ?? null });
}

/* ----------------------------------------------------------------- assistant */

export async function getAssistConfig(): Promise<AssistSettings> {
  return invoke<AssistSettings>("get_assist_config");
}

/**
 * Saves settings, and the API key when one is supplied.
 *
 * The key is optional so the model or endpoint can be changed without
 * re-typing it, and it is never read back.
 */
export async function saveAssistConfig(
  config: AssistConfig,
  apiKey?: string,
): Promise<void> {
  return invoke("save_assist_config", { config, apiKey: apiKey ?? null });
}

export async function clearAssistConfig(): Promise<void> {
  return invoke("clear_assist_config");
}

/** Confirms the configured model answers, before a diagnosis depends on it. */
export async function testAssistConnection(
  config: AssistConfig,
  apiKey?: string,
): Promise<string> {
  return invoke<string>("test_assist_connection", { config, apiKey: apiKey ?? null });
}

/* --------------------------------------------------------------------- cases */

/**
 * Opens a diagnosis from a symptom in the user's own words.
 *
 * Returns immediately with the case id; the work streams on `assist://progress`
 * and can run for minutes, including a deliberate pause while the user
 * reproduces the problem.
 */
export async function startCase(symptom: string): Promise<string> {
  return invoke<string>("start_case", { symptom });
}

/** Answers the question a case stopped on, and resumes it. */
export async function answerCase(id: string, text: string): Promise<void> {
  return invoke("answer_case", { id, text });
}

/** Stops the running diagnosis. Evidence gathered so far is kept. */
export async function cancelCase(): Promise<boolean> {
  return invoke<boolean>("cancel_case");
}

/** The open case, if any — so a reopened window rejoins one already going. */
export async function getRunningCase(): Promise<string | null> {
  return invoke<string | null>("get_running_case");
}

export async function listCases(limit = 25): Promise<CaseSummary[]> {
  return invoke<CaseSummary[]>("list_cases", { limit });
}

export async function getCase(id: string): Promise<TroubleshootingCase | null> {
  return invoke<TroubleshootingCase | null>("get_case", { id });
}

export async function deleteCase(id: string): Promise<boolean> {
  return invoke<boolean>("delete_case", { id });
}

/** Subscribes to live diagnosis progress. Returns an unsubscribe function. */
export async function onAssistEvent(
  handler: (event: AssistEvent) => void,
): Promise<UnlistenFn> {
  return listen<AssistEvent>("assist://progress", (message) => handler(message.payload));
}

/* ------------------------------------------------------------------ networks */

export async function listNetworks(): Promise<NetworkList> {
  return invoke<NetworkList>("list_networks");
}

/**
 * Fingerprints the current network and reports what to do about it.
 * Changes nothing — the decision is the caller's.
 */
export async function detectNetwork(): Promise<Detection> {
  return invoke<Detection>("detect_network");
}

export async function createNetwork(
  name: string,
  locationId?: string,
): Promise<NetworkProfile> {
  return invoke<NetworkProfile>("create_network", { name, locationId: locationId ?? null });
}

export async function switchNetwork(id: string): Promise<void> {
  return invoke("switch_network", { id });
}

export async function renameNetwork(id: string, name: string): Promise<void> {
  return invoke("rename_network", { id, name });
}

export async function deleteNetwork(id: string): Promise<void> {
  return invoke("delete_network", { id });
}

/** Creates a location, or returns the existing one with that name. */
export async function createLocation(name: string): Promise<Location> {
  return invoke<Location>("create_location", { name });
}

export async function renameLocation(id: string, name: string): Promise<void> {
  return invoke("rename_location", { id, name });
}

/**
 * Removes a location and returns how many networks moved to "no location".
 * The networks themselves, and their scans, are kept.
 */
export async function deleteLocation(id: string): Promise<number> {
  return invoke<number>("delete_location", { id });
}

/** Files a network under a location, or takes it out of one with `undefined`. */
export async function setNetworkLocation(id: string, locationId?: string): Promise<void> {
  return invoke("set_network_location", { id, locationId: locationId ?? null });
}

/** Deletes a network's scans but keeps the network and its settings. */
export async function clearNetworkHistory(id: string): Promise<number> {
  return invoke<number>("clear_network_history", { id });
}

/** Subnets this machine is attached to, for the network picker. */
export async function discoverLocalNetworks(): Promise<DiscoveredNetwork[]> {
  return invoke<DiscoveredNetwork[]>("discover_local_networks");
}

/**
 * Other subnets this machine can demonstrably reach, with the evidence for each.
 * Reads the routing table and the last snapshot; probes nothing.
 */
export async function discoverAdjacentNetworks(): Promise<AdjacentSubnet[]> {
  return invoke<AdjacentSubnet[]>("discover_adjacent_networks");
}

/** Resolves a range to a saved network, creating one when nothing covers it. */
export async function ensureNetworkForRange(
  cidr: string,
  name?: string,
  select = true,
): Promise<NetworkProfile> {
  return invoke<NetworkProfile>("ensure_network_for_range", { cidr, name, select });
}

/** Wipes all settings and histories, then quits the app. */
export async function factoryReset(): Promise<void> {
  return invoke("factory_reset");
}

/** True when the process runs elevated (Unix; always false on Windows). */
export async function isRunningElevated(): Promise<boolean> {
  return invoke<boolean>("is_running_elevated");
}

/** What "Run scan" would sweep right now. */
export async function previewScanTargets(extraRanges: string[]): Promise<ScanTarget[]> {
  return invoke<ScanTarget[]>("preview_scan_targets", { extraRanges });
}

/** Re-fingerprints the active network from where the machine is now. */
export async function refreshNetworkFingerprint(): Promise<void> {
  return invoke("refresh_network_fingerprint");
}
