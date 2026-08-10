import type { ReactNode } from "react";
import type { DeviceType } from "@/lib/types";

/** Shared presentational primitives. Kept server-safe (no hooks) so any panel can use them. */

export function Card({
  title,
  subtitle,
  actions,
  children,
  className = "",
}: {
  title?: string;
  subtitle?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={`min-w-0 rounded-xl border bg-[var(--surface-1)] ${className}`}
      style={{ borderColor: "var(--border)" }}
    >
      {(title || actions) && (
        <header
          className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3"
          style={{ borderColor: "var(--border)" }}
        >
          <div className="min-w-0">
            {title && <h2 className="text-sm font-semibold">{title}</h2>}
            {subtitle && (
              <p className="mt-0.5 text-xs" style={{ color: "var(--text-secondary)" }}>
                {subtitle}
              </p>
            )}
          </div>
          {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
        </header>
      )}
      <div className="min-w-0 p-4">{children}</div>
    </section>
  );
}

/**
 * One drawn icon set for the whole app.
 *
 * Everything here was a Unicode character — `◉` for the nav, `●▲■○` for status.
 * Those depend on whichever font the OS happens to resolve them in, which is
 * why the same screen did not look the same on Linux and Windows, and they
 * cannot be aligned or sized against the text they sit next to. A stroke path
 * inherits `currentColor`, lands on the same pixels everywhere, and is the same
 * vocabulary as the device-type icons below.
 *
 * 16×16 viewBox, ~1.5 units of padding, so every glyph has the same optical
 * weight at the sizes these are drawn at (9–15px).
 */
export type IconName =
  // Navigation.
  | "overview"
  | "devices"
  | "connectivity"
  | "wifi"
  | "controller"
  | "assistant"
  | "history"
  | "networks"
  | "location"
  | "host"
  | "setup"
  // Status. Filled, because at 10px a stroked outline reads as a smudge.
  | "dot"
  | "dotOutline"
  | "warning"
  | "critical"
  | "check"
  // Evidence for how another subnet was seen.
  | "route"
  | "hops"
  // Controls: disclosure and sort direction.
  | "chevronDown"
  | "chevronUp"
  | "chevronRight"
  | "arrowUp"
  | "arrowDown";

const ICONS: Record<IconName, ReactNode> = {
  overview: (
    <>
      <path d="M2.6 12.4a6.6 6.6 0 1 1 10.8 0" />
      <path d="M8 12.4 11 7.4" />
    </>
  ),
  devices: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1.5" />
      <path d="M2 6.4h12M6.2 6.4V13" />
    </>
  ),
  connectivity: (
    <>
      <path d="M2.4 5.6h11.2M10.8 2.8 13.6 5.6 10.8 8.4" />
      <path d="M13.6 10.4H2.4M5.2 7.6 2.4 10.4 5.2 13.2" />
    </>
  ),
  wifi: (
    <>
      <path d="M3.2 6.6A7 7 0 0 1 12.8 6.6" />
      <path d="M5.3 9A4 4 0 0 1 10.7 9" />
      <path d="M8 12.2h.01" />
    </>
  ),
  controller: (
    <>
      <rect x="6" y="1.8" width="4" height="3.4" rx="0.8" />
      <rect x="1.6" y="10.8" width="4" height="3.4" rx="0.8" />
      <rect x="10.4" y="10.8" width="4" height="3.4" rx="0.8" />
      <path d="M8 5.2v2.4M3.6 10.8V7.6h8.8v3.2" />
    </>
  ),
  assistant: <path d="M8 1.8 9.4 6.6 14.2 8 9.4 9.4 8 14.2 6.6 9.4 1.8 8 6.6 6.6z" />,
  history: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 4.6V8l2.4 1.5" />
    </>
  ),
  networks: (
    <>
      <path d="M8 1.8 14.2 5 8 8.2 1.8 5z" />
      <path d="M1.8 8 8 11.2 14.2 8" />
      <path d="M1.8 11 8 14.2 14.2 11" />
    </>
  ),
  location: (
    <>
      <path d="M8 14.2s4.6-4.4 4.6-7.6a4.6 4.6 0 1 0-9.2 0c0 3.2 4.6 7.6 4.6 7.6z" />
      <circle cx="8" cy="6.6" r="1.7" />
    </>
  ),
  host: (
    <>
      <path d="M2.6 6.8 8 2.4l5.4 4.4v5.7a1 1 0 0 1-1 1H3.6a1 1 0 0 1-1-1z" />
      <path d="M6.5 13.5V9.9h3v3.6" />
    </>
  ),
  setup: (
    <>
      <path d="M2 5.2h2.3M7.7 5.2H14" />
      <circle cx="6" cy="5.2" r="1.7" />
      <path d="M2 10.8h6.7M12.1 10.8H14" />
      <circle cx="10.4" cy="10.8" r="1.7" />
    </>
  ),
  dot: <circle cx="8" cy="8" r="3.6" fill="currentColor" stroke="none" />,
  dotOutline: <circle cx="8" cy="8" r="3.3" />,
  warning: <path d="M8 3.4 13.6 12.8H2.4z" fill="currentColor" stroke="none" />,
  critical: (
    <rect x="3.8" y="3.8" width="8.4" height="8.4" rx="1" fill="currentColor" stroke="none" />
  ),
  check: <path d="M3.4 8.3 6.5 11.4 12.6 5.2" />,
  route: (
    <>
      <path d="M1.8 12.4h2.6a3 3 0 0 0 3-3V6.6a3 3 0 0 1 3-3h3.4" />
      <path d="M11.8 1.9 14.2 3.6 11.8 5.3" />
    </>
  ),
  hops: <path d="M1.8 11.4 5.6 6.8 9 9.6 14.2 3.8" />,
  chevronDown: <path d="M4.2 6.4 8 10.2l3.8-3.8" />,
  chevronUp: <path d="M4.2 9.6 8 5.8l3.8 3.8" />,
  chevronRight: <path d="M6.4 4.2 10.2 8l-3.8 3.8" />,
  arrowUp: <path d="M8 12.8V3.6M4.6 7 8 3.6 11.4 7" />,
  arrowDown: <path d="M8 3.2v9.2M4.6 9 8 12.4 11.4 9" />,
};

