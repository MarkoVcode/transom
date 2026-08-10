//! The loop: symptom in, evidence-cited remedy out.
//!
//! The model decides *what to measure next*. It never measures, and it is never
//! the source of a number. Every reading in a case came from this crate's own
//! probes, was recorded as evidence with an id, and has to be cited before a
//! conclusion built on it is accepted.
//!
//! The loop is bounded on four axes — steps, output tokens, wall clock, and an
//! explicit cancel — because an agent that can call tools is an agent that can
//! call them forever. Every bound stops the case politely, with a note saying
//! which one was hit, rather than by vanishing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::case::{Case, CaseStatus, Question};
use super::provider::{Message, ProviderError, Request, StopReason};
use super::tools::{self, Control, ToolContext, ToolProgress};
use super::DynChatProvider;

/// How far the assistant may go before it must stop and say so.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Model round trips. Each one can run several tools.
    pub max_steps: u32,
    /// Output tokens across the whole case — the cost ceiling.
    pub max_output_tokens: u64,
    /// From the moment this run starts. `observe` sleeps inside this, so it
    /// has to be generous enough for a reproduction plus the reasoning around
    /// it.
    pub wall_clock: Duration,
    /// Per model call.
    pub max_tokens_per_turn: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 24,
            max_output_tokens: 120_000,
            wall_clock: Duration::from_secs(15 * 60),
            max_tokens_per_turn: 4096,
        }
    }
}

/// What the UI is told as the case progresses.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistEvent {
    /// The model's reasoning between tool calls.
    #[serde(rename_all = "camelCase")]
    Said { text: String },
    #[serde(rename_all = "camelCase")]
    ToolStarted { name: String },
    #[serde(rename_all = "camelCase")]
    ToolFinished {
        name: String,
        summary: String,
        is_error: bool,
    },
    /// The user has to do something for the next measurement to mean anything.
    /// The UI has to say so loudly — a silent 30-second wait measures nothing.
    #[serde(rename_all = "camelCase")]
    ReproduceNow { seconds: u64, reason: String },
    #[serde(rename_all = "camelCase")]
    Asked { question: Question },
    #[serde(rename_all = "camelCase")]
    Finding { title: String },
    #[serde(rename_all = "camelCase")]
    Remedy { title: String },
    #[serde(rename_all = "camelCase")]
    Finished {
        status: CaseStatus,
        note: Option<String>,
    },
}

/// Set by the UI to stop a run.
pub type Cancel = Arc<AtomicBool>;

/// Records the user's answer and puts the case back to work.
///
/// Returns false when the case was not waiting — answering a finished case is a
/// UI race, not an error worth surfacing.
pub fn answer(case: &mut Case, text: impl Into<String>) -> bool {
    if case.status != CaseStatus::WaitingForAnswer {
        return false;
    }
    case.transcript.push(Message::User { text: text.into() });
    case.question = None;
    case.status = CaseStatus::Running;
    case.touch();
    true
}

