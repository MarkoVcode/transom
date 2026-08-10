"use client";

import { useState } from "react";
import { AdjacentNetworks } from "./AdjacentNetworks";
import { NEW_LOCATION } from "./NetworkSwitcher";
import { Button, Card, EmptyState, Icon, Pill, StatusBadge, formatRelativeTime } from "./ui";
import * as api from "@/lib/api";
import { groupByLocation } from "@/lib/networks";
import type { AdjacentSubnet, Location, NetworkProfile } from "@/lib/types";

/**
 * Managing the tracked networks.
 *
 * The fingerprint is shown rather than hidden, because when detection gets it
 * wrong the reason is always visible here — usually a gateway address that was
 * never resolved, leaving nothing but the subnet to go on.
 */
export function NetworksPanel({
  networks,
  locations,
  activeId,
  onChanged,
  onDiscover,
  adjacent,
  onReloadAdjacent,
}: {
  networks: NetworkProfile[];
  /** Named places, already in display order. */
  locations: Location[];
  activeId?: string;
  onChanged: () => void;
  /** Reopens the first-run picker of currently-visible subnets. */
  onDiscover?: () => void;
  /** Neighbouring subnets found from evidence; `null` while loading. */
  adjacent: AdjacentSubnet[] | null;
  onReloadAdjacent: () => void | Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newRange, setNewRange] = useState("");
  const [rangeName, setRangeName] = useState("");
  /* Location editing. `assigning` holds a network id, the other two a location
     id — every one of them an open editor that `run` has to close. */
  const [assigning, setAssigning] = useState<string | null>(null);
  const [locationChoice, setLocationChoice] = useState("");
  const [locationName, setLocationName] = useState("");
  const [renamingLocation, setRenamingLocation] = useState<string | null>(null);
  const [locationDraft, setLocationDraft] = useState("");
  const [confirmDeleteLocation, setConfirmDeleteLocation] = useState<string | null>(null);
  const [creatingLocation, setCreatingLocation] = useState(false);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
      setRenaming(null);
      setConfirmDelete(null);
      setConfirmClear(null);
      setAssigning(null);
      setRenamingLocation(null);
      setConfirmDeleteLocation(null);
      setCreatingLocation(false);
      setLocationName("");
    }
  };

  /* Create the location first when the user typed a new name, so the network
     lands in it directly rather than being filed a moment later. */
  const applyLocation = async (networkId: string) => {
    const locationId =
      locationChoice === NEW_LOCATION
        ? (await api.createLocation(locationName)).id
        : locationChoice || undefined;
    await api.setNetworkLocation(networkId, locationId);
  };

  const inputStyle = {
    borderColor: "var(--border-strong)",
    background: "var(--surface-raised)",
    color: "var(--text-primary)",
  };

  /* Empty locations stay visible here — this is the only place they can be
     renamed or removed, so hiding them would strand them. */
  const groups = groupByLocation(networks, locations, { includeEmpty: true });
  const grouped = locations.length > 0;

  return (
    <div className="space-y-4">
      <Card
        title="Networks"
        subtitle="Each network keeps its own scan history, diffs and controller settings"
        actions={
          <>
            <Button
              onClick={() => {
                setCreatingLocation(true);
                setLocationName("");
              }}
              disabled={busy || creatingLocation}
            >
              New location…
            </Button>
            {onDiscover && (
              <Button onClick={onDiscover} disabled={busy}>
                Discover networks…
              </Button>
            )}
          </>
        }
      >
        <p className="mb-3 text-sm" style={{ color: "var(--text-secondary)" }}>
          Two places often use the same subnet — <code>192.168.0.0/24</code> is not unusual at
          both home and an office. Networks are told apart primarily by the gateway&apos;s
          hardware address, so scanning at a new site keeps its results separate instead of
          corrupting the history of the last one.
        </p>

        {creatingLocation && (
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <input
              type="text"
              value={locationName}
              onChange={(e) => setLocationName(e.target.value)}
              placeholder="Head office, Flat, client site…"
              aria-label="New location name"
              autoFocus
              disabled={busy}
              className="min-w-48 flex-1 rounded border px-2 py-1 text-sm"
              style={inputStyle}
            />
            <Button
              variant="primary"
              onClick={() => run(() => api.createLocation(locationName))}
              disabled={busy || !locationName.trim()}
            >
              Add location
            </Button>
            <button
              type="button"
              onClick={() => {
                setCreatingLocation(false);
                setLocationName("");
              }}
              className="text-xs hover:underline"
              style={{ color: "var(--text-muted)" }}
            >
              Cancel
            </button>
          </div>
        )}

        {networks.length === 0 && locations.length === 0 ? (
          <EmptyState
            title="No networks yet"
            hint="Create one for the network you are on now."
          />
        ) : (
          <div className="space-y-3">
            {groups.map((group) => (
              <div key={group.location?.id ?? "unassigned"}>
                {grouped && (
                  <div className="mb-1.5 flex flex-wrap items-center gap-2">
                    {renamingLocation === group.location?.id ? (
                      <>
                        <input
                          type="text"
                          value={locationDraft}
                          onChange={(e) => setLocationDraft(e.target.value)}
                          aria-label="Location name"
                          autoFocus
                          className="rounded border px-2 py-1 text-sm"
                          style={inputStyle}
                        />
                        <Button
                          onClick={() =>
                            run(() => api.renameLocation(group.location!.id, locationDraft))
                          }
                          disabled={busy || !locationDraft.trim()}
                        >
                          Save
                        </Button>
                        <button
                          type="button"
                          onClick={() => setRenamingLocation(null)}
                          className="text-xs hover:underline"
                          style={{ color: "var(--text-muted)" }}
                        >
                          Cancel
                        </button>
                      </>
                    ) : (
                      <>
                        <span
                          className="flex items-center gap-1 text-xs font-semibold uppercase tracking-wider"
                          style={{ color: "var(--text-secondary)" }}
                        >
                          <span aria-hidden style={{ display: "inline-flex" }}>
                            <Icon name="location" size={12} />
                          </span>
                          {group.location?.name ?? "No location"}
                        </span>
                        {group.location && (
                          <>
                            <button
                              type="button"
                              onClick={() => {
                                setRenamingLocation(group.location!.id);
                                setLocationDraft(group.location!.name);
                              }}
                              disabled={busy}
                              className="text-xs hover:underline"
                              style={{ color: "var(--text-muted)" }}
                            >
                              Rename
                            </button>
                            <button
                              type="button"
                              onClick={() => setConfirmDeleteLocation(group.location!.id)}
                              disabled={busy}
                              className="text-xs hover:underline"
                              style={{ color: "var(--status-critical)" }}
                            >
                              Delete
                            </button>
                          </>
                        )}
                      </>
                    )}
                  </div>
                )}

                {/* A location is a label, so say plainly that removing it keeps
                    what it labelled — next to a group of networks, "Delete" reads
                    as destructive until it says otherwise. */}
                {group.location && confirmDeleteLocation === group.location.id && (
                  <div
                    className="mb-2 rounded-lg border p-2.5"
                    style={{ borderColor: "var(--status-critical)" }}
                  >
                    <p className="text-sm">
                      Delete the location <strong>{group.location.name}</strong>? The{" "}
                      {group.networks.length} network
                      {group.networks.length === 1 ? "" : "s"} in it{" "}
                      {group.networks.length === 1 ? "is" : "are"} kept, along with{" "}
                      {group.networks.length === 1 ? "its" : "their"} scans, and move to{" "}
                      <em>No location</em>.
                    </p>
                    <div className="mt-2 flex flex-wrap gap-2">
                      <Button
                        variant="danger"
                        onClick={() => run(() => api.deleteLocation(group.location!.id))}
                        disabled={busy}
                      >
                        Delete location
                      </Button>
                      <Button onClick={() => setConfirmDeleteLocation(null)} disabled={busy}>
                        Cancel
                      </Button>
                    </div>
                  </div>
                )}

                {group.networks.length === 0 ? (
                  <p className="text-xs" style={{ color: "var(--text-muted)" }}>
                    No networks here yet — use <em>Location…</em> on a network below to move it
                    here.
                  </p>
                ) : (
                  <ul className="space-y-2">
                    {group.networks.map((network) => {
                      const active = network.id === activeId;
                      const weak = !network.fingerprint.gatewayMac && !network.fingerprint.ssid;

                      return (
                        <li
                          key={network.id}
                          className="rounded-lg border p-3"
                          style={{ borderColor: active ? "var(--series-1)" : "var(--border)" }}
                        >
                          <div className="flex flex-wrap items-start justify-between gap-2">
                            <div className="min-w-0">
                              {renaming === network.id ? (
                                <div className="flex flex-wrap items-center gap-2">
                                  <input
                                    type="text"
                                    value={draftName}
                                    onChange={(e) => setDraftName(e.target.value)}
                                    autoFocus
                                    className="rounded border px-2 py-1 text-sm"
                                    style={{
                                      borderColor: "var(--border-strong)",
                                      background: "var(--surface-raised)",
                                      color: "var(--text-primary)",
                                    }}
                                  />
                                  <Button
                                    onClick={() => run(() => api.renameNetwork(network.id, draftName))}
                                    disabled={busy || !draftName.trim()}
                                  >
                                    Save
                                  </Button>
                                  <button
                                    type="button"
                                    onClick={() => setRenaming(null)}
                                    className="text-xs hover:underline"
                                    style={{ color: "var(--text-muted)" }}
                                  >
                                    Cancel
                                  </button>
                                </div>
                              ) : (
                                <div className="flex flex-wrap items-center gap-2">
                                  <span className="font-medium">{network.name}</span>
                                  {active && <StatusBadge tone="good" label="Active" />}
                                  {network.hasUnifi && (
                                    <span
                                      className="rounded border px-1 text-[9px] font-semibold uppercase tracking-wide"
                                      style={{
                                        borderColor: "var(--border-strong)",
                                        color: "var(--series-1)",
                                      }}
                                      title="A UniFi controller is configured for this network"
                                    >
                                      UniFi
                                    </span>
                                  )}
                                  {weak && (
                                    <StatusBadge tone="warning" label="Weak fingerprint" />
                                  )}
                                </div>
                              )}

                              <div className="mt-1 flex flex-wrap gap-1.5">
                                {network.fingerprint.subnets.map((subnet) => (
                                  <Pill key={subnet} mono>
                                    {subnet}
                                  </Pill>
                                ))}
                                {network.fingerprint.ssid && <Pill>{network.fingerprint.ssid}</Pill>}
                                {network.fingerprint.gatewayMac && (
                                  <Pill mono>gw {network.fingerprint.gatewayMac}</Pill>
                                )}
                              </div>

                              <p className="mt-1 text-xs" style={{ color: "var(--text-muted)" }}>
                                {network.scanCount} scan{network.scanCount === 1 ? "" : "s"}
                                {network.lastSeenAt
                                  ? ` · last ${formatRelativeTime(network.lastSeenAt)}`
                                  : " · never scanned"}
                              </p>

                              {weak && (
                                <p className="mt-1.5 text-xs" style={{ color: "var(--text-secondary)" }}>
                                  Only the subnet identifies this network, so it cannot be told apart
                                  from another site using the same range. Select it and use{" "}
                                  <em>Re-detect</em> while connected to pick up the gateway address.
                                </p>
                              )}
                            </div>

                            <div className="flex shrink-0 flex-wrap gap-2">
                              {!active && (
                                <Button
                                  onClick={() => run(() => api.switchNetwork(network.id))}
                                  disabled={busy}
                                >
                                  Switch
                                </Button>
                              )}
                              <button
                                type="button"
                                onClick={() => {
                                  setRenaming(network.id);
                                  setDraftName(network.name);
                                }}
                                disabled={busy}
                                className="text-xs hover:underline"
                                style={{ color: "var(--text-muted)" }}
                              >
                                Rename
                              </button>
                              <button
                                type="button"
                                onClick={() => {
                                  setAssigning(network.id);
                                  setLocationChoice(network.locationId ?? "");
                                  setLocationName("");
                                }}
                                disabled={busy}
                                className="text-xs hover:underline"
                                style={{ color: "var(--text-muted)" }}
                              >
                                Location…
                              </button>
                              {network.scanCount > 0 && (
                                <button
                                  type="button"
                                  onClick={() => setConfirmClear(network.id)}
                                  disabled={busy}
                                  className="text-xs hover:underline"
                                  style={{ color: "var(--text-muted)" }}
                                >
                                  Clear history
                                </button>
                              )}
                              <button
                                type="button"
                                onClick={() => setConfirmDelete(network.id)}
                                disabled={busy}
                                className="text-xs hover:underline"
                                style={{ color: "var(--status-critical)" }}
                              >
                                Delete
                              </button>
                            </div>
                          </div>

                          {assigning === network.id && (
                            <div
                              className="mt-2 rounded-lg border p-2.5"
                              style={{ borderColor: "var(--border-strong)" }}
                            >
                              <div className="flex flex-wrap items-center gap-2">
                                <select
                                  value={locationChoice}
                                  onChange={(e) => setLocationChoice(e.target.value)}
                                  aria-label={`Location for ${network.name}`}
                                  disabled={busy}
                                  className="rounded border px-2 py-1 text-sm"
                                  style={inputStyle}
                                >
                                  <option value="">No location</option>
                                  {locations.map((location) => (
                                    <option key={location.id} value={location.id}>
                                      {location.name}
                                    </option>
                                  ))}
                                  <option value={NEW_LOCATION}>New location…</option>
                                </select>
                                {locationChoice === NEW_LOCATION && (
                                  <input
                                    type="text"
                                    value={locationName}
                                    onChange={(e) => setLocationName(e.target.value)}
                                    placeholder="Head office, Flat…"
                                    aria-label="New location name"
                                    autoFocus
                                    disabled={busy}
                                    className="rounded border px-2 py-1 text-sm"
                                    style={inputStyle}
                                  />
                                )}
                                <Button
                                  onClick={() => run(() => applyLocation(network.id))}
                                  disabled={
                                    busy || (locationChoice === NEW_LOCATION && !locationName.trim())
                                  }
                                >
                                  Save
                                </Button>
                                <button
                                  type="button"
                                  onClick={() => setAssigning(null)}
                                  className="text-xs hover:underline"
                                  style={{ color: "var(--text-muted)" }}
                                >
                                  Cancel
                                </button>
                              </div>
                              <p className="mt-2 text-xs" style={{ color: "var(--text-secondary)" }}>
                                A location groups the networks at one place — a main LAN, a guest
                                SSID and a lab VLAN in the same building. It changes nothing about
                                how they are scanned or kept apart.
                              </p>
                            </div>
                          )}

                          {confirmClear === network.id && (
                            <div
                              className="mt-2 rounded-lg border p-2.5"
                              style={{ borderColor: "var(--status-warning)" }}
                            >
                              <p className="text-sm">
                                Delete all {network.scanCount} scan{network.scanCount === 1 ? "" : "s"}{" "}
                                recorded for <strong>{network.name}</strong>? The network itself, its
                                fingerprint and controller settings stay; the next scan starts a fresh
                                history. Useful when scans from before version 1.2 mixed several sites
                                into one list.
                              </p>
                              <div className="mt-2 flex flex-wrap gap-2">
                                <Button
                                  variant="danger"
                                  onClick={() => run(() => api.clearNetworkHistory(network.id))}
                                  disabled={busy}
                                >
                                  Clear history
                                </Button>
                                <Button onClick={() => setConfirmClear(null)} disabled={busy}>
                                  Cancel
                                </Button>
                              </div>
                            </div>
                          )}

                          {confirmDelete === network.id && (
                            <div
                              className="mt-2 rounded-lg border p-2.5"
                              style={{ borderColor: "var(--status-critical)" }}
                            >
                              <p className="text-sm">
                                Delete <strong>{network.name}</strong> and all {network.scanCount} of its
                                scans? This cannot be undone.
                              </p>
                              <div className="mt-2 flex flex-wrap gap-2">
                                <Button
                                  variant="danger"
                                  onClick={() => run(() => api.deleteNetwork(network.id))}
                                  disabled={busy}
                                >
                                  Delete permanently
                                </Button>
                                <Button onClick={() => setConfirmDelete(null)} disabled={busy}>
                                  Cancel
                                </Button>
                              </div>
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
            ))}
          </div>
        )}

        {error && (
          <p className="mt-3 text-xs" style={{ color: "var(--status-critical)" }}>
            {error}
          </p>
        )}
      </Card>

      <Card title="Add the network you are on now">
        <div className="flex flex-wrap items-end gap-2">
          <label className="min-w-48 flex-1">
            <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
              Name
            </span>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="Home, Office, client name…"
              disabled={busy}
              className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
              style={{
                borderColor: "var(--border-strong)",
                background: "var(--surface-raised)",
                color: "var(--text-primary)",
              }}
            />
          </label>
          <Button
            variant="primary"
            onClick={() =>
              run(async () => {
                await api.createNetwork(newName);
                setNewName("");
              })
            }
            disabled={busy || !newName.trim()}
          >
            Create
          </Button>
        </div>
        <p className="mt-2 text-xs" style={{ color: "var(--text-secondary)" }}>
          The fingerprint is taken from the network this machine is attached to right now —
          whatever you call it. For a subnet you are <em>not</em> on, use the form below instead.
        </p>
      </Card>

      {/* The only way to track a network used to be "the one you are on", so a
          remote subnet could only be named — never actually defined. Naming it
          after the range produced a copy of the local network, which then got
          scanned in its place. */}
      <Card title="Add a network by address range">
        <div className="flex flex-wrap items-end gap-2">
          <label className="min-w-48 flex-1">
            <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
              Name
            </span>
            <input
              type="text"
              value={rangeName}
              onChange={(e) => setRangeName(e.target.value)}
              placeholder="Lab, Guest VLAN, DMZ…"
              disabled={busy}
              className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
              style={{
                borderColor: "var(--border-strong)",
                background: "var(--surface-raised)",
                color: "var(--text-primary)",
              }}
            />
          </label>
          <label className="min-w-40">
            <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
              Range
            </span>
            <input
              type="text"
              value={newRange}
              onChange={(e) => setNewRange(e.target.value)}
              placeholder="10.0.2.0/24"
              disabled={busy}
              className="mt-1 w-full rounded-lg border px-2 py-1.5 font-mono text-sm"
              style={{
                borderColor: "var(--border-strong)",
                background: "var(--surface-raised)",
                color: "var(--text-primary)",
              }}
            />
          </label>
          <Button
            variant="primary"
            onClick={() =>
              run(async () => {
                await api.ensureNetworkForRange(newRange, rangeName.trim() || undefined);
                setNewRange("");
                setRangeName("");
              })
            }
            disabled={busy || !newRange.trim()}
          >
            Add
          </Button>
        </div>
        <p className="mt-2 text-xs" style={{ color: "var(--text-secondary)" }}>
          For a subnet this machine can reach by routing but is not attached to — another VLAN,
          or a segment across a router. Scans of it sweep that range only, and it becomes the
          selected network so you can scan it straight away.
        </p>
      </Card>

      <AdjacentNetworks
        subnets={adjacent}
        onChanged={onChanged}
        onReload={onReloadAdjacent}
      />

      {activeId && (
        <Card title="Re-detect">
          <p className="text-sm" style={{ color: "var(--text-secondary)" }}>
            Updates the active network&apos;s fingerprint from where this machine is now. Use it
            if the network was created before its gateway address could be read, or after the
            router was replaced. It also decides what a scan of this network sweeps, so
            re-detecting is refused unless this machine is actually on it.
          </p>
          <div className="mt-2">
            <Button onClick={() => run(api.refreshNetworkFingerprint)} disabled={busy}>
              Re-detect this network
            </Button>
          </div>
        </Card>
      )}
    </div>
  );
}
