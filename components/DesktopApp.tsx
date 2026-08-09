"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { ConnectivityPanel } from "./ConnectivityPanel";
import { DeviceTable } from "./DeviceTable";
import { HistoryPanel } from "./HistoryPanel";
import { HostPanel } from "./HostPanel";
import { WifiPanel } from "./WifiPanel";
import { CapabilityBanner, SetupPanel } from "./SetupPanel";
import { UnifiSettings } from "./UnifiSettings";
import { ReconciliationPanel } from "./ReconciliationPanel";
import { UpdateGate } from "./UpdateDialog";
import { NetworkPrompt, NetworkSwitcher } from "./NetworkSwitcher";
import { NetworksPanel } from "./NetworksPanel";
import { NetworkDiscovery } from "./NetworkDiscovery";
import { AdjacentNotice } from "./AdjacentNetworks";
import { SignalMeter } from "./charts";
import {
  Button,
  Card,
  EmptyState,
  formatDuration,
  formatRelativeTime,
  latencyTone,
  LoadingState,
  lossTone,
  signalTone,
  Spinner,
  StatusBadge,
  type StatusTone,
} from "./ui";
import * as api from "@/lib/api";
import {
  isNewDevice,
  isRoutedScan,
  type AdjacentSubnet,
  type Detection,
  type DiscoveredNetwork,
  type NetworkList,
  type NetworkProfile,
  type AutoRepeatState,
  type DoctorReport,
  type PhaseState,
  type PortProfile,
  type ScanSnapshot,
  type ScanTarget,
} from "@/lib/types";

type Section =
  | "overview"
  | "devices"
  | "connectivity"
  | "wifi"
  | "controller"
  | "networks"
  | "host"
  | "history"
  | "setup";

type NavItem = { id: Section; label: string; icon: string };

/* The nav is split by scope: everything in the network group changes meaning
 * when the dropdown above it changes; the system group describes this machine
 * and the app, whichever network is selected. */
const NETWORK_NAV: NavItem[] = [
  { id: "overview", label: "Overview", icon: "◉" },
  { id: "devices", label: "Devices", icon: "▤" },
  { id: "connectivity", label: "Connectivity", icon: "↭" },
  { id: "wifi", label: "Wi-Fi", icon: "≋" },
  { id: "controller", label: "Controller", icon: "⊞" },
  { id: "history", label: "History", icon: "⟲" },
];

const SYSTEM_NAV: NavItem[] = [
  { id: "networks", label: "Networks", icon: "◈" },
  { id: "host", label: "Host & interfaces", icon: "⌂" },
  { id: "setup", label: "Setup & Status", icon: "⚙" },
];