/// Runs the case until it settles, or until a bound is reached.
///
/// The case is saved after every step, so a crash mid-diagnosis loses one turn
/// rather than the session.
pub async fn run(
    case: &mut Case,
    provider: &dyn DynChatProvider,
    ctx: &ToolContext,
    limits: Limits,
    cancel: &Cancel,
    emit: &mut (dyn FnMut(AssistEvent) + Send),
) {
    let started = Instant::now();
    let specs = tools::specs();

    // Vocabulary first, so nothing identifying is in the very first request.
    if let Ok(mut redactor) = ctx.redactor.lock() {
        redactor.learn_name(&ctx.network_name);
    }
    if let Some(snapshot) = ctx.store.load_latest().await {
        if let Ok(mut redactor) = ctx.redactor.lock() {
            redactor.learn_snapshot(&snapshot);
        }
    }

    // Built after learning: the network's own name is usually the operator's.
    let system = match ctx.redactor.lock() {
        Ok(mut redactor) => redactor.redact(&system_prompt(ctx)),
        Err(_) => system_prompt(ctx),
    };

    // Bounded so a model that will not use the tool surface cannot spin.
    let mut nudges = 0u32;

    while !case.is_settled() {
        if cancel.load(Ordering::Relaxed) {
            settle(case, CaseStatus::Cancelled, Some("Stopped.".into()), emit);
            return;
        }
        if let Some(reason) = exhausted(case, limits, started) {
            settle(case, CaseStatus::Done, Some(reason), emit);
            return;
        }

        case.steps += 1;

        let turn = match complete(provider, &system, case, ctx, &specs, limits, cancel).await {
            Ok(turn) => turn,
            Err(error) => {
                settle(case, CaseStatus::Failed, Some(error), emit);
                return;
            }
        };

        case.usage.input_tokens += turn.usage.input_tokens;
        case.usage.output_tokens += turn.usage.output_tokens;

        if !turn.text.trim().is_empty() {
            emit(AssistEvent::Said {
                text: turn.text.clone(),
            });
        }

        case.transcript.push(Message::Assistant {
            text: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
        });

        if turn.stop_reason == StopReason::Refusal {
            settle(
                case,
                CaseStatus::Failed,
                Some("The model declined to continue with this case.".into()),
                emit,
            );
            return;
        }

        if turn.tool_calls.is_empty() {
            // It answered in prose instead of finishing through the tool that
            // enforces citation. Say so once; if it does it again, keep what it
            // wrote rather than looping, and be clear the case ended informally.
            nudges += 1;
            if nudges > 2 {
                settle(
                    case,
                    CaseStatus::Done,
                    Some(if turn.text.trim().is_empty() {
                        "The assistant stopped without reaching a conclusion.".into()
                    } else {
                        turn.text.clone()
                    }),
                    emit,
                );
                return;
            }

            case.transcript.push(Message::User {
                text: "Continue by calling a tool. When you have enough to conclude, finish with \
                       propose_remedy citing the evidence ids — including when the conclusion is \
                       that the evidence does not identify a cause."
                    .into(),
            });
            let _ = case.save(&ctx.network_root).await;
            continue;
        }

        for call in &turn.tool_calls {
            emit(AssistEvent::ToolStarted {
                name: call.name.clone(),
            });

            let outcome = {
                let mut relay = |progress: ToolProgress| match progress {
                    ToolProgress::ReproduceNow { seconds, reason } => {
                        emit(AssistEvent::ReproduceNow { seconds, reason })
                    }
                    ToolProgress::Note(text) => emit(AssistEvent::Said { text }),
                };
                tools::run(&call.name, &call.input, case, ctx, &mut relay).await
            };

            emit(AssistEvent::ToolFinished {
                name: call.name.clone(),
                summary: case
                    .evidence
                    .last()
                    .filter(|_| !outcome.is_error)
                    .map(|evidence| evidence.summary.clone())
                    .unwrap_or_else(|| tool_note(&call.name, &outcome.content, outcome.is_error)),
                is_error: outcome.is_error,
            });

            if call.name == "record_finding" && !outcome.is_error {
                if let Some(finding) = case.findings.last() {
                    emit(AssistEvent::Finding {
                        title: finding.title.clone(),
                    });
                }
            }

            case.transcript.push(Message::ToolResult {
                call_id: call.id.clone(),
                name: call.name.clone(),
                content: outcome.content,
                is_error: outcome.is_error,
            });

            match outcome.control {
                Control::Continue => {}
                Control::Ask(question) => {
                    case.status = CaseStatus::WaitingForAnswer;
                    case.question = Some(question.clone());
                    emit(AssistEvent::Asked { question });
                }
                Control::Finish => {
                    if let Some(remedy) = &case.remedy {
                        emit(AssistEvent::Remedy {
                            title: remedy.title.clone(),
                        });
                    }
                }
            }
        }

        case.touch();
        let _ = case.save(&ctx.network_root).await;

        if case.is_settled() {
            emit(AssistEvent::Finished {
                status: case.status,
                note: case.note.clone(),
            });
            return;
        }
    }
}

