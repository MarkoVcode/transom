"use client";

import { Fragment } from "react";
import { Card, EmptyState, Icon, LoadingState, Pill, StatusBadge, StatusMark } from "./ui";
import type { Reconciliation, UnifiSnapshot } from "@/lib/types";

function formatSpeed(mbps?: number): string {
  if (mbps === undefined) return "up";
  return mbps >= 1000 ? `${mbps / 1000} Gbps` : `${mbps} Mbps`;
}

const SUBSYSTEM_LABELS: Record<string, string> = {
  wan: "WAN",
  www: "Internet",
  lan: "LAN",
  wlan: "Wi-Fi",
  vpn: "VPN",
};

function healthTone(status: string): "good" | "warning" | "critical" {
  if (status === "ok") return "good";
  if (status === "warning") return "warning";
  return "critical";
}

/** The controller's own per-subsystem verdicts, plus the LAN-vs-internet triage. */
function SiteHealth({ unifi }: { unifi: UnifiSnapshot }) {
  // "unknown" subsystems (usually an unconfigured VPN) are noise, not status.
  const health = (unifi.health ?? []).filter((h) => h.status !== "unknown");
  if (health.length === 0 && !unifi.wanTriage) return null;

  return (
    <Card title="Site health" subtitle="As reported by the controller itself">
      {health.length > 0 && (
        <div className="flex flex-wrap gap-3">
          {health.map((h) => (
            <div
              key={h.subsystem}
              className="flex items-center gap-2 rounded-lg border px-3 py-2"
              style={{ borderColor: "var(--border)" }}
            >
              <span className="text-sm font-medium">
                {SUBSYSTEM_LABELS[h.subsystem] ?? h.subsystem}
              </span>
              <StatusBadge tone={healthTone(h.status)} label={h.status} />
              {h.latencyMs !== undefined && <Pill mono>{Math.round(h.latencyMs)} ms</Pill>}
              {h.wanIp && <Pill mono>{h.wanIp}</Pill>}
              {h.clients !== undefined && (
                <Pill>{h.clients + (h.guests ?? 0)} clients</Pill>
              )}
              {h.disconnected !== undefined && h.disconnected > 0 && (
                <StatusBadge tone="warning" label={`${h.disconnected} device(s) down`} />
              )}
            </div>
          ))}
        </div>
      )}
      {unifi.wanTriage && (
        <p className="mt-3 text-sm" style={{ color: "var(--text-secondary)" }}>
          {unifi.wanTriage}
        </p>
      )}
    </Card>
  );
}

/**
 * Where the scan and the controller disagree.
 *
 * The quadrant is the point of the integration: a host answering TCP that the
 * controller has never issued a lease to is interesting *because* the two views
 * conflict. Neither tool can produce that category on its own.
 */