/** The one `<svg>` every icon in the app is drawn in. */
function Glyph({
  size,
  className,
  children,
}: {
  size: number;
  className: string;
  children: ReactNode;
}) {
  return (
    <svg
      aria-hidden
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.3}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`shrink-0 ${className}`}
    >
      {children}
    </svg>
  );
}

export function Icon({
  name,
  size = 14,
  className = "",
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  return (
    <Glyph size={size} className={className}>
      {ICONS[name]}
    </Glyph>
  );
}

export type StatusTone = "good" | "warning" | "serious" | "critical" | "neutral";

const TONE_COLOR: Record<StatusTone, string> = {
  good: "var(--status-good)",
  warning: "var(--status-warning)",
  serious: "var(--status-serious)",
  critical: "var(--status-critical)",
  neutral: "var(--text-muted)",
};

const TONE_ICON: Record<StatusTone, IconName> = {
  good: "dot",
  warning: "warning",
  serious: "warning",
  critical: "critical",
  neutral: "dotOutline",
};

/**
 * The status mark on its own, for the places that build their own row rather
 * than using [`StatusBadge`] — a warnings list, a capability line. Takes the
 * colour from the caller, since those sites choose it from their own state.
 */
export function StatusMark({
  tone,
  size = 11,
  className = "",
}: {
  tone: StatusTone;
  size?: number;
  className?: string;
}) {
  return (
    <span aria-hidden style={{ color: TONE_COLOR[tone], display: "inline-flex" }}>
      <Icon name={TONE_ICON[tone]} size={size} className={className} />
    </span>
  );
}

/**
 * Status is never carried by color alone — every badge pairs the swatch with an
 * icon and a text label, which is what the sub-3:1 `warning` step requires on a
 * light surface.
 */
export function StatusBadge({ tone, label }: { tone: StatusTone; label: string }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium"
      style={{ borderColor: "var(--border)", color: "var(--text-primary)" }}
    >
      <StatusMark tone={tone} size={9} />
      {label}
    </span>
  );
}

