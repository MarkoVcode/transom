"use client";

import { useEffect, useState } from "react";
import { Button, Card, LoadingState, Spinner, StatusBadge } from "./ui";
import * as api from "@/lib/api";
import type { AssistConfig, ProviderKind } from "@/lib/types";

const EMPTY: AssistConfig = {
  provider: "anthropic",
  redact: true,
  enabled: false,
};

/**
 * Which model the troubleshooting assistant uses, and what it may be told.
 *
 * System-scoped rather than per network: the choice of model is about this
 * machine and its operator, not the site being diagnosed. The key goes to the
 * OS keychain and is never read back — the same handling as the controller
 * password, for the same reason.
 */
export function AssistantSettings({
  onChanged,
}: {
  /** Saving or disconnecting decides whether the Assistant page is usable, and
   *  that is read one level up — so say when it changes rather than leaving the
   *  nav to catch up on the next launch. */
  onChanged?: () => void;
} = {}) {
  const [config, setConfig] = useState<AssistConfig>(EMPTY);
  const [hasKey, setHasKey] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    if (!api.isDesktop()) return;
    let cancelled = false;

    api
      .getAssistConfig()
      .then((stored) => {
        if (cancelled) return;
        const { hasKey: stored_key, ...rest } = stored;
        setConfig(rest);
        setHasKey(stored_key);
        setLoaded(true);
      })
      .catch(() => {
        if (!cancelled) setLoaded(true);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const update = <K extends keyof AssistConfig>(key: K, value: AssistConfig[K]) => {
    setConfig((current) => ({ ...current, [key]: value }));
    setResult(null);
  };

  const cloud = config.provider === "anthropic";
  /* The cloud provider cannot run without a key; the local one never needs one. */
  const ready = !cloud || hasKey || apiKey.trim().length > 0;

  const run = async (action: () => Promise<string | void>) => {
    setBusy(true);
    setResult(null);
    try {
      const message = await action();
      setResult({ ok: true, message: message || "Saved." });
    } catch (error) {
      setResult({ ok: false, message: String(error) });
    } finally {
      setBusy(false);
    }
  };

  const test = () =>
    run(async () => {
      const message = await api.testAssistConnection(config, apiKey || undefined);
      return message;
    });

  const save = () =>
    run(async () => {
      await api.saveAssistConfig(config, apiKey || undefined);
      if (apiKey.trim()) setHasKey(true);
      setApiKey("");
      onChanged?.();
    });

  const disconnect = () =>
    run(async () => {
      await api.clearAssistConfig();
      setConfig(EMPTY);
      setHasKey(false);
      setApiKey("");
      onChanged?.();
      return "Disconnected and the key removed.";
    });

  const field = (
    label: string,
    value: string,
    onChange: (v: string) => void,
    options: { type?: string; placeholder?: string; hint?: string; mono?: boolean } = {},
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
        className={`mt-1 w-full rounded-lg border px-2 py-1.5 text-sm ${options.mono ? "font-mono" : ""}`}
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
      title="Troubleshooting assistant"
      subtitle="Describe a symptom in your own words; the assistant investigates and proposes a fix"
    >
      {!loaded ? (
        <LoadingState title="Loading assistant settings…" />
      ) : (
        <div className="space-y-3">
          <div
            className="rounded-lg border p-3 text-xs"
            style={{ borderColor: "var(--border)", color: "var(--text-secondary)" }}
          >
            <strong style={{ color: "var(--text-primary)" }}>
              The model never measures anything.
            </strong>{" "}
            It decides what to check and explains what the readings mean; every number comes from
            this app&apos;s own probes. It reads only — it cannot change your network.
          </div>

          <label className="block">
            <span className="text-xs" style={{ color: "var(--text-secondary)" }}>
              Where the model runs
            </span>
            <select
              value={config.provider}
              onChange={(e) => update("provider", e.target.value as ProviderKind)}
              disabled={busy}
              className="mt-1 w-full rounded-lg border px-2 py-1.5 text-sm"
              style={{
                borderColor: "var(--border-strong)",
                background: "var(--surface-raised)",
                color: "var(--text-primary)",
              }}
            >
              <option value="anthropic">Anthropic (cloud) — best reasoning</option>
              <option value="ollama">Ollama (local) — nothing leaves this machine</option>
            </select>
            <span className="mt-1 block text-[11px]" style={{ color: "var(--text-muted)" }}>
              {cloud
                ? "Evidence is sent to Anthropic. Local models are weaker at the multi-step tool use this relies on, so the cloud option gives markedly better diagnoses."
                : "Requires Ollama running locally. Nothing is sent anywhere, at the cost of a weaker diagnosis — local models handle multi-step tool use less reliably."}
            </span>
          </label>

          {cloud ? (
            <>
              {field("API key", apiKey, setApiKey, {
                type: "password",
                placeholder: hasKey ? "•••••••• (leave blank to keep stored)" : "sk-ant-…",
                hint: "Stored in your operating system's keychain, never in a settings file.",
              })}
              {field("Model", config.model ?? "", (v) => update("model", v || undefined), {
                placeholder: "claude-opus-5",
                mono: true,
                hint: "Leave blank for the default.",
              })}
            </>
          ) : (
            <>
              {field("Ollama address", config.endpoint ?? "", (v) => update("endpoint", v || undefined), {
                placeholder: "http://127.0.0.1:11434",
                mono: true,
              })}
              {field("Model", config.model ?? "", (v) => update("model", v || undefined), {
                placeholder: "llama3.1",
                mono: true,
                hint: "Must be pulled already, and must support tool calling.",
              })}
            </>
          )}

          <label className="flex items-start gap-2 text-xs" style={{ color: "var(--text-secondary)" }}>
            <input
              type="checkbox"
              checked={config.redact}
              onChange={(e) => update("redact", e.target.checked)}
              disabled={busy}
              className="mt-0.5"
            />
            <span>
              Replace identifiers before sending
              <span className="mt-0.5 block text-[11px]" style={{ color: "var(--text-muted)" }}>
                MACs, hostnames, Wi-Fi names and your public IP become stable placeholders; the
                mapping stays on this machine. Vendors, models, port numbers, speeds, signal levels
                and error counts pass through — they carry the diagnosis and identify nobody.
                {!config.redact && cloud && " With this off, your device names leave the machine."}
              </span>
            </span>
          </label>

          <label className="flex items-center gap-2 text-xs" style={{ color: "var(--text-secondary)" }}>
            <input
              type="checkbox"
              checked={config.enabled}
              onChange={(e) => update("enabled", e.target.checked)}
              disabled={busy}
            />
            Enable the assistant
          </label>

          {config.enabled && !ready && (
            <p className="text-xs" style={{ color: "var(--status-warning)" }}>
              An API key is needed before the cloud model can be used.
            </p>
          )}

          {result && (
            <p
              className="text-xs"
              style={{ color: result.ok ? "var(--success-text)" : "var(--status-critical)" }}
            >
              {result.message}
            </p>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Button variant="primary" onClick={test} disabled={busy || !ready}>
              {busy ? (
                <>
                  <Spinner size={13} />
                  Contacting model…
                </>
              ) : (
                "Test connection"
              )}
            </Button>
            <Button onClick={save} disabled={busy}>
              Save
            </Button>
            <Button variant="danger" onClick={disconnect} disabled={busy}>
              Disconnect
            </Button>
            {hasKey && <StatusBadge tone="good" label="Key stored" />}
          </div>
        </div>
      )}
    </Card>
  );
}
