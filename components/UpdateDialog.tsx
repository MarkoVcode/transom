"use client";

import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "./ui";
import * as api from "@/lib/api";
import type { UpdateInfo } from "@/lib/types";

const formatBytes = (bytes: number): string => {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

/**
 * Startup update notice.
 *
 * Deliberately quiet. The check runs in Rust, fails silently when offline — this
 * app is often launched *because* the internet is broken — and the dialog only
 * appears when there is genuinely a newer release the user has not skipped.
 *
 * Where the installation can replace itself — everywhere except a `.deb`/`.rpm`,
 * which the package manager owns — the update is applied in place and the app
 * restarts. Otherwise this falls back to opening the release page.
 */
export function UpdateDialog({ info, onClose }: { info: UpdateInfo; onClose: () => void }) {
  const dialogRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [canInstall, setCanInstall] = useState(false);
  const [progress, setProgress] = useState<{ downloaded: number; total?: number } | null>(null);
  const [installing, setInstalling] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    api
      .updateInstallSupported()
      .then(setCanInstall)
      // A failure here just means the release page fallback is used.
      .catch(() => setCanInstall(false));
  }, []);

  // Focus the dialog so Escape works and screen readers announce it.
  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      // Escaping mid-install would hide a running download behind the app.
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  /** Opens the release page — the fallback path, and the only one for packages. */
  const download = async () => {
    setBusy(true);
    try {
      await openUrl(info.releaseUrl);
      onClose();
    } catch {
      // The permission is scoped to this repository; a failure here means the
      // release URL was unexpected, which is not worth an error dialog.
      onClose();
    }
  };

  /**
   * Applies the update in place. Never resolves on success: the process either
   * restarts or hands off to the platform installer and exits.
   */
  const install = async () => {
    setBusy(true);
    setFailed(null);
    setProgress({ downloaded: 0 });

    let unlisten: (() => void) | undefined;
    try {
      unlisten = await api.onUpdateProgress((event) => {
        if (event.done) setInstalling(true);
        else setProgress({ downloaded: event.downloaded, total: event.total });
      });

      await api.installUpdate();
    } catch (error) {
      // Verification failure, a broken connection, or a refused elevation
      // prompt all land here. The release page still works.
      setFailed(error instanceof Error ? error.message : String(error));
      setBusy(false);
      setInstalling(false);
      setProgress(null);
    } finally {
      unlisten?.();
    }
  };

  const skip = async () => {
    setBusy(true);
    try {
      await api.skipUpdateVersion(info.latestVersion);
    } catch {
      // Not being able to persist the preference is not worth interrupting for.
    }
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-6"
      style={{ background: "rgba(0,0,0,0.45)" }}
      onClick={(event) => {
        // A click on the backdrop must not abandon a running install.
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-title"
        className="w-full max-w-lg rounded-xl border shadow-xl"
        style={{ borderColor: "var(--border-strong)", background: "var(--surface-1)" }}
      >
        <header className="border-b px-5 py-4" style={{ borderColor: "var(--border)" }}>
          <h2 id="update-title" className="text-base font-semibold">
            Version {info.latestVersion} is available
          </h2>
          <p className="mt-1 text-sm" style={{ color: "var(--text-secondary)" }}>
            You have {info.currentVersion}.
            {info.publishedAt
              ? ` Released ${new Date(info.publishedAt).toLocaleDateString()}.`
              : ""}
          </p>
        </header>

        {info.releaseNotes && (
          <div className="max-h-64 overflow-y-auto px-5 py-4">
            <h3 className="mb-2 text-xs font-semibold" style={{ color: "var(--text-secondary)" }}>
              What&apos;s new
            </h3>
            <pre
              className="text-xs whitespace-pre-wrap"
              style={{ color: "var(--text-secondary)", fontFamily: "inherit" }}
            >
              {info.releaseNotes}
            </pre>
          </div>
        )}

        {(progress || installing) && (
          <div className="px-5 py-3" aria-live="polite">
            <p className="mb-2 text-xs" style={{ color: "var(--text-secondary)" }}>
              {installing
                ? "Installing — the app will restart."
                : progress?.total
                  ? `Downloading ${formatBytes(progress.downloaded)} of ${formatBytes(progress.total)}`
                  : `Downloading ${formatBytes(progress?.downloaded ?? 0)}`}
            </p>
            <div
              className="h-1.5 w-full overflow-hidden rounded-full"
              style={{ background: "var(--gridline)" }}
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={
                installing || !progress?.total
                  ? undefined
                  : Math.round((progress.downloaded / progress.total) * 100)
              }
            >
              <div
                className="h-full rounded-full transition-[width] duration-150"
                style={{
                  background: "var(--series-1)",
                  width:
                    installing || !progress?.total
                      ? "100%"
                      : `${Math.min(100, (progress.downloaded / progress.total) * 100)}%`,
                }}
              />
            </div>
          </div>
        )}

        {failed && (
          <div className="px-5 py-3" role="alert">
            <p className="text-xs" style={{ color: "var(--status-critical)" }}>
              The update could not be installed: {failed}
            </p>
            <p className="mt-1 text-xs" style={{ color: "var(--text-secondary)" }}>
              You can still download it manually.
            </p>
          </div>
        )}

        <footer
          className="flex flex-wrap items-center justify-between gap-2 border-t px-5 py-3"
          style={{ borderColor: "var(--border)" }}
        >
          <button
            type="button"
            onClick={skip}
            disabled={busy}
            className="text-xs hover:underline disabled:opacity-50"
            style={{ color: "var(--text-muted)" }}
          >
            Skip this version
          </button>

          <div className="flex items-center gap-2">
            <Button onClick={onClose} disabled={busy}>
              Later
            </Button>
            {canInstall && !failed ? (
              <Button variant="primary" onClick={install} disabled={busy}>
                {busy ? "Updating…" : "Update now"}
              </Button>
            ) : (
              <Button variant="primary" onClick={download} disabled={busy}>
                Download
              </Button>
            )}
          </div>
        </footer>

        <button ref={closeRef} className="sr-only" onClick={onClose} type="button">
          Close
        </button>
      </div>
    </div>
  );
}

/**
 * Runs the check once on mount and renders the dialog if there is something to
 * say. Returns nothing otherwise — no spinner, no error, no trace.
 */
export function UpdateGate() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    if (!api.isDesktop()) return;
    let cancelled = false;

    // Deliberately not awaited into any loading state: a slow or failed check
    // must never delay the window.
    api
      .checkForUpdate(false)
      .then((result) => {
        if (!cancelled && result?.updateAvailable) setInfo(result);
      })
      .catch(() => {
        // Offline is the expected case for this app. Silence is correct.
      });

    return () => {
      cancelled = true;
    };
  }, []);

  if (!info || dismissed) return null;
  return <UpdateDialog info={info} onClose={() => setDismissed(true)} />;
}