export function Pill({ children, mono = false }: { children: ReactNode; mono?: boolean }) {
  return (
    <span
      className={`inline-flex items-center rounded border px-1.5 py-0.5 text-xs ${mono ? "font-mono tabular" : ""}`}
      style={{ borderColor: "var(--border)", color: "var(--text-secondary)" }}
    >
      {children}
    </span>
  );
}

/**
 * Device types get an icon plus a written label rather than a color code — with
 * eleven categories, cycling hues would be unreadable and meaningless.
 */
export const DEVICE_TYPE_META: Record<DeviceType, { label: string }> = {
  // Covers switches and APs too — the vendor signal cannot tell them apart, and
  // labelling a UniFi switch "Router" would be wrong.
  router: { label: "Network gear" },
  phone: { label: "Phone" },
  computer: { label: "Computer" },
  iot: { label: "IoT" },
  media: { label: "Media" },
  printer: { label: "Printer" },
  nas: { label: "Storage" },
  camera: { label: "Camera" },
  tv: { label: "TV" },
  server: { label: "Server" },
  unknown: { label: "Unknown" },
};

/**
 * Drawn rather than typed. These were emoji, which put full-colour glyphs in a
 * table whose every other mark — the nav, the status dots — is a monochrome
 * line at the text colour, and which each platform renders in its own house
 * style, so the same table did not even look the same on Linux and Windows.
 *
 * A stroke path inherits `currentColor` and therefore the theme, and is
 * identical everywhere because it depends on no font being installed.
 */
const DEVICE_TYPE_PATHS: Record<DeviceType, ReactNode> = {
  router: (
    <>
      <rect x="2" y="9.5" width="12" height="4.5" rx="1.2" />
      <path d="M4.6 11.8h.01" />
      <path d="M6 7.3A2.6 2.6 0 0 1 10 7.3" />
      <path d="M4.2 5.8A5 5 0 0 1 11.8 5.8" />
    </>
  ),
  phone: (
    <>
      <rect x="5" y="1.8" width="6" height="12.4" rx="1.5" />
      <path d="M7 12.2h2" />
    </>
  ),
  computer: (
    <>
      <rect x="2.5" y="3" width="11" height="7.5" rx="1" />
      <path d="M1.5 13h13" />
    </>
  ),
  iot: (
    <>
      <rect x="4.5" y="4.5" width="7" height="7" rx="1" />
      <path d="M6.5 1.8v2.7M9.5 1.8v2.7M6.5 11.5v2.7M9.5 11.5v2.7" />
      <path d="M1.8 6.5h2.7M1.8 9.5h2.7M11.5 6.5h2.7M11.5 9.5h2.7" />
    </>
  ),
  media: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M6.6 5.9v4.2l3.6-2.1z" />
    </>
  ),
  printer: (
    <>
      <path d="M4.5 6V2.5h7V6" />
      <rect x="2" y="6" width="12" height="5" rx="1" />
      <path d="M4.5 9.5h7v4h-7z" />
    </>
  ),
  nas: (
    <>
      <rect x="2" y="3" width="12" height="4" rx="1" />
      <rect x="2" y="9" width="12" height="4" rx="1" />
      <path d="M4.5 5h.01M4.5 11h.01" />
    </>
  ),
  camera: (
    <>
      <path d="M5.6 4.2 6.6 2.5h2.8l1 1.7" />
      <rect x="1.8" y="4.2" width="12.4" height="9" rx="1.5" />
      <circle cx="8" cy="8.7" r="2.5" />
    </>
  ),
  // Antenna rather than a stand: a screen over a pedestal is indistinguishable
  // from the laptop at 14px, which is the only size this is ever drawn at.
  tv: (
    <>
      <rect x="1.8" y="5.6" width="12.4" height="8.2" rx="1.2" />
      <path d="M11 1.9 8 5.2 5 1.9" />
    </>
  ),
  server: (
    <>
      <rect x="3.5" y="1.8" width="9" height="12.4" rx="1.2" />
      <path d="M3.5 6h9M3.5 10h9" />
      <path d="M10 3.9h.01M10 8h.01M10 12.1h.01" />
    </>
  ),
  // Dashed: nothing about this device was determined, which is a different
  // statement from any of the shapes above.
  unknown: <circle cx="8" cy="8" r="5.8" strokeDasharray="2.2 2.2" />,
};

