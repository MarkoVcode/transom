"use client";

import { useState } from "react";
import { Button, Card, EmptyState, LoadingState, Pill, StatusBadge } from "./ui";
import * as api from "@/lib/api";
import type { AdjacentSubnet, SubnetEvidence } from "@/lib/types";

/**
 * Neighbouring subnets found from evidence already collected, offered as
 * networks to track.
 *
 * Deliberately a list of *proposals*, never an action. Adding a network here
 * only records it; it is not selected and not scanned, because a scan that
 * started itself against a range the user has not looked at is the same class of
 * surprise as a scan filing itself under the wrong network.
 *
 * The evidence is shown rather than summarised into a confidence score. "A
 * router at 10.0.2.1 answered at hop 2" is something the user can check against
 * what they know of their own network; "87% confident" is not.
 */
export function AdjacentNetworks({
  subnets,
  onChanged,
  onReload,
}: {
  /** `null` while the first read is in flight. */
  subnets: AdjacentSubnet[] | null;
  onChanged: () => void | Promise<void>;
  onReload: () => void | Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [names, setNames] = useState<Record<string, string>>({});

  const add = async (subnet: AdjacentSubnet) => {
    setBusy(subnet.cidr);
    setError(null);
    try {
      // `select: false` — recording a network must not move the user off the one
      // they are working on.
      await api.ensureNetworkForRange(
        subnet.cidr,
        names[subnet.cidr]?.trim() || undefined,
        false,
      );
      await onChanged();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const untracked = subnets?.filter((subnet) => !subnet.alreadyTracked) ?? [];

  return (
    <Card
      title="Other networks seen from here"
      subtitle="Subnets something on this machine can demonstrably reach, and the evidence for each"
      actions={
        <Button onClick={() => onReload()} disabled={busy !== null || subnets === null}>
          Re-check
        </Button>
      }
    >
      {subnets === null ? (
        <LoadingState title="Looking for neighbouring subnets…" />
      ) : untracked.length === 0 ? (
        <EmptyState
          title={
            subnets.length === 0 ? "Nothing else found" : "Everything found is already tracked"
          }
          hint={
            subnets.length === 0
              ? "Only subnets with proof of reachability are listed — a route this machine holds, a subnet the controller declares, or an address that answered a probe. Nothing is guessed at from neighbouring addresses. Running a scan gives this more to work with."
              : "Every subnet found is covered by a network you already have."
          }
        />
      ) : (
        <ul className="space-y-3">
          {untracked.map((subnet) => (
            <li
              key={subnet.cidr}
              className="rounded-lg border p-3"
              style={{ borderColor: "var(--border)" }}
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-mono text-sm font-medium tabular">{subnet.cidr}</span>
                {subnet.prefixAssumed ? (
                  <StatusBadge tone="warning" label="Size assumed" />
                ) : (
                  <StatusBadge tone="good" label="Size confirmed" />
                )}
                <Pill>
                  {subnet.evidence.length === 1 ? "1 sign" : `${subnet.evidence.length} signs`}
                </Pill>
              </div>

              <ul className="mt-2 space-y-1">
                {subnet.evidence.map((item, index) => (
                  <li
                    key={index}
                    className="flex items-start gap-2 text-xs"
                    style={{ color: "var(--text-secondary)" }}
                  >
                    <span aria-hidden style={{ color: "var(--text-muted)" }}>
                      {evidenceIcon(item)}
                    </span>
                    <span className="min-w-0">{explain(item)}</span>
                  </li>
                ))}
              </ul>

              {subnet.prefixAssumed && (
                <p className="mt-2 text-[11px]" style={{ color: "var(--text-muted)" }}>
                  Only the address is proven, not the mask — /24 is a guess. Correct it after
                  adding if the subnet is a different size.
                </p>
              )}

              <div className="mt-2.5 flex flex-wrap items-end gap-2">
                <label className="min-w-40 flex-1">
                  <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
                    Name
                  </span>
                  <input
                    type="text"
                    value={names[subnet.cidr] ?? ""}
                    onChange={(e) =>
                      setNames((current) => ({ ...current, [subnet.cidr]: e.target.value }))
                    }
                    placeholder={subnet.cidr}
                    disabled={busy !== null}
                    className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
                    style={{
                      borderColor: "var(--border-strong)",
                      background: "var(--surface-raised)",
                      color: "var(--text-primary)",
                    }}
                  />
                </label>
                <Button onClick={() => add(subnet)} disabled={busy !== null}>
                  {busy === subnet.cidr ? "Adding…" : "Track this network"}
                </Button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {error && (
        <p className="mt-3 text-xs" style={{ color: "var(--status-critical)" }}>
          {error}
        </p>
      )}

      <p className="mt-3 text-xs" style={{ color: "var(--text-muted)" }}>
        Adding one records it and nothing more — it is not selected and not scanned. Switch to it
        when you want to scan it.
      </p>
    </Card>
  );
}

function evidenceIcon(evidence: SubnetEvidence): string {
  switch (evidence.kind) {
    case "route":
      return "⇥";
    case "controller":
      return "⊞";
    case "responder":
      return "◉";
    case "traceHop":
      return "↭";
  }
}

/** Mirrors `SubnetEvidence::explain` in the engine. */
function explain(evidence: SubnetEvidence): string {
  switch (evidence.kind) {
    case "route":
      return evidence.via
        ? `This machine routes to it via ${evidence.via} on ${evidence.dev}.`
        : `This machine has a direct route to it on ${evidence.dev}.`;
    case "controller":
      return evidence.vlan !== undefined
        ? `The controller defines it as “${evidence.name}” on VLAN ${evidence.vlan}.`
        : `The controller defines it as “${evidence.name}”.`;
    case "responder":
      return `${evidence.ip} answered during the last scan (${evidence.detail}).`;
    case "traceHop":
      return `A router at ${evidence.ip} answered at hop ${evidence.hop} on the way out.`;
  }
}

/**
 * The finding, wherever the user happens to be.
 *
 * A card on the Networks page is not a suggestion — it only reaches someone who
 * already went looking. A scan that saw a device on a subnet nothing tracks has
 * discovered something, and should say so on the page the user is actually
 * reading.
 */
export function AdjacentNotice({
  subnets,
  onOpen,
  onDismiss,
}: {
  subnets: AdjacentSubnet[];
  onOpen: () => void;
  onDismiss: () => void;
}) {
  if (subnets.length === 0) return null;

  return (
    <div
      className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-xl border px-4 py-3"
      style={{ borderColor: "var(--status-warning)", background: "var(--surface-1)" }}
    >
      <div className="min-w-0">
        <p className="text-sm font-medium">
          {subnets.length === 1
            ? "Another network was seen from here"
            : `${subnets.length} other networks were seen from here`}
        </p>
        <p className="mt-0.5 text-xs" style={{ color: "var(--text-secondary)" }}>
          <span className="font-mono tabular">
            {subnets.map((subnet) => subnet.cidr).join(" · ")}
          </span>{" "}
          — reachable from this machine, and no saved network covers{" "}
          {subnets.length === 1 ? "it" : "them"}.
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button onClick={onDismiss}>Not now</Button>
        <Button variant="primary" onClick={onOpen}>
          Review
        </Button>
      </div>
    </div>
  );
}
