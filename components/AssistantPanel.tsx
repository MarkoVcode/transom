"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Card, EmptyState, Icon, LoadingState, Pill, Spinner, StatusBadge, StatusMark } from "./ui";
import type { StatusTone } from "./ui";
import * as api from "@/lib/api";
import type {
  AssistEvent,
  CaseSummary,
  Evidence,
  Remedy,
  TroubleshootingCase,
} from "@/lib/types";

/**
 * Describe a symptom, get an evidence-cited answer.
 *
 * The page is built around one claim it has to keep: every number here was
 * measured by this app, not produced by the model. That is why each finding and
 * remedy shows the readings it rests on, and why tool calls are visible at all
 * — a diagnosis you cannot check is just a confident sentence.
 */
export function AssistantPanel({
  networkId,
  networkName,
  configured,
  onOpenSetup,
}: {
  networkId?: string;
  networkName?: string;
  /** Whether a model is configured and switched on. */
  configured: boolean;
  onOpenSetup: () => void;
}) {
  const [cases, setCases] = useState<CaseSummary[] | null>(null);
  const [openCase, setOpenCase] = useState<TroubleshootingCase | null>(null);
  const [symptom, setSymptom] = useState("");
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  /* Live progress, kept separate from the stored case: the case file is only
   * written between steps, so without this the page would sit silent through
   * the slowest parts of the work. */
  const [live, setLive] = useState<AssistEvent[]>([]);
  const [running, setRunning] = useState(false);
  const [reproduce, setReproduce] = useState<{ seconds: number; reason: string } | null>(null);

  /* Read inside the event handler, which is registered once and would
   * otherwise close over the case that was open when it was created. */
  const openCaseRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    openCaseRef.current = openCase?.id;
  }, [openCase]);

  const reload = useCallback(
    async (selectId?: string) => {
      if (!api.isDesktop()) return;
      try {
        const list = await api.listCases();
        setCases(list);
        const wanted = selectId ?? openCaseRef.current ?? list[0]?.id;
        setOpenCase(wanted ? await api.getCase(wanted) : null);
      } catch (err) {
        setError(String(err));
      }
    },
    [],
  );

  /* No reset needed on a network change: this panel is keyed by network in
   * `DesktopApp`, so switching remounts it and nothing of the previous
   * network's cases survives. */
  useEffect(() => {
    void (async () => {
      await reload();
    })();
  }, [reload]);

  useEffect(() => {
    if (!api.isDesktop()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      const stop = await api.onAssistEvent((event) => {
        // Every event carries the network it belongs to, so a diagnosis
        // running under one network cannot narrate the page of another — the
        // same rule scan progress follows.
        if (networkId && event.networkId !== networkId) return;
        if (openCaseRef.current && event.caseId !== openCaseRef.current) return;

        setLive((current) => [...current, event]);

        if (event.type === "reproduceNow") {
          setReproduce({ seconds: event.seconds, reason: event.reason });
        }
        if (event.type === "toolStarted" || event.type === "said") {
          setReproduce(null);
        }
        if (event.type === "finished" || event.type === "asked") {
          setRunning(false);
          setReproduce(null);
          void reload(event.caseId);
        }
      });
      if (cancelled) stop();
      else unlisten = stop;

      // A window reopened mid-diagnosis should rejoin it rather than look
      // idle. `getCase` is scoped to the selected network, so a diagnosis
      // running under a different one is correctly not adopted here.
      try {
        const id = await api.getRunningCase();
        const found = id ? await api.getCase(id) : null;
        if (!cancelled && id && found) {
          openCaseRef.current = id;
          setRunning(true);
          void reload(id);
        }
      } catch {
        /* Nothing running is the ordinary case, not an error. */
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [reload, networkId]);

  const start = async () => {
    if (!symptom.trim()) return;
    setBusy(true);
    setError(null);
    setLive([]);
    // Cleared before the call, not after: the diagnosis starts the moment the
    // command returns, and events arriving while this still held the previous
    // case's id would be filtered out as somebody else's work.
    openCaseRef.current = undefined;
    try {
      const id = await api.startCase(symptom);
      setSymptom("");
      setRunning(true);
      openCaseRef.current = id;
      await reload(id);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const reply = async (text: string) => {
    if (!openCase || !text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await api.answerCase(openCase.id, text);
      setAnswer("");
      setRunning(true);
      await reload(openCase.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    try {
      await api.cancelCase();
    } catch (err) {
      setError(String(err));
    }
  };

  const remove = async (id: string) => {
    try {
      await api.deleteCase(id);
      openCaseRef.current = undefined;
      await reload();
    } catch (err) {
      setError(String(err));
    }
  };

  if (!api.isDesktop()) {
    return (
      <Card title="Troubleshooting assistant">
        <EmptyState title="Only available in the desktop app" />
      </Card>
    );
  }

  if (!configured) {
    return (
      <Card
        title="Troubleshooting assistant"
        subtitle="Describe a problem in your own words and get a diagnosis backed by measurements"
      >
        <EmptyState
          title="No model is connected yet"
          hint="Choose a model under Setup & Status — a cloud one for the best reasoning, or a local one if nothing may leave this machine."
        />
        <div className="mt-3">
          <Button variant="primary" onClick={onOpenSetup}>
            Open Setup &amp; Status
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <div className="space-y-4">
      <Card
        title="Describe the problem"
        subtitle={
          networkName
            ? `Diagnosed against “${networkName}” and its last scan`
            : "Diagnosed against this network and its last scan"
        }
      >
        <textarea
          value={symptom}
          onChange={(e) => setSymptom(e.target.value)}
          disabled={busy || running}
          rows={3}
          placeholder="e.g. When I upload a 30 MB file from my laptop to the NAS, the whole network freezes and the transfer crawls."
          className="w-full rounded-lg border px-3 py-2 text-sm"
          style={{
            borderColor: "var(--border-strong)",
            background: "var(--surface-raised)",
            color: "var(--text-primary)",
          }}
        />
        <p className="mt-2 text-[11px]" style={{ color: "var(--text-muted)" }}>
          Say what you did, what happened, and which machines were involved. The assistant measures
          rather than guesses, so it may ask you to reproduce the problem while it watches.
        </p>

        {error && (
          <p className="mt-2 text-xs" style={{ color: "var(--status-critical)" }}>
            {error}
          </p>
        )}

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button
            variant="primary"
            onClick={start}
            disabled={busy || running || !symptom.trim()}
          >
            {busy || running ? (
              <>
                <Spinner size={13} />
                Working…
              </>
            ) : (
              "Diagnose"
            )}
          </Button>
          {running && (
            <Button variant="danger" onClick={stop}>
              Stop
            </Button>
          )}
        </div>
      </Card>

      {reproduce && <ReproduceBanner seconds={reproduce.seconds} reason={reproduce.reason} />}

      {openCase && (
        <CaseView
          openCase={openCase}
          live={live}
          running={running}
          busy={busy}
          answer={answer}
          onAnswerChange={setAnswer}
          onReply={reply}
        />
      )}

      <CaseList
        cases={cases}
        openId={openCase?.id}
        onOpen={(id) => {
          setLive([]);
          openCaseRef.current = id;
          void reload(id);
        }}
        onDelete={remove}
      />
    </div>
  );
}

/**
 * The one moment the user has to act.
 *
 * A fault that only exists under load is invisible in an idle reading, and the
 * app cannot generate the user's traffic for them. If this instruction is
 * missed the measurement is wasted, so it is loud and it says how long.
 */
function ReproduceBanner({ seconds, reason }: { seconds: number; reason: string }) {
  return (
    <div
      className="flex items-start gap-3 rounded-xl border p-4"
      style={{
        borderColor: "var(--status-warning)",
        background: "var(--surface-raised)",
      }}
    >
      <Spinner size={18} />
      <div className="min-w-0">
        <p className="text-sm font-semibold">Reproduce the problem now</p>
        <p className="mt-1 text-sm" style={{ color: "var(--text-secondary)" }}>
          {reason}
        </p>
        <p className="mt-1 text-xs" style={{ color: "var(--text-muted)" }}>
          The assistant is watching the switch and radio counters for the next {seconds} seconds and
          will report what changed.
        </p>
      </div>
    </div>
  );
}

function CaseView({
  openCase,
  live,
  running,
  busy,
  answer,
  onAnswerChange,
  onReply,
}: {
  openCase: TroubleshootingCase;
  live: AssistEvent[];
  running: boolean;
  busy: boolean;
  answer: string;
  onAnswerChange: (value: string) => void;
  onReply: (text: string) => void;
}) {
  const evidenceById = useMemo(
    () => new Map(openCase.evidence.map((item) => [item.id, item])),
    [openCase.evidence],
  );

  return (
    <Card
      title={openCase.symptom}
      subtitle={`Opened ${new Date(openCase.createdAt).toLocaleString()} · ${openCase.evidence.length} measurement(s)`}
      actions={<StatusBadge {...statusBadge(openCase, running)} />}
    >
      <div className="space-y-4">
        {openCase.question && (
          <QuestionBox
            question={openCase.question}
            busy={busy}
            answer={answer}
            onAnswerChange={onAnswerChange}
            onReply={onReply}
          />
        )}

        {openCase.remedy && (
          <RemedyCard remedy={openCase.remedy} evidenceById={evidenceById} />
        )}

        {!openCase.remedy && openCase.note && (
          <div
            className="rounded-lg border p-3 text-sm"
            style={{ borderColor: "var(--border)", color: "var(--text-secondary)" }}
          >
            {openCase.note}
          </div>
        )}

        {openCase.findings.length > 0 && (
          <section>
            <h3 className="text-xs font-semibold uppercase tracking-wide" style={{ color: "var(--text-secondary)" }}>
              What it established
            </h3>
            <ul className="mt-2 space-y-2">
              {openCase.findings.map((finding, index) => (
                <li
                  key={index}
                  className="rounded-lg border p-3"
                  style={{ borderColor: "var(--border)" }}
                >
                  <p className="text-sm font-medium">{finding.title}</p>
                  {finding.detail && (
                    <p className="mt-1 text-xs" style={{ color: "var(--text-secondary)" }}>
                      {finding.detail}
                    </p>
                  )}
                  <EvidenceCitations ids={finding.evidenceIds} evidenceById={evidenceById} />
                </li>
              ))}
            </ul>
          </section>
        )}

        <Transcript openCase={openCase} live={live} running={running} />
      </div>
    </Card>
  );
}

function statusBadge(
  openCase: TroubleshootingCase,
  running: boolean,
): { tone: StatusTone; label: string } {
  if (running) return { tone: "neutral", label: "Working" };
  switch (openCase.status) {
    case "waitingForAnswer":
      return { tone: "warning", label: "Needs your answer" };
    case "done":
      return openCase.remedy
        ? { tone: "good", label: "Answered" }
        : { tone: "neutral", label: "Finished" };
    case "cancelled":
      return { tone: "neutral", label: "Stopped" };
    case "failed":
      return { tone: "critical", label: "Failed" };
    default:
      return { tone: "neutral", label: "Working" };
  }
}

function QuestionBox({
  question,
  busy,
  answer,
  onAnswerChange,
  onReply,
}: {
  question: NonNullable<TroubleshootingCase["question"]>;
  busy: boolean;
  answer: string;
  onAnswerChange: (value: string) => void;
  onReply: (text: string) => void;
}) {
  return (
    <div
      className="rounded-lg border p-3"
      style={{ borderColor: "var(--status-warning)", background: "var(--surface-raised)" }}
    >
      <p className="text-sm font-medium">{question.text}</p>

      {question.choices.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-2">
          {question.choices.map((choice) => (
            <Button key={choice} onClick={() => onReply(choice)} disabled={busy}>
              {choice}
            </Button>
          ))}
        </div>
      ) : null}

      {/* Always offered, even alongside choices: the honest answer is often
          "neither of those". */}
      <div className="mt-2 flex gap-2">
        <input
          value={answer}
          onChange={(e) => onAnswerChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onReply(answer);
          }}
          disabled={busy}
          placeholder={question.choices.length > 0 ? "Or answer in your own words…" : "Your answer…"}
          className="min-w-0 flex-1 rounded-lg border px-2 py-1.5 text-sm"
          style={{
            borderColor: "var(--border-strong)",
            background: "var(--surface-1)",
            color: "var(--text-primary)",
          }}
        />
        <Button variant="primary" onClick={() => onReply(answer)} disabled={busy || !answer.trim()}>
          Send
        </Button>
      </div>
    </div>
  );
}

const CONFIDENCE_TONE: Record<Remedy["confidence"], StatusTone> = {
  high: "good",
  medium: "warning",
  low: "neutral",
};

function RemedyCard({
  remedy,
  evidenceById,
}: {
  remedy: Remedy;
  evidenceById: Map<string, Evidence>;
}) {
  return (
    <div
      className="rounded-lg border p-4"
      style={{ borderColor: "var(--border-strong)", background: "var(--surface-raised)" }}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <h3 className="text-sm font-semibold">{remedy.title}</h3>
        <div className="flex shrink-0 gap-2">
          <StatusBadge
            tone={CONFIDENCE_TONE[remedy.confidence]}
            label={`${remedy.confidence} confidence`}
          />
          {/* Stated up front so the user can judge the risk before acting,
              rather than discovering it afterwards. */}
          <StatusBadge
            tone={remedy.reversible ? "good" : "warning"}
            label={remedy.reversible ? "Easy to undo" : "Hard to undo"}
          />
        </div>
      </div>

      <p className="mt-2 text-sm" style={{ color: "var(--text-secondary)" }}>
        {remedy.rationale}
      </p>

      {remedy.steps.length > 0 && (
        <ol className="mt-3 space-y-1.5 text-sm">
          {remedy.steps.map((step, index) => (
            <li key={index} className="flex gap-2">
              <span
                className="shrink-0 font-mono text-xs"
                style={{ color: "var(--text-muted)" }}
              >
                {index + 1}.
              </span>
              <span>{step}</span>
            </li>
          ))}
        </ol>
      )}

      <EvidenceCitations ids={remedy.evidenceIds} evidenceById={evidenceById} />

      <p className="mt-3 text-[11px]" style={{ color: "var(--text-muted)" }}>
        The assistant changes nothing itself — these are steps for you to carry out.
      </p>
    </div>
  );
}

/**
 * What a conclusion rests on.
 *
 * The engine refuses a remedy that cites nothing, so these are always present;
 * showing them is what lets a user check a claim instead of trusting it.
 */
function EvidenceCitations({
  ids,
  evidenceById,
}: {
  ids: string[];
  evidenceById: Map<string, Evidence>;
}) {
  if (ids.length === 0) return null;

  return (
    <div className="mt-3">
      <p className="text-[11px] uppercase tracking-wide" style={{ color: "var(--text-muted)" }}>
        Measured
      </p>
      <ul className="mt-1 space-y-1">
        {ids.map((id) => {
          const evidence = evidenceById.get(id);
          return (
            <li key={id} className="flex flex-wrap items-baseline gap-2 text-xs">
              <Pill mono>{evidence?.tool ?? id}</Pill>
              <span style={{ color: "var(--text-secondary)" }}>
                {evidence?.summary ?? "reading not found"}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** One line of the working record: something said, a tool run, or one still running. */
type TranscriptStep =
  | { kind: "said"; text: string }
  | { kind: "tool"; name: string; isError: boolean }
  | { kind: "pending"; name: string };

/**
 * The working record: what it said, what it measured, in order.
 *
 * Collapsed by default because the answer matters more than the route to it —
 * but present, because a diagnosis nobody can audit is worth very little.
 */
function Transcript({
  openCase,
  live,
  running,
}: {
  openCase: TroubleshootingCase;
  live: AssistEvent[];
  running: boolean;
}) {
  const [expanded, setExpanded] = useState(false);

  /* The stored case is the record; live events fill the gap while a step is
   * still in flight and nothing has been written yet. */
  const steps = useMemo<TranscriptStep[]>(() => {
    const stored = openCase.transcript.flatMap<TranscriptStep>((message) => {
      if (message.type === "assistant") {
        const said = message.text?.trim();
        return said ? [{ kind: "said" as const, text: said }] : [];
      }
      if (message.type === "toolResult") {
        return [{ kind: "tool" as const, name: message.name, isError: message.isError }];
      }
      return [];
    });

    if (!running) return stored;
    const tail = live
      .slice(-3)
      .flatMap<TranscriptStep>((event) =>
        event.type === "said"
          ? [{ kind: "said", text: event.text }]
          : event.type === "toolStarted"
            ? [{ kind: "pending", name: event.name }]
            : [],
      );
    return [...stored, ...tail];
  }, [openCase.transcript, live, running]);

  if (steps.length === 0) return null;

  return (
    <section>
      <button
        onClick={() => setExpanded((current) => !current)}
        className="text-xs underline"
        style={{ color: "var(--text-secondary)" }}
      >
        {expanded ? "Hide" : "Show"} how it worked this out ({openCase.evidence.length} measurement
        {openCase.evidence.length === 1 ? "" : "s"}, {openCase.steps} step
        {openCase.steps === 1 ? "" : "s"})
      </button>

      {expanded && (
        <ol className="mt-2 space-y-1.5">
          {steps.map((step, index) => (
            <li key={index} className="flex items-baseline gap-2 text-xs">
              {step.kind === "said" ? (
                <span style={{ color: "var(--text-secondary)" }}>{step.text}</span>
              ) : step.kind === "pending" ? (
                <>
                  <Spinner size={11} />
                  <Pill mono>{step.name}</Pill>
                </>
              ) : (
                <>
                  {step.isError ? (
                    <StatusMark tone="warning" />
                  ) : (
                    <span aria-hidden style={{ color: "var(--text-muted)", display: "inline-flex" }}>
                      <Icon name="check" size={12} />
                    </span>
                  )}
                  <Pill mono>{step.name}</Pill>
                </>
              )}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function CaseList({
  cases,
  openId,
  onOpen,
  onDelete,
}: {
  cases: CaseSummary[] | null;
  openId?: string;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  if (cases === null) return <LoadingState title="Loading earlier diagnoses…" />;
  if (cases.length === 0) return null;

  return (
    <Card title="Earlier diagnoses" subtitle="Kept with this network, like its scans">
      <ul className="space-y-1">
        {cases.map((item) => (
          <li
            key={item.id}
            className="flex flex-wrap items-center justify-between gap-2 rounded-lg border px-3 py-2"
            style={{
              borderColor: item.id === openId ? "var(--border-strong)" : "var(--border)",
            }}
          >
            <button onClick={() => onOpen(item.id)} className="min-w-0 flex-1 text-left">
              <p className="truncate text-sm">{item.remedyTitle ?? item.symptom}</p>
              <p className="mt-0.5 text-[11px]" style={{ color: "var(--text-muted)" }}>
                {new Date(item.createdAt).toLocaleString()} · {item.findingCount} finding
                {item.findingCount === 1 ? "" : "s"}
              </p>
            </button>
            <Button variant="danger" onClick={() => onDelete(item.id)}>
              Delete
            </Button>
          </li>
        ))}
      </ul>
    </Card>
  );
}