export function DeviceTypeIcon({
  type,
  size = 14,
  className = "",
}: {
  type: DeviceType;
  size?: number;
  className?: string;
}) {
  return (
    <Glyph size={size} className={className}>
      {DEVICE_TYPE_PATHS[type]}
    </Glyph>
  );
}

export function DeviceTypeBadge({ type }: { type: DeviceType }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 text-xs whitespace-nowrap"
      style={{ color: "var(--text-secondary)" }}
    >
      <DeviceTypeIcon type={type} />
      <span>{DEVICE_TYPE_META[type].label}</span>
    </span>
  );
}

export function Button({
  children,
  onClick,
  variant = "secondary",
  disabled,
  title,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  variant?: "primary" | "secondary" | "danger";
  disabled?: boolean;
  title?: string;
  type?: "button" | "submit";
}) {
  const base =
    "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50";

  const styles: Record<string, React.CSSProperties> = {
    primary: { background: "var(--series-1)", color: "#ffffff" },
    secondary: {
      background: "transparent",
      color: "var(--text-primary)",
      border: "1px solid var(--border-strong)",
    },
    danger: {
      background: "transparent",
      color: "var(--status-critical)",
      border: "1px solid var(--status-critical)",
    },
  };

  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={base}
      style={styles[variant]}
    >
      {children}
    </button>
  );
}

/**
 * Work-in-progress indicator.
 *
 * Paired with text everywhere it is used: a bare spinner says "wait" but never
 * what for, and the waits here (a sweep, a controller that answers slowly) are
 * long enough that the difference matters.
 */
export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <span
      aria-hidden
      className="animate-spin-soft inline-block shrink-0 rounded-full border-2 border-solid align-middle"
      style={{
        width: size,
        height: size,
        borderColor: "var(--gridline)",
        borderTopColor: "var(--series-1)",
      }}
    />
  );
}

/**
 * The counterpart to `EmptyState` for the case that is *not* empty, only not
 * here yet. Rendering "nothing found" while data is still loading is a lie the
 * user can only disprove by navigating away and back.
 */
export function LoadingState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="py-10 text-center" role="status" aria-live="polite">
      <Spinner size={22} />
      <p className="mt-3 text-sm font-medium">{title}</p>
      {hint && (
        <p className="mx-auto mt-1 max-w-md text-xs" style={{ color: "var(--text-secondary)" }}>
          {hint}
        </p>
      )}
    </div>
  );
}

export function EmptyState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="py-10 text-center">
      <p className="text-sm font-medium">{title}</p>
      {hint && (
        <p className="mx-auto mt-1 max-w-md text-xs" style={{ color: "var(--text-secondary)" }}>
          {hint}
        </p>
      )}
    </div>
  );
}

export function KeyValue({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5 py-1">
      <dt className="w-40 shrink-0 text-xs" style={{ color: "var(--text-secondary)" }}>
        {label}
      </dt>
      <dd className={`min-w-0 flex-1 text-sm break-words ${mono ? "font-mono tabular" : ""}`}>{value}</dd>
    </div>
  );
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${Math.round(seconds % 60)}s`;
}

export function formatRelativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 60_000) return "just now";
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** Latency thresholds tuned for a LAN gateway, where >20 ms already means trouble. */
export function latencyTone(ms: number | undefined, isLocal: boolean): StatusTone {
  if (ms === undefined) return "neutral";
  const limits = isLocal ? [5, 20, 50] : [30, 80, 150];
  if (ms <= limits[0]) return "good";
  if (ms <= limits[1]) return "warning";
  if (ms <= limits[2]) return "serious";
  return "critical";
}

export function lossTone(percent: number): StatusTone {
  if (percent === 0) return "good";
  if (percent < 2) return "warning";
  if (percent < 10) return "serious";
  return "critical";
}

export function signalTone(percent: number): StatusTone {
  if (percent >= 70) return "good";
  if (percent >= 50) return "warning";
  if (percent >= 30) return "serious";
  return "critical";
}