export function ReconciliationPanel({
  reconciliation,
  unifi,
  loading = false,
  fetching = false,
  configured = false,
  networkName,
}: {
  reconciliation?: Reconciliation;
  unifi?: UnifiSnapshot;
  /** The snapshot this would come from is still being read. */
  loading?: boolean;
  /** A scan is in flight and currently talking to the controller. */
  fetching?: boolean;
  /** A controller is configured for the selected network. */
  configured?: boolean;
  networkName?: string;
}) {
  if (fetching || loading) {
    return (
      <Card title="Controller reconciliation">
        <LoadingState
          title={fetching ? "Querying the controller…" : "Loading controller data…"}
          hint={
            fetching
              ? "Matching this scan's devices against the controller's records. A controller that is slow to answer can add a minute or more."
              : "Reading the latest scan for this network."
          }
        />
      </Card>
    );
  }

  if (!reconciliation) {
    // "No controller connected" is wrong when one *is* connected and simply has
    // not been through a scan yet — and it sends the user looking for a setting
    // that is already set.
    return (
      <Card title="Controller reconciliation">
        <EmptyState
          title={configured ? "No controller data in this scan" : "No controller connected"}
          hint={
            configured
              ? `A controller is configured for ${
                  networkName ? `“${networkName}”` : "this network"
                }, but the latest scan predates it or could not reach it. Run a scan to compare what the scan finds against what the controller knows.`
              : "Connect a UniFi controller below to compare what the scan finds against what the controller knows."
          }
        />
      </Card>
    );
  }

  const { matched, shadow, missed, hiddenSegments, identityConflicts } = reconciliation;
  // Absent on snapshots stored before these findings existed.
  const wirelessIssues = reconciliation.wirelessIssues ?? [];
  const degradedLinks = reconciliation.degradedLinks ?? [];
  const unscannedNetworks = reconciliation.unscannedNetworks ?? [];
  const flappingClients = reconciliation.flappingClients ?? [];
  const troubledPorts = reconciliation.troubledPorts ?? [];
  const saturatedRadios = reconciliation.saturatedRadios ?? [];
  const loadedDevices = reconciliation.loadedDevices ?? [];
  const meshedAccessPoints = reconciliation.meshedAccessPoints ?? [];
  const alarms = unifi?.alarms ?? [];
  const clean =
    shadow.length === 0 &&
    missed.length === 0 &&
    hiddenSegments.length === 0 &&
    identityConflicts.length === 0 &&
    wirelessIssues.length === 0 &&
    degradedLinks.length === 0 &&
    unscannedNetworks.length === 0 &&
    flappingClients.length === 0 &&
    troubledPorts.length === 0 &&
    saturatedRadios.length === 0 &&
    loadedDevices.length === 0 &&
    alarms.length === 0;

  return (
    <div className="space-y-4">
      {unifi && <SiteHealth unifi={unifi} />}

      {alarms.length > 0 && (
        <Card
          title={`Controller alarms — ${alarms.length}`}
          subtitle="Active alarms raised by the controller, verbatim"
        >
          <ul className="space-y-1.5">
            {alarms.map((alarm, i) => (
              <li key={i} className="flex items-start gap-2 text-sm">
                <StatusMark tone="critical" className="mt-1" />
                <span style={{ color: "var(--text-secondary)" }}>
                  {alarm.message}
                  {alarm.time !== undefined && (
                    <span className="ml-2 text-xs" style={{ color: "var(--text-muted)" }}>
                      {new Date(alarm.time * 1000).toLocaleString()}
                    </span>
                  )}
                </span>
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Card title="Reconciliation" subtitle={reconciliation.summary}>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          {[
            { label: "Accounted for", value: matched, tone: "good" as const },
            { label: "Unknown to controller", value: shadow.length, tone: "critical" as const },
            { label: "Scan did not reach", value: missed.length, tone: "warning" as const },
            { label: "Ports hiding devices", value: hiddenSegments.length, tone: "warning" as const },
          ].map((tile) => (
            <div
              key={tile.label}
              className="rounded-lg border p-3"
              style={{ borderColor: "var(--border)" }}
            >
              <p className="text-xs" style={{ color: "var(--text-secondary)" }}>
                {tile.label}
              </p>
              <p className="mt-1 flex items-center gap-2 text-xl font-semibold tabular">
                {tile.value}
                {tile.value > 0 && tile.tone !== "good" && (
                  <span
                    aria-hidden
                    style={{
                      fontSize: "0.5em",
                      color:
                        tile.tone === "critical"
                          ? "var(--status-critical)"
                          : "var(--status-warning)",
                      display: "inline-flex",
                    }}
                  >
                    <Icon name="dot" size={9} />
                  </span>
                )}
              </p>
            </div>
          ))}
        </div>

        {clean && (
          <p className="mt-3 text-sm" style={{ color: "var(--text-secondary)" }}>
            Everything the scan found is known to the controller, and everything the controller
            knows was reachable. Nothing to investigate.
          </p>
        )}
      </Card>

      {shadow.length > 0 && (
        <Card
          title={`Unknown to the controller — ${shadow.length}`}
          subtitle="Reachable on the network, but the controller has no record of them"
        >
          <ul className="space-y-2">
            {shadow.map((device) => (
              <li
                key={device.ip}
                className="rounded-lg border p-3"
                style={{ borderColor: "var(--status-critical)" }}
              >
                <div className="flex flex-wrap items-center gap-2">
                  <StatusBadge tone="critical" label="Unaccounted" />
                  <span className="font-medium">{device.displayName}</span>
                  <span className="font-mono text-xs tabular" style={{ color: "var(--text-muted)" }}>
                    {device.ip}
                  </span>
                  {device.vendor && <Pill>{device.vendor}</Pill>}
                </div>

                {device.mac && (
                  <p className="mt-1 font-mono text-[11px] tabular" style={{ color: "var(--text-muted)" }}>
                    {device.mac}
                  </p>
                )}

                {device.openPorts.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1">
                    {device.openPorts.slice(0, 8).map((port, i) => (
                      <Pill key={`${port}-${i}`} mono>
                        {port}
                      </Pill>
                    ))}
                  </div>
                )}

                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {device.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {missed.length > 0 && (
        <Card
          title={`The scan did not reach — ${missed.length}`}
          subtitle="Known to the controller but not seen by this scan — where our own coverage is blind"
        >
          <ul className="space-y-2">
            {missed.map((device) => (
              <li key={device.mac} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <StatusBadge tone="warning" label="Not reached" />
                  <span className="font-medium">{device.name}</span>
                  {device.ip && (
                    <span className="font-mono text-xs tabular" style={{ color: "var(--text-muted)" }}>
                      {device.ip}
                    </span>
                  )}
                  {device.location && <Pill>{device.location}</Pill>}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {device.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {hiddenSegments.length > 0 && (
        <Card
          title={`Ports hiding other devices — ${hiddenSegments.length}`}
          subtitle="Several hardware addresses behind one switch port"
        >
          <ul className="space-y-2">
            {hiddenSegments.map((segment, i) => (
              <li key={i} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{segment.switchName}</span>
                  <Pill mono>port {segment.port}</Pill>
                  {segment.portName && <Pill>{segment.portName}</Pill>}
                  <StatusBadge tone="warning" label={`${segment.macCount} devices`} />
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {segment.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {identityConflicts.length > 0 && (
        <Card
          title={`Identity conflicts — ${identityConflicts.length}`}
          subtitle="The controller's idea of what a device is disagrees with what it actually runs"
        >
          <ul className="space-y-1.5">
            {identityConflicts.map((conflict, i) => (
              <li key={i} className="flex items-start gap-2 text-sm">
                <StatusMark tone="serious" className="mt-1" />
                <span style={{ color: "var(--text-secondary)" }}>{conflict}</span>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {unifi && (unifi.networks ?? []).length > 0 && (
        <Card
          title={`Configured networks — ${unifi.networks!.length}`}
          subtitle="Every network the controller defines, whether or not this scan could see it"
        >
          <ul className="space-y-2">
            {unifi.networks!.map((net, i) => {
              const blindSpot = unscannedNetworks.find((u) => u.name === net.name);
              return (
                <li
                  key={i}
                  className="rounded-lg border p-3"
                  style={{
                    borderColor: blindSpot ? "var(--status-warning)" : "var(--border)",
                    opacity: net.enabled ? 1 : 0.55,
                  }}
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{net.name}</span>
                    {net.subnet && <Pill mono>{net.subnet}</Pill>}
                    {net.vlan !== undefined && <Pill mono>VLAN {net.vlan}</Pill>}
                    {net.purpose === "guest" && <Pill>guest</Pill>}
                    {!net.enabled && <Pill>disabled</Pill>}
                    {blindSpot ? (
                      <StatusBadge tone="warning" label="Never scanned" />
                    ) : (
                      /* WAN/VPN entries are not scannable LANs, so neither badge applies. */
                      net.enabled &&
                      net.subnet &&
                      !["wan", "wan2", "vpn-client", "site-vpn", "remote-user-vpn"].includes(
                        net.purpose ?? "",
                      ) && <StatusBadge tone="good" label="Covered" />
                    )}
                  </div>
                  {blindSpot && (
                    <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                      {blindSpot.explanation}
                    </p>
                  )}
                </li>
              );
            })}
          </ul>
        </Card>
      )}

      {wirelessIssues.length > 0 && (
        <Card
          title={`Struggling on Wi-Fi — ${wirelessIssues.length}`}
          subtitle="Connected, but the controller itself rates the connection as poor"
        >
          <ul className="space-y-2">
            {wirelessIssues.map((issue) => (
              <li key={issue.ip} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{issue.displayName}</span>
                  <span className="font-mono text-xs tabular" style={{ color: "var(--text-muted)" }}>
                    {issue.ip}
                  </span>
                  {issue.satisfaction !== undefined && (
                    <StatusBadge
                      tone={issue.satisfaction < 60 ? "critical" : "warning"}
                      label={`${issue.satisfaction}%`}
                    />
                  )}
                  {issue.signalDbm !== undefined && <Pill mono>{issue.signalDbm} dBm</Pill>}
                  {issue.accessPoint && <Pill>{issue.accessPoint}</Pill>}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {issue.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {flappingClients.length > 0 && (
        <Card
          title={`Unstable connections — ${flappingClients.length}`}
          subtitle="Clients the controller's event log shows repeatedly dropping off"
        >
          <ul className="space-y-2">
            {flappingClients.map((flap) => (
              <li key={flap.mac} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{flap.name}</span>
                  <span className="font-mono text-[11px] tabular" style={{ color: "var(--text-muted)" }}>
                    {flap.mac}
                  </span>
                  <StatusBadge tone="warning" label={`${flap.disconnects} drops / 24 h`} />
                  {flap.accessPoint && <Pill>{flap.accessPoint}</Pill>}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {flap.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {/* Separate from "degraded links", which is about negotiated speed. These
          are the faults a healthy link speed hides — and the usual answer to
          "the link says gigabit but the transfer crawls". */}
      {troubledPorts.length > 0 && (
        <Card
          title={`Port faults — ${troubledPorts.length}`}
          subtitle="Links that are up and fast on paper, and faulty in practice"
        >
          <ul className="space-y-2">
            {troubledPorts.map((port) => (
              <li
                key={`${port.switchName}-${port.port}`}
                className="rounded-lg border p-3"
                style={{ borderColor: "var(--border)" }}
              >
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{port.switchName}</span>
                  <Pill mono>
                    port {port.port}
                    {port.portName ? ` · ${port.portName}` : ""}
                  </Pill>
                  <Pill mono>{port.speedMbps} Mbps</Pill>
                  {port.halfDuplex && <StatusBadge tone="critical" label="Half duplex" />}
                  {port.stpBlocking && <StatusBadge tone="warning" label="STP blocking" />}
                  {port.errors > 0 && (
                    <StatusBadge tone="serious" label={`${port.errors.toLocaleString()} errors`} />
                  )}
                  {port.drops > 0 && (
                    <StatusBadge tone="warning" label={`${port.drops.toLocaleString()} drops`} />
                  )}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {port.halfDuplex
                    ? "This link negotiated half duplex. Throughput collapses under traffic in both directions at once while a ping still looks healthy — usually a forced speed/duplex setting at one end, or a failing cable."
                    : port.stpBlocking
                      ? "Spanning tree has blocked this port, which means it found a loop. The port carries no traffic until the loop is removed."
                      : "Frames on this link are being discarded. On a gigabit link that almost always means a damaged pair or a marginal connector — errors rise with traffic while the link stays up."}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {saturatedRadios.length > 0 && (
        <Card
          title={`Saturated radios — ${saturatedRadios.length}`}
          subtitle="Airtime is shared, so a busy channel slows every client on it"
        >
          <ul className="space-y-2">
            {saturatedRadios.map((radio, i) => (
              <li key={i} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{radio.accessPoint}</span>
                  <Pill>{radio.band}</Pill>
                  <StatusBadge
                    tone={radio.utilisationPercent >= 90 ? "critical" : "serious"}
                    label={`${radio.utilisationPercent.toFixed(0)}% airtime`}
                  />
                  <Pill>{radio.clients} client(s)</Pill>
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {radio.interferencePercent !== undefined && radio.interferencePercent > 40
                    ? `${radio.interferencePercent.toFixed(0)}% of that airtime is not this network's traffic — a neighbouring network or non-Wi-Fi interference is using the channel. Moving to a quieter one helps more than anything done to the clients.`
                    : "Most of this airtime is this network's own traffic. The band is simply full: a slow client holds the channel while it transmits, so everyone else waits."}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {loadedDevices.length > 0 && (
        <Card
          title={`Devices under load — ${loadedDevices.length}`}
          subtitle="Infrastructure at its limit adds delay to everything it forwards"
        >
          <ul className="space-y-2">
            {loadedDevices.map((device, i) => (
              <li key={i} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{device.device}</span>
                  <Pill>{device.role}</Pill>
                  <StatusBadge
                    tone={device.cpuPercent >= 95 ? "critical" : "serious"}
                    label={`${device.cpuPercent.toFixed(0)}% CPU`}
                  />
                  {device.memPercent !== undefined && (
                    <Pill>{device.memPercent.toFixed(0)}% memory</Pill>
                  )}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  Sustained load here affects every flow this device handles. On a gateway that
                  includes traffic routed between VLANs, which is why a transfer between two local
                  machines on different networks can stall the whole site.
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {meshedAccessPoints.length > 0 && (
        <Card
          title={`Wireless backhaul — ${meshedAccessPoints.length}`}
          subtitle="Context rather than a fault: these access points have no cable"
        >
          <ul className="space-y-2">
            {meshedAccessPoints.map((ap, i) => (
              <li key={i} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{ap.accessPoint}</span>
                  {ap.speedMbps !== undefined && <Pill mono>{ap.speedMbps} Mbps uplink</Pill>}
                  {ap.rssi !== undefined && <Pill mono>RSSI {ap.rssi}</Pill>}
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  Every client on this access point shares its wireless uplink, and that uplink
                  competes for the same airtime as they do. A transfer through it is capped by the
                  weaker of the two hops, whatever the clients&apos; own signal strength suggests.
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {degradedLinks.length > 0 && (
        <Card
          title={`Degraded links — ${degradedLinks.length}`}
          subtitle="Switch ports running far below what the hardware can do"
        >
          <ul className="space-y-2">
            {degradedLinks.map((link, i) => (
              <li key={i} className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium">{link.switchName}</span>
                  <Pill mono>port {link.port}</Pill>
                  {link.portName && <Pill>{link.portName}</Pill>}
                  <StatusBadge tone="warning" label={formatSpeed(link.speedMbps)} />
                </div>
                <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
                  {link.explanation}
                </p>
              </li>
            ))}
          </ul>
        </Card>
      )}

      {unifi && unifi.devices.length > 0 && (
        <Card
          title={`Controller infrastructure — ${unifi.devices.length}`}
          subtitle={`${unifi.controllerHost} · site "${unifi.site}"`}
        >
          <div className="scroll-x">
            <table className="w-full min-w-[560px] border-collapse text-sm">
              <thead>
                <tr className="border-b text-left" style={{ borderColor: "var(--border)" }}>
                  {["Device", "Type", "Model", "Firmware", "Status"].map((label) => (
                    <th
                      key={label}
                      className="pb-2 pr-3 text-xs font-semibold"
                      style={{ color: "var(--text-secondary)" }}
                    >
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {unifi.devices.map((device, i) => {
                  const ports = device.ports ?? [];
                  return (
                    <Fragment key={i}>
                      <tr
                        className={ports.length === 0 ? "border-b" : ""}
                        style={{ borderColor: "var(--border)" }}
                      >
                        <td className="py-1.5 pr-3">
                          <span className="font-medium">{device.name}</span>
                          {device.ip && (
                            <p className="font-mono text-[11px] tabular" style={{ color: "var(--text-muted)" }}>
                              {device.ip}
                            </p>
                          )}
                        </td>
                        <td className="py-1.5 pr-3 text-xs">{device.kind}</td>
                        <td className="py-1.5 pr-3 font-mono text-xs">{device.model ?? "—"}</td>
                        <td className="py-1.5 pr-3 font-mono text-xs">{device.version ?? "—"}</td>
                        <td className="py-1.5">
                          <span className="flex flex-wrap gap-1.5">
                            {device.stateLabel && (
                              <StatusBadge tone="critical" label={device.stateLabel} />
                            )}
                            {!device.adopted && <StatusBadge tone="warning" label="Not adopted" />}
                            {device.upgradable && <StatusBadge tone="warning" label="Update available" />}
                            {device.adopted && !device.upgradable && !device.stateLabel && (
                              <StatusBadge tone="good" label="Up to date" />
                            )}
                          </span>
                        </td>
                      </tr>
                      {ports.length > 0 && (
                        <tr className="border-b" style={{ borderColor: "var(--border)" }}>
                          <td colSpan={5} className="pb-2">
                            <details>
                              <summary
                                className="cursor-pointer text-xs"
                                style={{ color: "var(--text-secondary)" }}
                              >
                                Ports — {ports.filter((p) => p.up).length} of {ports.length} up
                              </summary>
                              <ul className="mt-1.5 flex flex-wrap gap-1.5">
                                {ports.map((port) => (
                                  <li
                                    key={port.index}
                                    className="rounded border px-2 py-1 text-xs"
                                    style={{
                                      borderColor: "var(--border)",
                                      opacity: port.up ? 1 : 0.55,
                                    }}
                                  >
                                    <span className="font-mono font-semibold tabular">{port.index}</span>
                                    {port.name && <span className="ml-1.5">{port.name}</span>}
                                    <span
                                      className="ml-1.5 font-mono"
                                      style={{ color: "var(--text-secondary)" }}
                                    >
                                      {port.up ? formatSpeed(port.speedMbps) : "down"}
                                    </span>
                                    {port.poeWatts !== undefined && port.poeWatts > 0 && (
                                      <span
                                        className="ml-1.5 font-mono"
                                        style={{ color: "var(--text-secondary)" }}
                                      >
                                        ⚡{port.poeWatts.toFixed(1)} W
                                      </span>
                                    )}
                                  </li>
                                ))}
                              </ul>
                            </details>
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  );
                })}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {unifi && (unifi.neighborAps ?? []).length > 0 && (
        <Card
          title={`Nearby networks — ${
            unifi.neighborApTotal && unifi.neighborApTotal > unifi.neighborAps!.length
              ? `strongest ${unifi.neighborAps!.length} of ${unifi.neighborApTotal}`
              : unifi.neighborAps!.length
          }`}
          subtitle="Foreign access points overheard by your own radios — something no host scan can see"
        >
          <ul className="space-y-1.5">
            {unifi.neighborAps!.map((ap, i) => (
              <li
                key={i}
                className="flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2 text-sm"
                style={{
                  borderColor: ap.evilTwin ? "var(--status-critical)" : "var(--border)",
                }}
              >
                <span className="font-medium">{ap.ssid ?? "(hidden SSID)"}</span>
                {ap.evilTwin && (
                  <StatusBadge tone="critical" label="Broadcasts your SSID" />
                )}
                {ap.bssid && (
                  <span className="font-mono text-[11px] tabular" style={{ color: "var(--text-muted)" }}>
                    {ap.bssid}
                  </span>
                )}
                {ap.channel !== undefined && <Pill mono>ch {ap.channel}</Pill>}
                {ap.signalDbm !== undefined && <Pill mono>{ap.signalDbm} dBm</Pill>}
                {ap.security && <Pill>{ap.security}</Pill>}
                {ap.evilTwin && (
                  <p className="w-full text-xs" style={{ color: "var(--text-secondary)" }}>
                    A radio that is not one of your access points is broadcasting this site&apos;s
                    SSID. That is the evil-twin signature — worth confirming it is not a
                    neighbour&apos;s honest coincidence or your own second site.
                  </p>
                )}
              </li>
            ))}
          </ul>
        </Card>
      )}
    </div>
  );
}
