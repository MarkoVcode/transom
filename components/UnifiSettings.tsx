"use client";

import { useEffect, useState } from "react";
import { Button, Card, Icon, LoadingState, Spinner, StatusBadge } from "./ui";
import * as api from "@/lib/api";
import type { UnifiConfig } from "@/lib/types";

const EMPTY: UnifiConfig = {
  host: "",
  port: 443,
  site: "default",
  username: "",
  enabled: true,
};

export function UnifiSettings({ onChanged }: { onChanged?: () => void }) {
  const [config, setConfig] = useState<UnifiConfig>(EMPTY);
  const [password, setPassword] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);
  const [coverage, setCoverage] = useState<api.DiagnosticCoverage[] | null>(null);

  useEffect(() => {
    if (!api.isDesktop()) return;
    let cancelled = false;

    api
      .getUnifiConfig()
      .then((stored) => {
        if (cancelled) return;
        if (stored) setConfig(stored);
        setLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const update = <K extends keyof UnifiConfig>(key: K, value: UnifiConfig[K]) => {
    setConfig((current) => ({ ...current, [key]: value }));
    setResult(null);
  };

  const test = async () => {
    setBusy(true);
    setResult(null);
    try {
      const message = await api.testUnifiConnection(config, password || undefined);
      setResult({ ok: true, message });
      // A successful test pins the certificate, so reload to pick it up.
      const stored = await api.getUnifiConfig();
      if (stored) setConfig(stored);
      setPassword("");
      onChanged?.();
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  /* Which of the fields the diagnostics rely on this controller actually
   * sends. A detector can be correct and still never fire because the release
   * omits its field, and that is indistinguishable from "nothing is wrong"
   * unless it is stated. */
  const checkFields = async () => {
    setBusy(true);
    setResult(null);
    try {
      setCoverage(await api.checkControllerFields());
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    setResult(null);
    try {
      await api.saveUnifiConfig(config, password || undefined);
      setPassword("");
      setResult({ ok: true, message: "Saved." });
      onChanged?.();
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const disconnect = async () => {
    setBusy(true);
    try {
      await api.clearUnifiConfig();
      setConfig(EMPTY);
      setPassword("");
      setResult({ ok: true, message: "Disconnected and credentials removed." });
      onChanged?.();
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const field = (
    label: string,
    value: string,
    onChange: (v: string) => void,
    options: { type?: string; placeholder?: string; hint?: string } = {},
  ) => (
    <label className="block">
      <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
        {label}
      </span>
      <input
        type={options.type ?? "text"}
        value={value}
        placeholder={options.placeholder}
        onChange={(e) => onChange(e.target.value)}
        disabled={busy}
        className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
        style={{
          borderColor: "var(--border-strong)",
          background: "var(--surface-raised)",
          color: "var(--text-primary)",
        }}
      />
      {options.hint && (
        <span className="mt-1 block text-[11px]" style={{ color: "var(--text-muted)" }}>
          {options.hint}
        </span>
      )}
    </label>
  );

  return (
    <Card
      title="UniFi controller"
      subtitle="Adds physical location, controller names, and detection of devices the controller has never seen"
    >
      {!loaded ? (
        <LoadingState title="Loading controller settings…" />
      ) : (
        <div className="space-y-3">
          <div
            className="rounded-lg border p-3 text-xs"
            style={{ borderColor: "var(--border)", color: "var(--text-secondary)" }}
          >
            <strong style={{ color: "var(--text-primary)" }}>
              Create a read-only account first.
            </strong>{" "}
            In the UniFi console, add a <em>local</em> user with the <em>Viewer</em> role. This
            integration only ever reads, and a read-only credential cannot change your network even
            if it leaks. The password is stored in your operating system&apos;s keychain, never in
            scan snapshots.
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            {field("Controller address", config.host, (v) => update("host", v), {
              placeholder: "10.0.3.12",
              hint: "Must be a private address.",
            })}
            {field("Port", String(config.port), (v) => update("port", Number(v) || 443), {
              placeholder: "443",
              hint: "443 for UniFi OS, 8443 for a legacy controller.",
            })}
            {field("Site", config.site, (v) => update("site", v), {
              placeholder: "default",
            })}
            {field("Username", config.username, (v) => update("username", v), {
              placeholder: "viewer",
            })}
          </div>

          {field("Password", password, setPassword, {
            type: "password",
            placeholder: config.username ? "•••••••• (leave blank to keep stored)" : "",
            hint: "Stored in the OS keychain. Leave blank to keep the existing one.",
          })}

          <label className="flex items-center gap-2 text-xs" style={{ color: "var(--text-secondary)" }}>
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => update("enabled", e.target.checked)}
              disabled={busy}
            />
            Use the controller during scans
          </label>

          {config.fingerprint && (
            <div className="rounded-lg border p-2.5" style={{ borderColor: "var(--border)" }}>
              <div className="flex flex-wrap items-center gap-2">
                <StatusBadge tone="good" label="Certificate pinned" />
                <button
                  type="button"
                  onClick={() => update("fingerprint", undefined)}
                  disabled={busy}
                  className="text-[11px] hover:underline"
                  style={{ color: "var(--text-muted)" }}
                >
                  Clear pin
                </button>
              </div>
              <p className="mt-1 font-mono text-[10px] break-all" style={{ color: "var(--text-muted)" }}>
                {config.fingerprint}
              </p>
              <p className="mt-1 text-[11px]" style={{ color: "var(--text-secondary)" }}>
                Connections are refused if this changes. Clear it only after deliberately
                reinstalling the controller or regenerating its certificate.
              </p>
            </div>
          )}

          {result && (
            <p
              className="text-xs"
              style={{ color: result.ok ? "var(--success-text)" : "var(--status-critical)" }}
            >
              {result.message}
            </p>
          )}

          <div className="flex flex-wrap gap-2">
            <Button variant="primary" onClick={test} disabled={busy || !config.host || !config.username}>
              {busy ? (
                <>
                  <Spinner size={13} />
                  Contacting controller…
                </>
              ) : (
                "Test connection"
              )}
            </Button>
            <Button onClick={save} disabled={busy || !config.host || !config.username}>
              Save
            </Button>
            {config.host && (
              <Button onClick={checkFields} disabled={busy}>
                Check diagnostic fields
              </Button>
            )}
            {config.host && (
              <Button variant="danger" onClick={disconnect} disabled={busy}>
                Disconnect
              </Button>
            )}
          </div>

          {coverage && <FieldCoverageReport coverage={coverage} />}
        </div>
      )}
    </Card>
  );
}

/**
 * Which diagnostics this controller can support.
 *
 * A finding whose evidence this release does not report is *unavailable*, which
 * is a different answer from "nothing wrong" — and without saying so, an empty
 * result is indistinguishable from a clean one. Named by finding rather than by
 * JSON field, because that is what the absence actually costs the user.
 */
function FieldCoverageReport({ coverage }: { coverage: api.DiagnosticCoverage[] }) {
  const unavailable = coverage.filter((entry) => !entry.available);

  return (
    <div className="rounded-lg border p-3" style={{ borderColor: "var(--border)" }}>
      <p className="text-xs font-medium">
        {unavailable.length === 0
          ? "This controller supports every diagnostic."
          : `${unavailable.length} of ${coverage.length} diagnostics cannot run against this controller.`}
      </p>
      <ul className="mt-2 space-y-1">
        {coverage.map((entry) => (
          <li key={entry.finding} className="flex flex-wrap items-center gap-2 text-[11px]">
            <span
              aria-hidden
              style={{
                color: entry.available ? "var(--status-good)" : "var(--text-muted)",
                display: "inline-flex",
              }}
            >
              <Icon name={entry.available ? "dot" : "dotOutline"} size={9} />
            </span>
            <span style={{ color: "var(--text-secondary)" }}>{entry.finding}</span>
            {!entry.available && (
              <span className="font-mono" style={{ color: "var(--text-muted)" }}>
                needs {entry.missing.join(", ")}
              </span>
            )}
          </li>
        ))}
      </ul>
      {unavailable.length > 0 && (
        <p className="mt-2 text-[11px]" style={{ color: "var(--text-secondary)" }}>
          Those findings will stay silent here. That is a limit of this controller release, not a
          clean bill of health.
        </p>
      )}
    </div>
  );
}