export function DesktopApp() {
  const [snapshot, setSnapshot] = useState<ScanSnapshot | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | null>(null);
  const [doctorLoading, setDoctorLoading] = useState(false);
  const [phases, setPhases] = useState<PhaseState[]>([]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("overview");
  const [refreshToken, setRefreshToken] = useState(0);
  const [autoRepeat, setAutoRepeat] = useState<AutoRepeatState>({
    enabled: false,
    intervalMinutes: 15,
  });
  const [portProfile, setPortProfile] = useState<PortProfile>("standard");
  const [extraRange, setExtraRange] = useState("");
  const [dataDir, setDataDir] = useState<string>();
  const [appVersion, setAppVersion] = useState<string>();
  const [networks, setNetworks] = useState<NetworkProfile[]>([]);
  const [activeNetwork, setActiveNetwork] = useState<string>();
  const [detection, setDetection] = useState<Detection | null>(null);
  const [scanTargets, setScanTargets] = useState<ScanTarget[] | null>(null);
  const [discovery, setDiscovery] = useState<DiscoveredNetwork[] | null>(null);
  /* `snapshot === null` cannot distinguish "nothing scanned yet" from "not read
   * yet", and rendering the empty state during a load is what made the app look
   * like it had lost a network's history until the user navigated away and
   * back. These two say which it is. */
  const [booted, setBooted] = useState(false);
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [adjacent, setAdjacent] = useState<AdjacentSubnet[] | null>(null);
  /* What the running scan is actually doing. Separate from `activeNetwork`,
   * which is only what the user is looking at — conflating them made a scan
   * relabel itself the moment the user browsed elsewhere. */
  const [scanningNetwork, setScanningNetwork] = useState<{
    id?: string;
    name?: string;
  } | null>(null);
  /* A scan that finished under a network the user had navigated away from.
   * Its results are not on screen and must not be forced there, but silently
   * finishing is how a scan looks like it did nothing. */
  const [finishedElsewhere, setFinishedElsewhere] = useState<{
    id: string;
    name?: string;
  } | null>(null);
  /* Subnets the user has said "not now" to. Held for the session only: a scan
   * that keeps seeing one is not new information, but a fresh launch is a fair
   * moment to mention it again. */
  const [dismissedSubnets, setDismissedSubnets] = useState<string[]>([]);

  const mounted = useRef(true);
  /* The event subscription is set up once and must not be torn down on every
   * switch, so it reads the selection through a ref rather than closing over a
   * stale value. */
  const activeNetworkRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    activeNetworkRef.current = activeNetwork;
  }, [activeNetwork]);

  /**
   * Neighbouring subnets the last scan proved reachable.
   *
   * Read here rather than inside the Networks page so the finding can be
   * surfaced where the user is — a suggestion nobody sees until they go looking
   * for it is not a suggestion.
   */
  const refreshAdjacent = useCallback(async () => {
    try {
      const found = await api.discoverAdjacentNetworks();
      if (mounted.current) setAdjacent(found);
    } catch {
      // Informational; a failure leaves the previous list alone.
    }
  }, []);

  const loadSnapshot = useCallback(async (id: string) => {
    setSnapshotLoading(true);
    try {
      const result = await api.getSnapshot(id);
      if (mounted.current) setSnapshot(result.snapshot);
    } catch {
      // A missing snapshot is not an error worth interrupting the user for —
      // but it must not leave the previous network's data on screen either.
      if (mounted.current) setSnapshot(null);
    } finally {
      if (mounted.current) setSnapshotLoading(false);
    }
  }, []);

  /** Reads the selected network's newest snapshot, or clears it if it has none. */
  const reloadSnapshot = useCallback(async () => {
    setSnapshotLoading(true);
    try {
      const status = await api.getStatus();
      if (!mounted.current) return;
      setRunning(status.running);
      setAutoRepeat(status.autoRepeat);
      if (status.phases.length) setPhases(status.phases);
      setScanningNetwork(
        status.running
          ? { id: status.scanningNetworkId, name: status.scanningNetworkName }
          : null,
      );

      if (!status.lastSnapshotId) {
        setSnapshot(null);
        return;
      }
      const result = await api.getSnapshot(status.lastSnapshotId);
      if (mounted.current) setSnapshot(result.snapshot);
    } catch {
      if (mounted.current) setSnapshot(null);
    } finally {
      if (mounted.current) setSnapshotLoading(false);
    }
  }, []);

  const refreshNetworks = useCallback(async (): Promise<NetworkList> => {
    try {
      const list = await api.listNetworks();
      if (mounted.current) {
        setNetworks(list.networks);
        setActiveNetwork(list.active);
      }
      return list;
    } catch {
      // Not fatal: the app still works against whichever network is selected.
      return { networks: [] };
    }
  }, []);

  /* The scan form is a view of the selected network, not a free-text field the
   * user has to remember to keep in sync: re-scanning is the common case, and
   * the range it should use is the one the network is defined by. */
  const syncScanRange = useCallback((list: NetworkProfile[], id?: string) => {
    const profile = list.find((network) => network.id === id);
    setExtraRange(profile?.fingerprint.subnets[0] ?? "");
  }, []);

  const refreshDoctor = useCallback(async (force: boolean) => {
    setDoctorLoading(true);
    try {
      const report = await api.runDoctor(force);
      if (mounted.current) setDoctor(report);
    } catch (err) {
      if (mounted.current) setError(String(err));
    } finally {
      if (mounted.current) setDoctorLoading(false);
    }
  }, []);

  /**
   * Switching swaps the whole history, so everything derived from it has to be
   * re-read. Clearing the snapshot first avoids briefly showing one network's
   * devices under another network's name.
   */
  const changeNetwork = useCallback(
    async (id: string) => {
      setSnapshot(null);
      setSnapshotLoading(true);
      setFinishedElsewhere((pending) => (pending?.id === id ? null : pending));
      try {
        await api.switchNetwork(id);
        setActiveNetwork(id);
        const list = await refreshNetworks();
        syncScanRange(list.networks, id);
        await reloadSnapshot();
        setRefreshToken((token) => token + 1);

        // The doctor's controller check is per network; a stale report would
        // describe the previous network's controller.
        void refreshDoctor(false);
      } catch (err) {
        setError(String(err));
        if (mounted.current) setSnapshotLoading(false);
      }
    },
    [refreshNetworks, reloadSnapshot, refreshDoctor, syncScanRange],
  );

  // Initial load: status, capabilities, latest snapshot.
  useEffect(() => {
    mounted.current = true;

    (async () => {
      // During `next build`'s static export pass, and under `dev:web`, there is
      // no Tauri host and nothing to load — settle immediately so the shell
      // renders its empty states rather than spinning forever.
      if (!api.isDesktop()) {
        setBooted(true);
        return;
      }

      // The doctor probes hardware and, when a controller is configured, a
      // live network service — seconds, tens when something is unreachable.
      // It must never gate showing data that is already on disk, so it runs
      // concurrently and fills its panel in whenever it finishes.
      void refreshDoctor(true);

      await reloadSnapshot();
      if (!mounted.current) return;
      setBooted(true);

      const [dir, version] = await Promise.all([
        api.getDataDir().catch(() => undefined),
        api.getAppVersion().catch(() => undefined),
      ]);
      if (!mounted.current) return;
      setDataDir(dir);
      setAppVersion(version);

      const list = await refreshNetworks();
      if (!mounted.current) return;
      syncScanRange(list.networks, list.active);

      if (list.networks.length === 0) {
        // A true first run (or post-reset): offer the networks this machine
        // can see instead of the single-network detection prompt.
        try {
          const found = await api.discoverLocalNetworks();
          if (mounted.current) setDiscovery(found);
        } catch {
          // Discovery is best-effort; the Networks page can still create one.
        }
        return;
      }

      // Ask which network this is only after the rest has loaded, so the prompt
      // does not race the first render.
      try {
        const result = await api.detectNetwork();
        if (mounted.current && result.kind !== "current" && result.kind !== "noNetwork") {
          setDetection(result);
        }
      } catch {
        // Detection is best-effort; a failure must not block the app.
      }
    })();

    return () => {
      mounted.current = false;
    };
  }, [reloadSnapshot, refreshDoctor, refreshNetworks, syncScanRange]);

  // Re-read after every scan and on every switch: the evidence comes from the
  // selected network's newest snapshot, so both change the answer.
  useEffect(() => {
    if (!api.isDesktop()) return;
    (async () => {
      await refreshAdjacent();
    })();
  }, [refreshAdjacent, refreshToken, activeNetwork]);

  // What "Run scan" would sweep, shown under the button so the target is never
  // a mystery. Debounced because it re-runs as the extra-range field is typed;
  // refreshed on refreshToken so a completed scan or network switch updates it.
  useEffect(() => {
    if (!api.isDesktop()) return;
    // Frozen while a scan runs: the dock shows these as what is *being* swept,
    // and re-reading them on a network switch rewrote a running scan's target
    // to the newly selected network's range.
    if (running) return;
    const timer = setTimeout(async () => {
      try {
        const targets = await api.previewScanTargets(extraRange.trim() ? [extraRange.trim()] : []);
        if (mounted.current) setScanTargets(targets);
      } catch {
        // The preview is informational; a failure just leaves it blank.
      }
    }, 350);
    return () => clearTimeout(timer);
  }, [extraRange, refreshToken, activeNetwork, running]);

  // Live progress. Subscribed once for the app's lifetime, so a run started by
  // the auto-repeat timer streams here too.
  useEffect(() => {
    if (!api.isDesktop()) return;
    let unlisten: (() => void) | undefined;

    api
      .onScanEvent((event) => {
        switch (event.type) {
          case "phase":
            setRunning(true);
            setPhases(event.phases);
            setScanningNetwork({ id: event.networkId, name: event.networkName });
            break;
          case "networkChanged":
            // Only fires when nothing was selected and the scan had to be filed
            // somewhere; follow it so the dropdown and the scan form describe
            // the network the results actually went to.
            setSnapshot(null);
            setActiveNetwork(event.id);
            refreshNetworks().then((list) => syncScanRange(list.networks, event.id));
            break;
          case "done":
            setRunning(false);
            setScanningNetwork(null);
            setRefreshToken((token) => token + 1);
            // The result belongs to the network that was scanned. Loading it
            // while looking at another one asks its store for an id that is not
            // there, and the result silently disappears.
            if (!event.networkId || event.networkId === activeNetworkRef.current) {
              loadSnapshot(event.snapshotId);
            } else {
              setFinishedElsewhere({ id: event.networkId, name: event.networkName });
            }
            // The scan count and last-seen time live in the index, which the
            // switcher renders from — without this it kept reading "0 scans"
            // beside a freshly-filled device list.
            refreshNetworks();
            break;
          case "error":
            setRunning(false);
            setScanningNetwork(null);
            setError(event.message);
            break;
          case "cancelled":
            setRunning(false);
            setScanningNetwork(null);
            break;
          case "warning":
            break;
        }
      })
      .then((fn) => {
        unlisten = fn;
      });

    return () => unlisten?.();
  }, [loadSnapshot, refreshNetworks, syncScanRange]);

  const blocked = doctor?.blocked ?? false;

  const startScan = async () => {
    setError(null);
    setRunning(true);
    try {
      const range = extraRange.trim();
      if (range) {
        // A range nothing covers yet becomes a network of its own before the
        // scan starts, so the results have somewhere of their own to live —
        // and a range an existing network covers selects that network. The
        // field normally already holds the selected network's subnet, in which
        // case this resolves back to it and changes nothing.
        const profile = await api.ensureNetworkForRange(range);
        const list = await refreshNetworks();
        if (mounted.current && profile.id !== activeNetwork) {
          setActiveNetwork(profile.id);
          syncScanRange(list.networks, profile.id);
          // The loaded snapshot belongs to the network just left. Keeping it on
          // screen labelled the page with another network's devices and ranges
          // for the length of the scan — a network showing "0 scans" in the
          // switcher while listing 29 of them.
          setSnapshot(null);
        }
      }
      // Resolves only when the whole scan has finished, and the result is
      // handled by the `done` event rather than here. Loading it here as well
      // was a second path to the same job that did not know which network the
      // user was looking at: after browsing elsewhere mid-scan it asked that
      // network's store for an id belonging to the scanned one, got nothing,
      // and cleared the page.
      await api.startScan({
        portProfile,
        extraRanges: range ? [range] : [],
      });
    } catch (err) {
      setError(String(err));
    } finally {
      // Safe here rather than premature: the command only resolves once the run
      // is over, including the controller step. It is a backstop for the case
      // where the command rejects outright — "a scan is already running" — and
      // no event is emitted at all.
      if (mounted.current) {
        setRunning(false);
        setScanningNetwork(null);
      }
    }
  };

  const updateAutoRepeat = async (enabled: boolean, intervalMinutes: number) => {
    try {
      setAutoRepeat(await api.setAutoRepeat(enabled, intervalMinutes));
    } catch (err) {
      setError(String(err));
    }
  };

  const exportSnapshot = async (format: "json" | "csv") => {
    if (!snapshot) return;
    try {
      const path = await save({
        defaultPath: `network-scan-${snapshot.id}.${format}`,
        filters: [{ name: format.toUpperCase(), extensions: [format] }],
      });
      if (path) await api.exportSnapshot(snapshot.id, format, path);
    } catch (err) {
      setError(String(err));
    }
  };

  // Same rule as the banner: an unconfigured optional integration is not a
  // problem to badge, but anything broken is.
  const problemCount = useMemo(
    () =>
      doctor?.capabilities.filter(
        (c) => c.status === "missing" || (c.status === "degraded" && c.tier !== "optional"),
      ).length ?? 0,
    [doctor],
  );

  const activePhase = phases.find((p) => p.status === "running");
  const activeProfile = networks.find((network) => network.id === activeNetwork);

  /* Everything about the scan is labelled from here, never from the selection.
   * When they differ the user has navigated away mid-scan, which is allowed —
   * and is exactly when saying "scanning <selected network>" would be a lie. */
  const scanLabel =
    scanningNetwork?.name ??
    networks.find((network) => network.id === scanningNetwork?.id)?.name;
  const viewingElsewhere =
    running && !!scanningNetwork?.id && scanningNetwork.id !== activeNetwork;

  /* What the docked strip holds. Rendered only when it has something to say —
   * an empty sticky wrapper would still reserve its padding and push the page
   * down by a band of nothing. */
  const showScanNotice = running && section !== "setup";
  const showFinishedNotice = !running && !!finishedElsewhere && section !== "setup";
  const docked = !!error || showScanNotice || showFinishedNotice;

  const untrackedSubnets = useMemo(
    () => (adjacent ?? []).filter((subnet) => !subnet.alreadyTracked),
    [adjacent],
  );
  const noticeSubnets = untrackedSubnets.filter(
    (subnet) => !dismissedSubnets.includes(subnet.cidr),
  );

  /* Everything network-scoped is unreadable until the first load settles, so one
   * flag covers the lot rather than each panel inventing its own. */
  const contentLoading = !booted || snapshotLoading;

  const renderNavItem = (item: NavItem) => {
    const active = section === item.id;
    const findings =
      (snapshot?.reconciliation?.shadow.length ?? 0) +
      (snapshot?.reconciliation?.identityConflicts.length ?? 0);
    const badgeCount =
      item.id === "setup"
        ? problemCount
        : item.id === "controller"
          ? findings
          : item.id === "networks"
            ? untrackedSubnets.length
            : 0;
    const badge = badgeCount > 0;

    return (
      <button
        key={item.id}
        type="button"
        onClick={() => setSection(item.id)}
        className="mb-0.5 flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors"
        style={{
          background: active ? "var(--surface-raised)" : "transparent",
          color: active ? "var(--text-primary)" : "var(--text-secondary)",
          fontWeight: active ? 600 : 400,
        }}
      >
        <span aria-hidden style={{ color: active ? "var(--series-1)" : "var(--text-muted)" }}>
          {item.icon}
        </span>
        <span className="min-w-0 flex-1 truncate">{item.label}</span>
        {badge && (
          <span
            aria-label={`${badgeCount} item(s) need attention`}
            className="shrink-0 rounded-full px-1.5 text-[10px] font-semibold tabular"
            style={{
              background:
                item.id === "setup" && doctor?.blocked
                  ? "var(--status-critical)"
                  : item.id === "controller"
                    ? "var(--status-critical)"
                    : "var(--status-warning)",
              color: "#000",
            }}
          >
            {badgeCount}
          </span>
        )}
      </button>
    );
  };

  return (
    <div className="flex h-screen overflow-hidden">
      <UpdateGate />

      {discovery && (
        <NetworkDiscovery
          candidates={discovery}
          onDone={async () => {
            // Reopenable from the Networks page, where a snapshot is already on
            // screen — and adopting a subnet here selects a different network,
            // so that snapshot is no longer this network's.
            setDiscovery(null);
            setSnapshot(null);
            const list = await refreshNetworks();
            syncScanRange(list.networks, list.active);
            await reloadSnapshot();
            setRefreshToken((token) => token + 1);
          }}
          onSkip={() => setDiscovery(null)}
        />
      )}

      {detection && !discovery && (
        <NetworkPrompt
          detection={detection}
          onResolved={async () => {
            setDetection(null);
            setSnapshot(null);
            const list = await refreshNetworks();
            setActiveNetwork(list.active);
            syncScanRange(list.networks, list.active);
            await reloadSnapshot();
            setRefreshToken((token) => token + 1);
          }}
          onDismiss={() => setDetection(null)}
        />
      )}

      {/* ------------------------------------------------------------ sidebar */}
      <aside
        className="flex w-56 shrink-0 flex-col border-r"
        style={{ borderColor: "var(--border)", background: "var(--surface-1)" }}
      >
        <div className="px-4 py-4">
          <h1 className="text-sm font-semibold leading-tight">Local Network</h1>
          <h1 className="text-sm font-semibold leading-tight">Diagnostics</h1>
          {snapshot && (
            <p className="mt-1.5 font-mono text-[11px]" style={{ color: "var(--text-muted)" }}>
              {snapshot.host.hostname}
            </p>
          )}
        </div>

        <NetworkSwitcher
          networks={networks}
          activeId={activeNetwork}
          onSwitch={changeNetwork}
          onManage={() => setSection("networks")}
        />

        <nav className="flex-1 overflow-y-auto px-2">
          <p
            className="mb-1 mt-1 px-2.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--text-muted)" }}
          >
            This network
          </p>
          {NETWORK_NAV.map(renderNavItem)}

          <p
            className="mb-1 mt-4 px-2.5 text-[10px] font-semibold uppercase tracking-wider"
            style={{ color: "var(--text-muted)" }}
          >
            System
          </p>
          {SYSTEM_NAV.map(renderNavItem)}
        </nav>

        {/* Scan controls dock to the sidebar rather than floating in a header.
            While a scan runs, the same dock becomes the progress display: it is
            always visible (the sidebar never scrolls away), stage by stage. */}
        <div className="border-t p-3" style={{ borderColor: "var(--border)" }}>
          {running ? (
            <ScanDock phases={phases} targets={scanTargets} networkName={scanLabel} />
          ) : (
            <>
              <Button
                variant="primary"
                onClick={startScan}
                disabled={blocked}
                title={blocked ? "A required capability is unavailable" : undefined}
              >
                Run scan
              </Button>

              <TargetPreview targets={scanTargets} />

              <ScanOptions
                portProfile={portProfile}
                setPortProfile={setPortProfile}
                extraRange={extraRange}
                setExtraRange={setExtraRange}
                autoRepeat={autoRepeat}
                updateAutoRepeat={updateAutoRepeat}
              />
            </>
          )}
        </div>
      </aside>

      {/* ---------------------------------------------------------- main area */}
      <div className="flex min-w-0 flex-1 flex-col">
        <main className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
          {/* Live status stays put while the content scrolls under it.
              A scan takes a minute or two and these lines say what the app is
              doing right now — scrolling a long device list must not be a way
              to lose sight of that. The negative margins let the strip span the
              full width so nothing shows through beside it. */}
          {docked && (
            <div
              /* `pb-px` is load-bearing: without a bottom padding the last
                 notice's margin collapses through the wrapper, so the
                 background stops short and content scrolls visibly through the
                 gap beneath it. */
              className="sticky top-0 z-20 -mx-6 -mt-5 px-6 pb-px pt-5"
              style={{ background: "var(--surface-page)" }}
            >
              {error && (
                <div
                  className="mb-4 flex items-start justify-between gap-3 rounded-lg border px-3 py-2 text-sm"
                  style={{
                    borderColor: "var(--status-critical)",
                    color: "var(--status-critical)",
                    background: "var(--surface-1)",
                  }}
                >
                  <span className="min-w-0">{error}</span>
                  <button type="button" onClick={() => setError(null)} aria-label="Dismiss">
                    ✕
                  </button>
                </div>
              )}

              {showScanNotice && (
                <ScanNotice
                  phases={phases}
                  networkName={scanLabel}
                  viewingName={viewingElsewhere ? activeProfile?.name : undefined}
                />
              )}

              {showFinishedNotice && finishedElsewhere && (
                <div
                  role="status"
                  className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border px-4 py-3"
                  style={{ borderColor: "var(--status-good)", background: "var(--surface-1)" }}
                >
                  <p className="min-w-0 text-sm">
                    The scan of{" "}
                    <strong>{finishedElsewhere.name ?? "the other network"}</strong> finished. Its
                    results are under that network, not this one.
                  </p>
                  <div className="flex shrink-0 items-center gap-2">
                    <Button onClick={() => setFinishedElsewhere(null)}>Dismiss</Button>
                    <Button variant="primary" onClick={() => changeNetwork(finishedElsewhere.id)}>
                      View results
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}

          {section !== "setup" && (
            <CapabilityBanner report={doctor} onOpenSetup={() => setSection("setup")} />
          )}

          {/* Where the user is, not where the feature lives. A scan that saw a
              device on an untracked subnet has discovered something. */}
          {!running && section !== "networks" && section !== "setup" && (
            <AdjacentNotice
              subnets={noticeSubnets}
              onOpen={() => setSection("networks")}
              onDismiss={() =>
                setDismissedSubnets((current) => [
                  ...current,
                  ...noticeSubnets.map((subnet) => subnet.cidr),
                ])
              }
            />
          )}

          {section === "setup" ? (
            /* System scope only — the per-network controller settings live on
               the Controller page, beside the data they produce. */
            <SetupPanel
              report={doctor}
              onRecheck={() => refreshDoctor(true)}
              loading={doctorLoading}
              dataDir={dataDir}
              appVersion={appVersion}
            />
          ) : section === "networks" ? (
            /* Handled before the no-snapshot case: managing networks must work
               even when the selected one has never been scanned. */
            <NetworksPanel
              networks={networks}
              activeId={activeNetwork}
              adjacent={adjacent}
              onReloadAdjacent={refreshAdjacent}
              onDiscover={async () => {
                try {
                  setDiscovery(await api.discoverLocalNetworks());
                } catch (err) {
                  setError(String(err));
                }
              }}
              onChanged={async () => {
                // Switching or deleting changes which history is current, so
                // drop the loaded snapshot rather than showing a stale one.
                setSnapshot(null);
                const list = await refreshNetworks();
                syncScanRange(list.networks, list.active);
                setActiveNetwork(list.active);
                await reloadSnapshot();
                setRefreshToken((token) => token + 1);
              }}
            />
          ) : section === "controller" ? (
            /* Also before the no-snapshot case: the controller can be
               configured before the first scan. Keyed by network so a switch
               reloads this network's settings instead of showing the last
               one's — the controller is per network. */
            <div className="space-y-4">
              <ReconciliationPanel
                reconciliation={snapshot?.reconciliation}
                unifi={snapshot?.unifi}
                loading={contentLoading}
                fetching={running && controllerPhaseIsRunning(phases)}
                configured={activeProfile?.hasUnifi ?? false}
                networkName={activeProfile?.name}
              />
              <UnifiSettings
                key={activeNetwork ?? "none"}
                onChanged={() => refreshDoctor(true)}
              />
            </div>
          ) : contentLoading ? (
            <Card>
              <LoadingState
                title="Loading this network’s latest scan…"
                hint={
                  activeProfile
                    ? `Reading the stored history for “${activeProfile.name}”.`
                    : "Reading the stored scan history."
                }
              />
            </Card>
          ) : !snapshot ? (
            <Card>
              {running && !viewingElsewhere ? (
                /* The notice above already says what is happening; telling the
                   user to run a scan while one is running would contradict it. */
                <LoadingState
                  title={
                    scanLabel
                      ? `First scan of ${scanLabel} in progress…`
                      : "First scan in progress…"
                  }
                  hint="Devices, connectivity and Wi-Fi appear here as soon as it finishes."
                />
              ) : (
                <EmptyState
                  title="No scan yet"
                  hint={
                    blocked
                      ? "Scanning is unavailable until the required capability in Setup & Status is resolved."
                      : "Run a scan to discover devices, measure connectivity and survey Wi-Fi. A full sweep of a /24 takes roughly 30–90 seconds."
                  }
                />
              )}
            </Card>
          ) : (
            <>
              {section === "overview" && (
                <Overview snapshot={snapshot} onNavigate={setSection} />
              )}
              {section === "devices" && (
                <Card
                  title={`Devices — ${snapshot.devices.length}`}
                  subtitle={
                    /* ARP is link-local, so a network reached only by routing
                       can never yield a MAC or a vendor. Said once here rather
                       than flagged on every row as though it were a surprise. */
                    `Scanned ${snapshot.host.scanTargets
                      .map((t) => t.cidr)
                      .join(", ")} in ${formatDuration(snapshot.durationMs)}${
                      isRoutedScan(snapshot)
                        ? " · reached by routing, so hardware addresses and vendors are unavailable"
                        : ""
                    }`
                  }
                >
                  <DeviceTable snapshot={snapshot} />
                </Card>
              )}
              {section === "connectivity" && (
                <ConnectivityPanel connectivity={snapshot.connectivity} />
              )}
              {section === "wifi" && <WifiPanel wifi={snapshot.wifi} />}
              {section === "host" && <HostPanel snapshot={snapshot} />}
              {section === "history" && (
                <HistoryPanel
                  currentId={snapshot.id}
                  onSelect={loadSnapshot}
                  refreshToken={refreshToken}
                  onExport={exportSnapshot}
                  onRevealDataDir={
                    dataDir ? () => revealItemInDir(dataDir).catch(() => {}) : undefined
                  }
                />
              )}
            </>
          )}
        </main>

        {/* ---------------------------------------------------------- status bar */}
        <footer
          className="flex shrink-0 items-center justify-between gap-4 border-t px-4 py-1.5 text-[11px]"
          style={{ borderColor: "var(--border)", background: "var(--surface-1)" }}
        >
          <div className="flex min-w-0 items-center gap-3">
            <span className="flex items-center gap-1.5">
              <span
                aria-hidden
                style={{
                  color: running
                    ? "var(--series-1)"
                    : blocked
                      ? "var(--status-critical)"
                      : "var(--status-good)",
                }}
              >
                ●
              </span>
              <span style={{ color: "var(--text-secondary)" }}>
                {running
                  ? activePhase
                    ? `${activePhase.label}${
                        activePhase.progress
                          ? ` ${activePhase.progress.current}/${activePhase.progress.total}`
                          : ""
                      }`
                    : "Scanning…"
                  : blocked
                    ? "Scanning unavailable"
                    : "Ready"}
              </span>
            </span>

            {snapshot && !running && (
              <span className="truncate" style={{ color: "var(--text-muted)" }}>
                {snapshot.devices.length} devices · last scan{" "}
                {formatRelativeTime(snapshot.startedAt)}
              </span>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-3" style={{ color: "var(--text-muted)" }}>
            {autoRepeat.enabled && autoRepeat.nextRunAt && !running && (
              <span>
                next run {formatRelativeTime(autoRepeat.nextRunAt).replace(" ago", " from now")}
              </span>
            )}
            {doctor && (
              <button
                type="button"
                onClick={() => setSection("setup")}
                className="hover:underline"
                style={{
                  color: doctor.blocked
                    ? "var(--status-critical)"
                    : problemCount > 0
                      ? "var(--status-warning)"
                      : "var(--text-muted)",
                }}
              >
                {problemCount > 0 ? `${problemCount} capability issue(s)` : "All checks passed"}
              </button>
            )}
            {appVersion && <span className="tabular">v{appVersion}</span>}
          </div>
        </footer>
      </div>
    </div>
  );
}

/** True while the controller step of a run is in flight. */
function controllerPhaseIsRunning(phases: PhaseState[]): boolean {
  return phases.some((phase) => phase.phase === "controller" && phase.status === "running");
}

/**
 * In-content notice while a scan runs.
 *
 * The sidebar dock has the detail; this exists because the content area is
 * where the user is actually looking, and a page that shows the previous scan
 * with no explanation is indistinguishable from one that has stopped updating.
 */
function ScanNotice({
  phases,
  networkName,
  viewingName,
}: {
  phases: PhaseState[];
  /** The network being scanned — never the one merely selected. */
  networkName?: string;
  /** Set only when the user has navigated to a different network mid-scan. */
  viewingName?: string;
}) {
  const active = phases.find((phase) => phase.status === "running");
  const controller = controllerPhaseIsRunning(phases);

  const where = networkName ? `Scanning ${networkName}` : "Scanning";
  const detail = controller
    ? "Matching this scan's devices against the controller. A controller that is slow to answer can add a minute or more."
    : active
      ? `${active.label}${
          active.progress ? ` — ${active.progress.current} of ${active.progress.total}` : ""
        }.`
      : "Starting.";

  return (
    <div
      role="status"
      aria-live="polite"
      className="mb-4 flex items-center gap-3 rounded-xl border px-4 py-3"
      style={{ borderColor: "var(--border-strong)", background: "var(--surface-1)" }}
    >
      <Spinner />
      <div className="min-w-0 text-sm">
        <p className="font-medium">
          {controller ? `${where} — contacting the UniFi controller…` : `${where}…`}
        </p>
        <p className="mt-0.5 text-xs" style={{ color: "var(--text-secondary)" }}>
          {detail}{" "}
          {viewingName
            ? `You are viewing ${viewingName}; the results will appear under ${
                networkName ?? "the scanned network"
              }.`
            : "This page updates when the scan completes."}
        </p>
      </div>
    </div>
  );
}

/**
 * What "Run scan" will sweep (idle) or is sweeping (running). The default is
 * the selected network's subnet, which was previously never stated anywhere —
 * an empty range field made it look like the scan had no defined target.
 */
function TargetPreview({ targets }: { targets: ScanTarget[] | null }) {
  if (targets === null) return null;
  if (targets.length === 0) {
    return (
      <p className="mt-2 text-[11px]" style={{ color: "var(--status-warning)" }}>
        No scannable network detected.
      </p>
    );
  }
  const hosts = targets.reduce((sum, target) => sum + target.hostCount, 0);
  return (
    <p className="mt-2 text-[11px]" style={{ color: "var(--text-secondary)" }}>
      Scans{" "}
      <span className="font-mono tabular">{targets.map((t) => t.cidr).join(" · ")}</span>{" "}
      ({hosts} addresses)
    </p>
  );
}

/**
 * The scan-in-progress view of the sidebar dock.
 *
 * It lives where the Run-scan button was, so progress is always on screen —
 * the sidebar never scrolls — and each stage gets its own bar rather than one
 * line that scrolls out of the main area.
 */
function ScanDock({
  phases,
  targets,
  networkName,
}: {
  phases: PhaseState[];
  targets: ScanTarget[] | null;
  /** The network being scanned, pinned for the run. */
  networkName?: string;
}) {
  return (
    <div>
      <Button variant="danger" onClick={() => api.cancelScan()}>
        Cancel scan
      </Button>

      {networkName && (
        <p className="mt-2 truncate text-[11px] font-medium" title={networkName}>
          {networkName}
        </p>
      )}

      {targets !== null && targets.length > 0 && (
        <p className="mt-1 text-[11px]" style={{ color: "var(--text-secondary)" }}>
          Scanning{" "}
          <span className="font-mono tabular">{targets.map((t) => t.cidr).join(" · ")}</span>
        </p>
      )}

      {phases.length === 0 ? (
        <p
          className="animate-pulse-soft mt-3 text-[11px]"
          style={{ color: "var(--text-secondary)" }}
        >
          Starting…
        </p>
      ) : (
        <ol className="mt-3 space-y-2">
          {phases.map((phase) => {
            const indeterminate = phase.status === "running" && !phase.progress;
            const percent =
              phase.status === "done"
                ? 100
                : phase.status === "running" && phase.progress
                  ? (phase.progress.current / Math.max(1, phase.progress.total)) * 100
                  : 0;
            return (
              <li key={phase.phase}>
                <div className="flex items-baseline justify-between gap-2">
                  <span
                    className="min-w-0 flex-1 truncate text-[11px]"
                    style={{
                      color:
                        phase.status === "pending" || phase.status === "skipped"
                          ? "var(--text-muted)"
                          : "var(--text-secondary)",
                    }}
                  >
                    {phase.label}
                  </span>
                  <span
                    className="shrink-0 text-[10px] tabular"
                    style={{
                      color:
                        phase.status === "error" ? "var(--status-critical)" : "var(--text-muted)",
                    }}
                  >
                    {phase.status === "skipped"
                      ? "skipped"
                      : phase.status === "error"
                        ? "error"
                        : phase.status === "done"
                          ? "✓"
                          : phase.status === "running" && phase.progress
                            ? `${phase.progress.current}/${phase.progress.total}`
                            : ""}
                  </span>
                </div>
                <div
                  className="mt-1 h-1 w-full overflow-hidden rounded-full"
                  style={{ background: "var(--gridline)" }}
                  role="progressbar"
                  aria-label={phase.label}
                  aria-valuenow={indeterminate ? undefined : Math.round(percent)}
                  aria-valuemin={0}
                  aria-valuemax={100}
                >
                  <div
                    className={
                      indeterminate
                        ? "animate-pulse-soft h-full rounded-full"
                        : "h-full rounded-full transition-[width] duration-300"
                    }
                    style={{
                      width: indeterminate ? "100%" : `${percent}%`,
                      opacity: indeterminate ? 0.45 : 1,
                      background:
                        phase.status === "error"
                          ? "var(--status-critical)"
                          : phase.status === "done"
                            ? "var(--status-good)"
                            : "var(--series-1)",
                    }}
                  />
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </div>
  );
}

/** The idle view of the sidebar dock: port profile, extra range, auto-repeat. */
function ScanOptions({
  portProfile,
  setPortProfile,
  extraRange,
  setExtraRange,
  autoRepeat,
  updateAutoRepeat,
}: {
  portProfile: PortProfile;
  setPortProfile: (profile: PortProfile) => void;
  extraRange: string;
  setExtraRange: (range: string) => void;
  autoRepeat: AutoRepeatState;
  updateAutoRepeat: (enabled: boolean, intervalMinutes: number) => void;
}) {
  return (
    <>
      <label className="mt-3 block text-[11px]" style={{ color: "var(--text-secondary)" }}>
        Ports
        <select
          value={portProfile}
          onChange={(e) => setPortProfile(e.target.value as PortProfile)}
          className="mt-1 w-full rounded-lg border px-2 py-1 text-xs"
          style={{
            borderColor: "var(--border-strong)",
            background: "var(--surface-raised)",
            color: "var(--text-primary)",
          }}
        >
          <option value="quick">Quick (~15)</option>
          <option value="standard">Standard (~40)</option>
          <option value="deep">Deep (~150)</option>
        </select>
      </label>

      <label className="mt-2 block text-[11px]" style={{ color: "var(--text-secondary)" }}>
        Scan range
        <input
          type="text"
          value={extraRange}
          onChange={(e) => setExtraRange(e.target.value)}
          placeholder="192.168.1.0/24"
          title="Pre-filled with the selected network's subnet, so re-scanning covers the same ground. A range no saved network covers becomes a new network when the scan starts."
          className="mt-1 w-full rounded-lg border px-2 py-1 text-xs"
          style={{
            borderColor: "var(--border-strong)",
            background: "var(--surface-raised)",
            color: "var(--text-primary)",
          }}
        />
      </label>

      <label
        className="mt-2 flex items-center gap-1.5 text-[11px]"
        style={{ color: "var(--text-secondary)" }}
      >
        <input
          type="checkbox"
          checked={autoRepeat.enabled}
          onChange={(e) => updateAutoRepeat(e.target.checked, autoRepeat.intervalMinutes)}
        />
        Repeat every
        <select
          value={autoRepeat.intervalMinutes}
          onChange={(e) => updateAutoRepeat(autoRepeat.enabled, Number(e.target.value))}
          className="rounded border px-1 py-0.5 text-[11px]"
          style={{
            borderColor: "var(--border-strong)",
            background: "var(--surface-raised)",
            color: "var(--text-primary)",
          }}
        >
          <option value={5}>5m</option>
          <option value={15}>15m</option>
          <option value={60}>60m</option>
        </select>
      </label>
    </>
  );
}

function Overview({
  snapshot,
  onNavigate,
}: {
  snapshot: ScanSnapshot;
  onNavigate: (section: Section) => void;
}) {
  const newDevices = snapshot.devices.filter((d) => isNewDevice(d, snapshot)).length;
  const gateway = snapshot.connectivity.gateway;
  const wifi = snapshot.wifi.status === "ok" ? snapshot.wifi.data : undefined;
  const openPorts = snapshot.devices.reduce((sum, d) => sum + d.ports.length, 0);

  const tiles: {
    label: string;
    value: string;
    tone?: StatusTone;
    detail?: string;
    extra?: React.ReactNode;
    section?: Section;
  }[] = [
    {
      label: "Devices found",
      value: String(snapshot.devices.length),
      detail: newDevices > 0 ? `${newDevices} new since last scan` : "no new devices",
      tone: newDevices > 0 ? "warning" : "good",
      section: "devices",
    },
    {
      label: "Gateway latency",
      value: gateway?.avgMs !== undefined ? `${gateway.avgMs.toFixed(1)} ms` : "—",
      tone: latencyTone(gateway?.avgMs, true),
      detail: gateway
        ? `${gateway.lossPercent.toFixed(0)}% loss · ${gateway.jitterMs?.toFixed(1) ?? "?"} ms jitter`
        : undefined,
      section: "connectivity",
    },
    {
      label: "Internet",
      value: snapshot.connectivity.wanReachable ? "Reachable" : "Down",
      tone: snapshot.connectivity.wanReachable ? "good" : "critical",
      detail: snapshot.connectivity.publicIp,
      section: "connectivity",
    },
    {
      label: "Wi-Fi signal",
      value: wifi?.current ? `${wifi.current.signal}%` : "—",
      tone: wifi?.current ? signalTone(wifi.current.signal) : "neutral",
      detail: wifi?.current ? `${wifi.current.ssid} · ch ${wifi.current.channel}` : "not connected",
      extra: wifi?.current ? <SignalMeter percent={wifi.current.signal} /> : undefined,
      section: "wifi",
    },
    {
      label: "Open ports",
      value: String(openPorts),
      detail: `across ${snapshot.devices.filter((d) => d.ports.length > 0).length} hosts`,
      tone: "neutral",
      section: "devices",
    },
    {
      label: "Packet loss",
      value: gateway ? `${gateway.lossPercent.toFixed(0)}%` : "—",
      tone: gateway ? lossTone(gateway.lossPercent) : "neutral",
      detail: "to gateway",
      section: "connectivity",
    },
  ];

  return (
    <div className="space-y-4">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {tiles.map((tile) => (
          <button
            key={tile.label}
            type="button"
            onClick={() => tile.section && onNavigate(tile.section)}
            className="rounded-xl border p-3 text-left transition-colors hover:bg-[var(--surface-raised)]"
            style={{ borderColor: "var(--border)", background: "var(--surface-1)" }}
          >
            <p className="text-xs" style={{ color: "var(--text-secondary)" }}>
              {tile.label}
            </p>
            <p className="mt-1 flex items-center gap-2 text-2xl font-semibold">
              {tile.value}
              {tile.extra}
            </p>
            {tile.detail && (
              <p
                className="mt-1 flex flex-wrap items-center gap-1.5 text-xs"
                style={{ color: "var(--text-muted)" }}
              >
                {tile.tone && tile.tone !== "neutral" && (
                  <span
                    aria-hidden
                    style={{
                      color:
                        tile.tone === "good"
                          ? "var(--status-good)"
                          : tile.tone === "warning"
                            ? "var(--status-warning)"
                            : tile.tone === "serious"
                              ? "var(--status-serious)"
                              : "var(--status-critical)",
                      fontSize: "0.7em",
                    }}
                  >
                    ●
                  </span>
                )}
                {tile.detail}
              </p>
            )}
          </button>
        ))}
      </div>

      {snapshot.warnings.length > 0 && (
        <Card title={`Scan notes (${snapshot.warnings.length})`}>
          <ul className="space-y-1.5">
            {snapshot.warnings.map((warning, i) => (
              <li key={i} className="flex items-start gap-2 text-sm">
                <span aria-hidden style={{ color: "var(--status-warning)" }}>
                  ▲
                </span>
                <span style={{ color: "var(--text-secondary)" }}>{warning}</span>
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Card title="Scan summary">
        <dl className="grid gap-x-8 gap-y-1 sm:grid-cols-2">
          {[
            ["Started", new Date(snapshot.startedAt).toLocaleString()],
            ["Duration", formatDuration(snapshot.durationMs)],
            ["Port profile", snapshot.config.portProfile],
            ["Ranges", snapshot.host.scanTargets.map((t) => t.cidr).join(", ")],
            ["Platform", snapshot.host.platform],
            ["Engine", `v${snapshot.host.appVersion}`],
          ].map(([label, value]) => (
            <div key={label} className="flex flex-wrap gap-2 py-1">
              <dt className="w-32 shrink-0 text-xs" style={{ color: "var(--text-secondary)" }}>
                {label}
              </dt>
              <dd className="min-w-0 flex-1 text-sm break-words">{value}</dd>
            </div>
          ))}
        </dl>
      </Card>

      {snapshot.devices.some((d) => isNewDevice(d, snapshot)) && (
        <Card title="New devices in this scan">
          <ul className="space-y-1.5">
            {snapshot.devices
              .filter((d) => isNewDevice(d, snapshot))
              .map((device) => (
                <li key={device.ip} className="flex flex-wrap items-center gap-2 text-sm">
                  <StatusBadge tone="warning" label="New" />
                  <span className="font-medium">{device.displayName}</span>
                  <span className="font-mono text-xs" style={{ color: "var(--text-muted)" }}>
                    {device.ip}
                  </span>
                  {device.vendor && (
                    <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
                      {device.vendor}
                    </span>
                  )}
                </li>
              ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
