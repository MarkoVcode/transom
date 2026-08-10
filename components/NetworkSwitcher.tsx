"use client";

import { useEffect, useRef, useState } from "react";
import { Button, Icon } from "./ui";
import * as api from "@/lib/api";
import { groupByLocation } from "@/lib/networks";
import type { Detection, Location, NetworkProfile } from "@/lib/types";

/** Sentinel for the "create one" option in a location picker. Not an id. */
export const NEW_LOCATION = "__new";

/**
 * Picks which network's history is being viewed.
 *
 * Sits at the top of the sidebar because it changes the meaning of everything
 * below it: switching swaps the entire scan history, diff baseline and
 * controller configuration.
 */
export function NetworkSwitcher({
  networks,
  locations,
  activeId,
  onSwitch,
  onManage,
}: {
  networks: NetworkProfile[];
  locations: Location[];
  activeId?: string;
  onSwitch: (id: string) => void;
  onManage: () => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onClickAway = (event: MouseEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onClickAway);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClickAway);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const active = networks.find((n) => n.id === activeId);
  const activeLocation = locations.find((l) => l.id === active?.locationId);

  /* Headers only once there is a location to head. Someone who never files a
     network under one sees exactly the flat list this was before. */
  const groups = groupByLocation(networks, locations);
  const grouped = locations.length > 0;

  // The controller integration is configured per network, so the label marks
  // exactly where controller data applies — and by absence, where it does not.
  const unifiTag = (
    <span
      className="shrink-0 rounded border px-1 text-[9px] font-semibold uppercase tracking-wide"
      style={{
        borderColor: "var(--border-strong)",
        color: "var(--series-1)",
      }}
      title="A UniFi controller is configured for this network"
    >
      UniFi
    </span>
  );

  return (
    <div ref={containerRef} className="relative px-2 pb-2">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors hover:bg-[var(--surface-raised)]"
        style={{ borderColor: "var(--border)" }}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span aria-hidden style={{ color: "var(--series-1)", display: "inline-flex" }}>
          <Icon name="networks" size={15} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5">
            <span className="min-w-0 truncate text-sm font-medium">
              {active?.name ?? "No network"}
            </span>
            {active?.hasUnifi && unifiTag}
          </span>
          {/* Where you are, without having to open the dropdown to find out. */}
          <span className="block truncate text-[11px]" style={{ color: "var(--text-muted)" }}>
            {active
              ? `${activeLocation ? `${activeLocation.name} · ` : ""}${active.scanCount} scan${
                  active.scanCount === 1 ? "" : "s"
                }`
              : "Create one to start"}
          </span>
        </span>
        <span aria-hidden style={{ color: "var(--text-muted)", display: "inline-flex" }}>
          <Icon name={open ? "chevronUp" : "chevronDown"} size={13} />
        </span>
      </button>

      {open && (
        <div
          role="listbox"
          className="absolute left-2 right-2 z-30 mt-1 overflow-hidden rounded-lg border shadow-lg"
          style={{ borderColor: "var(--border-strong)", background: "var(--surface-raised)" }}
        >
          <ul className="max-h-64 overflow-y-auto">
            {networks.length === 0 && (
              <li className="px-3 py-2 text-xs" style={{ color: "var(--text-muted)" }}>
                No networks yet
              </li>
            )}
            {groups.map((group) => {
              const rows = group.networks.map((network) => (
                <li key={network.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={network.id === activeId}
                    onClick={() => {
                      setOpen(false);
                      if (network.id !== activeId) onSwitch(network.id);
                    }}
                    className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-[var(--surface-1)]"
                    style={{
                      fontWeight: network.id === activeId ? 600 : 400,
                    }}
                  >
                    <span
                      aria-hidden
                      style={{
                        color: network.id === activeId ? "var(--series-1)" : "transparent",
                        display: "inline-flex",
                      }}
                    >
                      <Icon name="check" size={13} />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1.5">
                        <span className="min-w-0 truncate">{network.name}</span>
                        {network.hasUnifi && unifiTag}
                      </span>
                      <span
                        className="block truncate text-[11px]"
                        style={{ color: "var(--text-muted)" }}
                      >
                        {network.fingerprint.subnets[0] ?? "unknown subnet"}
                      </span>
                    </span>
                  </button>
                </li>
              ));

              if (!grouped) return rows;

              const label = group.location?.name ?? "No location";
              return (
                /* A group is presentational scaffolding; the options inside it
                   stay the listbox's options. */
                <li key={group.location?.id ?? "unassigned"} role="presentation">
                  <div
                    className="sticky top-0 z-10 flex items-center gap-1 px-3 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-wider"
                    style={{ color: "var(--text-muted)", background: "var(--surface-raised)" }}
                  >
                    <span aria-hidden style={{ display: "inline-flex" }}>
                      <Icon name="location" size={10} />
                    </span>
                    <span className="min-w-0 truncate">{label}</span>
                  </div>
                  <ul role="group" aria-label={label}>
                    {rows}
                  </ul>
                </li>
              );
            })}
          </ul>

          <div className="border-t p-2" style={{ borderColor: "var(--border)" }}>
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                onManage();
              }}
              className="w-full rounded px-2 py-1 text-left text-xs hover:bg-[var(--surface-1)]"
              style={{ color: "var(--text-secondary)" }}
            >
              Manage networks…
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

/**
 * Asks what to do when the machine is on a network the app does not currently
 * have selected.
 *
 * This deliberately prompts rather than acting. Silently adopting a guess is how
 * two sites end up sharing one history — the failure this whole feature exists
 * to prevent — and a wrong guess is not obviously wrong until the diffs are
 * already nonsense.
 */
export function NetworkPrompt({
  detection,
  locations = [],
  onResolved,
  onDismiss,
}: {
  detection: Detection;
  locations?: Location[];
  onResolved: () => void;
  onDismiss: () => void;
}) {
  const [name, setName] = useState(
    detection.kind === "unknown" ? detection.suggestedName : "",
  );
  /* "" is no location, NEW_LOCATION opens the name field. Asked here because
     this is the one moment the user demonstrably knows where they are. */
  const [locationChoice, setLocationChoice] = useState("");
  const [newLocationName, setNewLocationName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const needsLocationName = locationChoice === NEW_LOCATION && !newLocationName.trim();

  /* Create the location first, then the network, so it is never briefly
     ungrouped. If the network then fails, an empty location is left behind —
     harmless, because creating by name is idempotent, so a retry reuses it. */
  const createNetwork = async () => {
    const locationId =
      locationChoice === NEW_LOCATION
        ? (await api.createLocation(newLocationName)).id
        : locationChoice || undefined;
    await api.createNetwork(name, locationId);
  };

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      onResolved();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const body = () => {
    switch (detection.kind) {
      case "switch":
        return {
          title: `This looks like “${detection.name}”`,
          message:
            detection.strength === "definitive"
              ? "The gateway's hardware address matches a network you already track. Switching keeps its history and diffs intact."
              : "This network closely matches one you already track. Switching keeps its history and diffs intact.",
          actions: (
            <>
              <Button onClick={onDismiss} disabled={busy}>
                Stay here
              </Button>
              <Button
                variant="primary"
                onClick={() => run(() => api.switchNetwork(detection.id))}
                disabled={busy}
              >
                Switch to {detection.name}
              </Button>
            </>
          ),
        };

      case "unknown":
        return {
          title: "New network",
          message:
            "This network is not one you track yet. Creating one keeps its scans separate, so its history and diffs stay meaningful.",
          actions: (
            <>
              <Button onClick={onDismiss} disabled={busy}>
                Not now
              </Button>
              <Button
                variant="primary"
                onClick={() => run(createNetwork)}
                disabled={busy || !name.trim() || needsLocationName}
              >
                Create
              </Button>
            </>
          ),
          input: true,
        };

      case "ambiguous":
        return {
          title: "Which network is this?",
          message:
            "Several tracked networks use this subnet and nothing distinguishes them — no gateway address was resolved, and the Wi-Fi name does not match. Picking the wrong one would merge two histories, so the app will not guess.",
          actions: (
            <>
              <Button onClick={onDismiss} disabled={busy}>
                Decide later
              </Button>
            </>
          ),
          candidates: detection.candidates,
        };

      default:
        return null;
    }
  };

  const content = body();
  if (!content) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-6"
      style={{ background: "rgba(0,0,0,0.45)" }}
    >
      <div
        role="dialog"
        aria-modal="true"
        className="w-full max-w-md rounded-xl border shadow-xl"
        style={{ borderColor: "var(--border-strong)", background: "var(--surface-1)" }}
      >
        <header className="border-b px-5 py-4" style={{ borderColor: "var(--border)" }}>
          <h2 className="text-base font-semibold">{content.title}</h2>
          <p className="mt-1 text-sm" style={{ color: "var(--text-secondary)" }}>
            {content.message}
          </p>
        </header>

        {(content.input || content.candidates) && (
          <div className="px-5 py-4">
            {content.input && (
              <label className="block">
                <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
                  Name
                </span>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  disabled={busy}
                  autoFocus
                  className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
                  style={{
                    borderColor: "var(--border-strong)",
                    background: "var(--surface-raised)",
                    color: "var(--text-primary)",
                  }}
                />
                <span className="mt-1 block text-[11px]" style={{ color: "var(--text-muted)" }}>
                  Something you will recognise later — “Home”, “Office”, a client name.
                </span>
              </label>
            )}

            {/* One place often has several networks — a main LAN, a guest SSID,
                a lab VLAN. Grouping them is easiest to get right now, while the
                user is standing in the place in question. */}
            {content.input && (
              <div className="mt-3">
                <label className="block">
                  <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
                    Location (optional)
                  </span>
                  <select
                    value={locationChoice}
                    onChange={(e) => setLocationChoice(e.target.value)}
                    disabled={busy}
                    className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
                    style={{
                      borderColor: "var(--border-strong)",
                      background: "var(--surface-raised)",
                      color: "var(--text-primary)",
                    }}
                  >
                    <option value="">No location</option>
                    {locations.map((location) => (
                      <option key={location.id} value={location.id}>
                        {location.name}
                      </option>
                    ))}
                    <option value={NEW_LOCATION}>New location…</option>
                  </select>
                </label>
                {locationChoice === NEW_LOCATION && (
                  <input
                    type="text"
                    value={newLocationName}
                    onChange={(e) => setNewLocationName(e.target.value)}
                    placeholder="Head office, Flat, client site…"
                    aria-label="New location name"
                    disabled={busy}
                    className="mt-2 w-full rounded-lg border px-2 py-1.5 text-sm"
                    style={{
                      borderColor: "var(--border-strong)",
                      background: "var(--surface-raised)",
                      color: "var(--text-primary)",
                    }}
                  />
                )}
              </div>
            )}

            {content.candidates && (
              <ul className="space-y-1.5">
                {content.candidates.map((candidate) => (
                  <li key={candidate.id}>
                    <button
                      type="button"
                      onClick={() => run(() => api.switchNetwork(candidate.id))}
                      disabled={busy}
                      className="w-full rounded-lg border px-3 py-2 text-left text-sm hover:bg-[var(--surface-raised)]"
                      style={{ borderColor: "var(--border)" }}
                    >
                      {candidate.name}
                    </button>
                  </li>
                ))}
                <li>
                  <button
                    type="button"
                    onClick={() => run(() => api.createNetwork(name || "New network"))}
                    disabled={busy}
                    className="w-full rounded-lg border px-3 py-2 text-left text-sm hover:bg-[var(--surface-raised)]"
                    style={{ borderColor: "var(--border)", color: "var(--text-secondary)" }}
                  >
                    None of these — create a new one
                  </button>
                </li>
              </ul>
            )}
          </div>
        )}

        {error && (
          <p className="px-5 pb-2 text-xs" style={{ color: "var(--status-critical)" }}>
            {error}
          </p>
        )}

        <footer
          className="flex flex-wrap items-center justify-end gap-2 border-t px-5 py-3"
          style={{ borderColor: "var(--border)" }}
        >
          {content.actions}
        </footer>
      </div>
    </div>
  );
}