/// Which bound, if any, has been reached.
fn exhausted(case: &Case, limits: Limits, started: Instant) -> Option<String> {
    if case.steps >= limits.max_steps {
        return Some(format!(
            "Stopped after {} steps without reaching a conclusion. What has been measured so far \
             is recorded below.",
            limits.max_steps
        ));
    }
    if case.usage.output_tokens >= limits.max_output_tokens {
        return Some("Stopped at this case's token limit.".into());
    }
    if started.elapsed() >= limits.wall_clock {
        return Some(format!(
            "Stopped after {} minutes.",
            limits.wall_clock.as_secs() / 60
        ));
    }
    None
}

/// One model call, retrying only what is worth retrying.
///
/// A busy provider is a transient condition; a bad key is not. Retrying the
/// second wastes the user's time to arrive at the same answer.
async fn complete(
    provider: &dyn DynChatProvider,
    system: &str,
    case: &Case,
    ctx: &ToolContext,
    specs: &[super::provider::ToolSpec],
    limits: Limits,
    cancel: &Cancel,
) -> Result<super::provider::Turn, String> {
    // Pseudonymised here and nowhere else. This is the only point in the
    // program where case data crosses the machine boundary, so it is the one
    // place the substitution has to happen — the stored transcript keeps the
    // real values, and the user reads those.
    let outbound: Vec<Message> = match ctx.redactor.lock() {
        Ok(mut redactor) => case
            .transcript
            .iter()
            .map(|message| redact_message(&mut redactor, message))
            .collect(),
        Err(_) => case.transcript.clone(),
    };

    let mut attempt = 0u32;
    loop {
        let result = provider
            .complete(Request {
                system,
                messages: &outbound,
                tools: specs,
                max_tokens: limits.max_tokens_per_turn,
            })
            .await;

        match result {
            // Restored on the way back, so everything stored and shown from
            // here on is in the user's own terms again.
            Ok(turn) => {
                return Ok(match ctx.redactor.lock() {
                    Ok(redactor) => restore_turn(&redactor, turn),
                    Err(_) => turn,
                })
            }
            Err(ProviderError::Busy(detail)) if attempt < 3 => {
                attempt += 1;
                let backoff = Duration::from_secs(2u64.pow(attempt));
                if cancel.load(Ordering::Relaxed) {
                    return Err(format!("busy, then cancelled: {detail}"));
                }
                tokio::time::sleep(backoff).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Pseudonymises one stored message for transmission.
fn redact_message(redactor: &mut super::Redactor, message: &Message) -> Message {
    match message {
        Message::User { text } => Message::User {
            text: redactor.redact(text),
        },
        Message::Assistant { text, tool_calls } => Message::Assistant {
            text: redactor.redact(text),
            tool_calls: tool_calls
                .iter()
                .map(|call| super::ToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    input: redact_json(redactor, &call.input),
                })
                .collect(),
        },
        Message::ToolResult {
            call_id,
            name,
            content,
            is_error,
        } => Message::ToolResult {
            call_id: call_id.clone(),
            name: name.clone(),
            content: redactor.redact(content),
            is_error: *is_error,
        },
    }
}

/// Substitutes inside a JSON value by way of its text form.
///
/// Structure is untouched — only the leaves change — and a value that will not
/// re-parse is left exactly as it was rather than silently dropped.
fn redact_json(redactor: &mut super::Redactor, value: &serde_json::Value) -> serde_json::Value {
    let Ok(text) = serde_json::to_string(value) else {
        return value.clone();
    };
    serde_json::from_str(&redactor.redact(&text)).unwrap_or_else(|_| value.clone())
}

/// Puts real names back into what the model said.
///
/// Tool inputs are restored too, and that is not cosmetic: a model asking to
/// look up `host-4` must be handed the name the tool can actually find.
fn restore_turn(redactor: &super::Redactor, turn: super::provider::Turn) -> super::provider::Turn {
    super::provider::Turn {
        text: redactor.restore(&turn.text),
        tool_calls: turn
            .tool_calls
            .into_iter()
            .map(|call| super::ToolCall {
                input: serde_json::to_string(&call.input)
                    .ok()
                    .and_then(|text| serde_json::from_str(&redactor.restore(&text)).ok())
                    .unwrap_or(call.input),
                ..call
            })
            .collect(),
        ..turn
    }
}

fn settle(
    case: &mut Case,
    status: CaseStatus,
    note: Option<String>,
    emit: &mut (dyn FnMut(AssistEvent) + Send),
) {
    case.status = status;
    if note.is_some() {
        case.note = note.clone();
    }
    case.touch();
    emit(AssistEvent::Finished { status, note });
}

/// A short line for the UI when a tool produced no evidence — an error, or one
/// of the terminal tools.
fn tool_note(name: &str, content: &str, is_error: bool) -> String {
    if is_error {
        content.chars().take(160).collect()
    } else {
        match name {
            "ask_user" => "Waiting for your answer".into(),
            "record_finding" => "Finding recorded".into(),
            "propose_remedy" => "Remedy proposed".into(),
            other => other.replace('_', " "),
        }
    }
}

/// What the model is told about its job and its limits.
///
/// The constraints here are restated by construction elsewhere — it cannot call
/// a tool that does not exist, and `propose_remedy` rejects an uncited claim
/// outright. The prompt explains *why*, which is what makes a model cooperate
/// with a rule rather than work around it.
fn system_prompt(ctx: &ToolContext) -> String {
    let scope = if ctx.network_subnets.is_empty() {
        "not yet recorded".to_string()
    } else {
        ctx.network_subnets.join(", ")
    };

    let controller = if ctx.unifi.is_some() {
        "A UniFi controller is connected, so switch port counters, radio airtime and device load \
         are available — including the `observe` tool."
    } else {
        "No controller is connected. Infrastructure counters and `observe` are unavailable; work \
         from the scan, ping and traceroute, and say plainly when a question needs controller \
         data you do not have."
    };

    format!(
        "You are the troubleshooting assistant inside a local network diagnostics desktop app. A \
         user has described a symptom in their own words. Your job is to find out what is \
         actually causing it and tell them what to do.

You are diagnosing the network named \"{name}\", covering {scope}. {controller}

HOW YOU WORK

You do not measure anything yourself and you never state a number you were not given. The app's \
own probes produce every reading; you decide which one to take next. Each tool result carries an \
`evidenceId`. Cite those ids in record_finding and propose_remedy — a claim with no id behind it \
will be rejected, because a plausible sentence built on an invented number is worse than no \
answer.

Work like an engineer with a meter, not like a search engine:

1. Read what already exists first. scan_summary, then infrastructure_findings if a controller is \
   connected — the fault is often already visible there and costs nothing to look at.
2. Form a specific hypothesis, then take the one measurement that would kill it. Do not collect \
   evidence generally; collect the evidence that discriminates.
3. Ask the user only what you cannot measure — which two machines, wired or wireless, whether a \
   small file behaves differently. One question at a time.
4. When a symptom appears only under load, use `observe`. Idle counters cannot show a port that \
   errors while traffic flows or a radio that saturates during a transfer. This is usually the \
   measurement that settles the case.
5. Finish with propose_remedy. If the evidence does not identify a cause, say exactly that, and \
   say which measurement would — that is a useful answer, and guessing is not.

WHAT YOU CANNOT DO

You read only. You cannot change any setting, on this machine or on the controller, and there is \
no tool that would. Remedies are steps for the user to carry out. Say whether they are easy to \
undo, and prefer the reversible check before the disruptive one — ask them to swap a cable before \
you ask them to re-plan their VLANs.

The app runs on this one machine, so it cannot measure throughput directly between two other \
hosts. Do not claim to have done so. What you can do is watch the infrastructure while the user \
reproduces the transfer, and see where the bytes went and what broke while they moved.

HOW TO WRITE

Plainly, to someone who runs their own network but is not a network engineer. Name the specific \
port, radio or device. Give the reading and what it means, not the reading alone. Do not pad the \
answer with everything you ruled out — say what is wrong and what to do about it.",
        name = ctx.network_name,
        scope = scope,
        controller = controller,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::provider::{ToolCall, Turn, Usage};
    use crate::store::Store;
    use std::sync::Mutex;

    /// Replays a scripted set of turns, so the loop is testable without a
    /// model, a controller or a network — which is what keeps this feature
    /// regression-testable at all.
    struct FixtureProvider {
        turns: Mutex<std::collections::VecDeque<Turn>>,
        /// Everything the provider was handed, so a test can assert on what
        /// would actually have left the machine.
        seen: Mutex<Vec<String>>,
    }

    impl FixtureProvider {
        fn new(turns: Vec<Turn>) -> Self {
            Self {
                turns: Mutex::new(turns.into()),
                seen: Mutex::new(Vec::new()),
            }
        }

        /// The full outbound payload of every call made so far.
        fn outbound(&self) -> String {
            self.seen.lock().unwrap().join("\n")
        }
    }

    impl DynChatProvider for FixtureProvider {
        fn describe(&self) -> String {
            "fixture".into()
        }

        fn complete<'a>(
            &'a self,
            request: Request<'a>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Turn, ProviderError>> + Send + 'a>,
        > {
            self.seen.lock().unwrap().push(format!(
                "{}\n{}",
                request.system,
                serde_json::to_string(request.messages).unwrap_or_default()
            ));
            let next = self.turns.lock().unwrap().pop_front();
            Box::pin(async move {
                next.ok_or_else(|| ProviderError::Protocol("fixture ran out of turns".into()))
            })
        }
    }

    fn call(name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.into(),
            input,
        }
    }

    fn turn(text: &str, calls: Vec<ToolCall>) -> Turn {
        Turn {
            text: text.into(),
            stop_reason: if calls.is_empty() {
                StopReason::EndTurn
            } else {
                StopReason::ToolUse
            },
            tool_calls: calls,
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
            },
        }
    }

    fn context(root: &std::path::Path) -> ToolContext {
        ToolContext {
            store: Store::new(root.join("scans")),
            network_name: "Home".into(),
            network_subnets: vec!["10.0.3.0/24".into()],
            unifi: None,
            network_root: root.to_path_buf(),
            redactor: std::sync::Mutex::new(crate::assist::Redactor::new(true)),
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("netdiag-session-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn drive(turns: Vec<Turn>, root: &std::path::Path) -> (Case, Vec<AssistEvent>) {
        let provider = FixtureProvider::new(turns);
        let ctx = context(root);
        let mut case = Case::new("net-1", "uploads freeze the network");
        let cancel: Cancel = Arc::new(AtomicBool::new(false));
        let mut events = Vec::new();

        run(
            &mut case,
            &provider,
            &ctx,
            Limits {
                wall_clock: Duration::from_secs(30),
                ..Limits::default()
            },
            &cancel,
            &mut |event| events.push(event),
        )
        .await;

        (case, events)
    }

    #[tokio::test]
    async fn a_question_pauses_the_case_until_it_is_answered() {
        // The pause is the whole point: the model asked something only the user
        // knows, and the loop must not carry on inventing the answer.
        let root = scratch("question");
        let (mut case, events) = drive(
            vec![turn(
                "I need to know how they are connected.",
                vec![call(
                    "ask_user",
                    serde_json::json!({"question": "Are both machines wired?",
                                       "choices": ["Both wired", "One is Wi-Fi"]}),
                )],
            )],
            &root,
        )
        .await;

        assert_eq!(case.status, CaseStatus::WaitingForAnswer);
        assert_eq!(case.question.as_ref().unwrap().choices.len(), 2);
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistEvent::Asked { .. })));

        // And it is saved, so the answer may arrive after a restart.
        let reloaded = Case::load(&root, &case.id).await.unwrap();
        assert_eq!(reloaded.status, CaseStatus::WaitingForAnswer);

        assert!(answer(&mut case, "Both wired"));
        assert_eq!(case.status, CaseStatus::Running);
        assert!(case.question.is_none());
        assert!(
            !answer(&mut case, "again"),
            "answering a case that is not waiting must be a no-op"
        );
    }

    #[tokio::test]
    async fn a_remedy_without_evidence_is_rejected_and_the_case_keeps_going() {
        // The structural guard, exercised through the loop rather than the
        // tool: the rejection has to come back as something the model can read
        // and correct, not as a failed case.
        let root = scratch("uncited");
        let (case, _events) = drive(
            vec![
                turn(
                    "It is obviously the cable.",
                    vec![call(
                        "propose_remedy",
                        serde_json::json!({"title": "Replace the cable", "rationale": "hunch",
                                           "steps": ["swap it"], "evidence_ids": [],
                                           "confidence": "high"}),
                    )],
                ),
                turn(
                    "Understood, let me look first.",
                    vec![call("scan_summary", serde_json::json!({}))],
                ),
                turn("No scan to work from.", vec![]),
                turn("Still nothing.", vec![]),
                turn("Giving up.", vec![]),
            ],
            &root,
        )
        .await;

        assert!(case.remedy.is_none(), "the uncited remedy must not stick");
        let rejected = case.transcript.iter().any(|message| {
            matches!(message, Message::ToolResult { is_error, content, .. }
                     if *is_error && content.contains("Cite the evidence"))
        });
        assert!(rejected, "the model must be told why, so it can correct it");
    }

    #[tokio::test]
    async fn the_step_limit_stops_the_case_and_says_so() {
        // An agent with tools is an agent that can loop forever. It has to stop
        // in a way that leaves the user with the work done so far.
        let root = scratch("steps");
        let provider = FixtureProvider::new(
            (0..50)
                .map(|_| {
                    turn(
                        "thinking",
                        vec![call("scan_summary", serde_json::json!({}))],
                    )
                })
                .collect(),
        );
        let ctx = context(&root);
        let mut case = Case::new("net-1", "slow");
        let cancel: Cancel = Arc::new(AtomicBool::new(false));
        let mut events = Vec::new();

        run(
            &mut case,
            &provider,
            &ctx,
            Limits {
                max_steps: 3,
                ..Limits::default()
            },
            &cancel,
            &mut |event| events.push(event),
        )
        .await;

        assert_eq!(case.steps, 3);
        assert!(case.is_settled());
        assert!(case.note.as_ref().unwrap().contains("3 steps"));
    }

    #[tokio::test]
    async fn cancelling_stops_before_the_next_model_call() {
        let root = scratch("cancel");
        let provider = FixtureProvider::new(vec![turn("hello", vec![])]);
        let ctx = context(&root);
        let mut case = Case::new("net-1", "slow");
        let cancel: Cancel = Arc::new(AtomicBool::new(true));
        let mut events = Vec::new();

        run(
            &mut case,
            &provider,
            &ctx,
            Limits::default(),
            &cancel,
            &mut |event| events.push(event),
        )
        .await;

        assert_eq!(case.status, CaseStatus::Cancelled);
        assert_eq!(case.steps, 0, "no model call should have been made");
    }

    #[tokio::test]
    async fn a_provider_failure_fails_the_case_rather_than_hanging() {
        let root = scratch("failure");
        let (case, events) = drive(vec![], &root).await; // fixture immediately runs dry

        assert_eq!(case.status, CaseStatus::Failed);
        assert!(case.note.is_some());
        assert!(matches!(
            events.last(),
            Some(AssistEvent::Finished {
                status: CaseStatus::Failed,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn prose_instead_of_a_tool_call_is_nudged_before_it_is_accepted() {
        let root = scratch("nudge");
        let (case, _) = drive(
            vec![
                turn("Probably your Wi-Fi.", vec![]),
                turn("Probably your Wi-Fi.", vec![]),
                turn("Probably your Wi-Fi.", vec![]),
            ],
            &root,
        )
        .await;

        assert!(case.is_settled());
        assert_eq!(case.note.as_deref(), Some("Probably your Wi-Fi."));
        let nudged = case.transcript.iter().filter(
            |message| matches!(message, Message::User { text } if text.contains("propose_remedy")),
        );
        assert_eq!(nudged.count(), 2, "nudged twice, then accepted");
    }

    /// Drives one case against a provider the caller keeps, so it can inspect
    /// exactly what was sent.
    async fn drive_with(provider: &FixtureProvider, ctx: &ToolContext, symptom: &str) -> Case {
        let mut case = Case::new("net-1", symptom);
        let cancel: Cancel = Arc::new(AtomicBool::new(false));
        run(
            &mut case,
            provider,
            ctx,
            Limits {
                max_steps: 4,
                ..Limits::default()
            },
            &cancel,
            &mut |_| {},
        )
        .await;
        case
    }

    #[tokio::test]
    async fn nothing_identifying_reaches_the_provider() {
        // The end-to-end privacy check. Redaction is worth nothing if it is
        // applied anywhere other than the boundary it claims to guard, so this
        // asserts on the bytes the provider was actually handed.
        let root = scratch("privacy");
        let mut ctx = context(&root);
        ctx.network_name = "Ashgrove House".into();
        if let Ok(mut redactor) = ctx.redactor.lock() {
            redactor.learn_name("studio-macbook");
        }

        let provider = FixtureProvider::new(vec![turn(
            "Looking at host-1 now.",
            vec![call("find_devices", serde_json::json!({"query": "host-1"}))],
        )]);

        let case = drive_with(
            &provider,
            &ctx,
            "studio-macbook uploads to 10.0.3.14 freeze; wan is 203.0.113.9",
        )
        .await;

        let sent = provider.outbound();
        for secret in ["studio-macbook", "Ashgrove House", "203.0.113.9"] {
            assert!(!sent.contains(secret), "{secret} left the machine:\n{sent}");
        }
        // The private address is what the diagnosis is about, and it stays.
        assert!(sent.contains("10.0.3.14"));

        // And the stored case still reads in the user's own terms — the
        // pseudonyms exist only in flight.
        assert!(case.symptom.contains("studio-macbook"));
        let restored = case.transcript.iter().any(|message| {
            matches!(message, Message::Assistant { text, .. } if text.contains("studio-macbook"))
        });
        assert!(
            restored,
            "the model's reply should be restored before storage"
        );
    }

    #[tokio::test]
    async fn a_tool_call_naming_a_placeholder_is_restored_before_it_runs() {
        // Not cosmetic: a lookup for `host-1` finds nothing, because that name
        // exists nowhere outside the substitution table.
        let root = scratch("restore-input");
        let ctx = context(&root);
        if let Ok(mut redactor) = ctx.redactor.lock() {
            redactor.learn_name("studio-macbook");
        }

        let provider = FixtureProvider::new(vec![turn(
            "",
            vec![call("find_devices", serde_json::json!({"query": "host-1"}))],
        )]);
        let case = drive_with(&provider, &ctx, "slow uploads").await;

        let ran_with_the_real_name = case.transcript.iter().any(|message| {
            matches!(message, Message::Assistant { tool_calls, .. }
                     if tool_calls.iter().any(|c| c.input["query"] == "studio-macbook"))
        });
        assert!(
            ran_with_the_real_name,
            "the placeholder must not reach the tool"
        );
    }

    #[tokio::test]
    async fn redaction_off_sends_the_evidence_as_it_is() {
        // The setting has to actually be a setting.
        let root = scratch("no-redact");
        let mut ctx = context(&root);
        ctx.redactor = std::sync::Mutex::new(crate::assist::Redactor::new(false));

        let provider = FixtureProvider::new(vec![turn("ok", vec![])]);
        drive_with(&provider, &ctx, "studio-macbook is slow").await;

        assert!(provider.outbound().contains("studio-macbook"));
    }

    #[test]
    fn the_prompt_tells_the_model_what_it_does_not_have() {
        // A model that does not know the controller is absent will confidently
        // reason about counters it can never read.
        let root = scratch("prompt");
        let without = system_prompt(&context(&root));
        assert!(without.contains("No controller is connected"));
        assert!(without.contains("10.0.3.0/24"));
        assert!(without.contains("evidenceId"));
        assert!(without.contains("read only"));

        let mut ctx = context(&root);
        ctx.unifi = Some((crate::unifi::UnifiConfig::default(), String::new()));
        assert!(system_prompt(&ctx).contains("observe"));
    }
}
