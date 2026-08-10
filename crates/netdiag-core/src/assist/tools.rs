//! What the assistant is allowed to do.
//!
//! Every tool here is **read-only**. The assistant diagnoses and proposes; it
//! changes nothing. That is a property of this list, not of the prompt — a
//! model cannot call a tool that does not exist, and no amount of persuasion
//! adds one.
//!
//! Each tool returns structured JSON *and* records an evidence id. The model is
//! required to cite those ids when it concludes anything, which is what stops a
//! confident sentence being built on a number nobody measured.

use serde_json::json;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use super::case::{Case, CaseStatus, Confidence, Finding, Question, Remedy};
use super::provider::ToolSpec;
use super::redact::Redactor;
use crate::netutil;
use crate::store::Store;
use crate::types::ScanSnapshot;
use crate::unifi::UnifiConfig;

/// What the tools need in order to answer.
///
/// Assembled by the desktop layer, which is the only place that can read the
/// controller password — the engine is handed it, exactly as the scanner is.
pub struct ToolContext {
    pub store: Store,
    pub network_name: String,
    pub network_subnets: Vec<String>,
    /// Present only when a controller is configured for this network.
    pub unifi: Option<(UnifiConfig, String)>,
    /// Where the assistant may write nothing — reserved so a future apply step
    /// has an obvious home rather than being bolted onto a read-only path.
    pub network_root: PathBuf,
    /// Hides identifiers on the way to the model.
    ///
    /// Shared rather than owned by the loop because tools that fetch *live*
    /// controller data see names the stored scan never had — a client that
    /// joined an hour ago is exactly the one that would otherwise escape.
    pub redactor: std::sync::Mutex<Redactor>,
}

impl ToolContext {
    /// Teaches the redactor the names in a controller reading.
    fn learn_unifi(&self, snapshot: &crate::unifi::model::UnifiSnapshot) {
        if let Ok(mut redactor) = self.redactor.lock() {
            redactor.learn_unifi(snapshot);
        }
    }
}

/// Something that happened while a tool ran, for the UI.
#[derive(Debug, Clone)]
pub enum ToolProgress {
    /// The user must act before the measurement means anything.
    ReproduceNow {
        seconds: u64,
        reason: String,
    },
    Note(String),
}

/// The result of running one tool.
pub struct ToolOutcome {
    /// Fed back to the model.
    pub content: String,
    /// True when the tool failed in a way the model should try to work around.
    pub is_error: bool,
    /// Set when the tool ends the loop.
    pub control: Control,
}

/// How a tool affects the loop.
pub enum Control {
    Continue,
    /// Stop and wait for the user.
    Ask(Question),
    /// Stop: the assistant has answered.
    Finish,
}

impl ToolOutcome {
    fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            control: Control::Continue,
        }
    }

    fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            control: Control::Continue,
        }
    }
}

/// The tools offered to the model, with their schemas.
pub fn specs() -> Vec<ToolSpec> {
    let no_args = json!({"type": "object", "properties": {}, "additionalProperties": false});

    vec![
        ToolSpec {
            name: "scan_summary".into(),
            description: "What the most recent scan of this network covered and found: when it \
                          ran, which ranges it swept, how many devices answered, and any warnings \
                          it recorded. Start here — it is free and it frames everything else."
                .into(),
            input_schema: no_args.clone(),
        },
        ToolSpec {
            name: "find_devices".into(),
            description: "Search the last scan's devices by IP, MAC, name, vendor or open port. \
                          Use it to locate the machines a symptom is about before asking for \
                          detail on them."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Substring to match. Empty lists all."}
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "device_detail".into(),
            description: "Everything the last scan and the controller know about one device: \
                          open ports, switch port and access point, negotiated rates, signal, \
                          VLAN. This is where a wired-vs-wireless question is settled."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"ip": {"type": "string", "description": "IPv4 address"}},
                "required": ["ip"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "infrastructure_findings".into(),
            description: "What the controller and the scan disagree about, plus the faults the \
                          app detects on its own: half-duplex links, port error counters, \
                          saturated radios, loaded gateways, mesh uplinks, unstable clients. \
                          Usually the fastest route to a cause."
                .into(),
            input_schema: no_args.clone(),
        },
        ToolSpec {
            name: "connectivity".into(),
            description: "Latency, jitter, loss and DNS timings from the last scan, plus the \
                          path out. Answers whether the problem is local or upstream."
                .into(),
            input_schema: no_args.clone(),
        },
        ToolSpec {
            name: "wifi_survey".into(),
            description: "This machine's own Wi-Fi: which network and channel it is on, signal \
                          strength, and how crowded each channel is."
                .into(),
            input_schema: no_args.clone(),
        },
        ToolSpec {
            name: "ping".into(),
            description: "Measure latency, jitter and loss to one address right now. Use it to \
                          check a specific path rather than to confirm something is switched on."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target": {"type": "string", "description": "Private IPv4 address"},
                    "count": {"type": "integer", "minimum": 3, "maximum": 50}
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "traceroute".into(),
            description: "The hops between this machine and an address, with per-hop latency and \
                          loss. Shows whether traffic between two local subnets is being routed \
                          through a gateway."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"target": {"type": "string"}},
                "required": ["target"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "port_scan".into(),
            description: "Scan one host's ports now, with service banners. Slower than the other \
                          tools — use it when identifying what a device actually is matters."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {"ip": {"type": "string"}},
                "required": ["ip"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "observe".into(),
            description: "Watch the infrastructure while the user reproduces the problem. Takes \
                          a reading of switch port counters, radio airtime and device load, asks \
                          the user to reproduce, waits, then reports what CHANGED. This is the \
                          only way to see a fault that exists only under load — a port that \
                          errors while traffic flows, a radio that saturates during a transfer. \
                          Prefer it over guessing from idle readings. Requires a controller."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "seconds": {"type": "integer", "minimum": 10, "maximum": 180,
                                "description": "How long the user needs to reproduce it."},
                    "reason": {"type": "string",
                               "description": "What to tell the user to do, in one sentence."}
                },
                "required": ["seconds", "reason"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "ask_user".into(),
            description: "Ask something you cannot measure — which two machines, wired or \
                          wireless, whether it happens with a small file. Stops until they \
                          answer, so ask only what changes what you would check next, and ask \
                          one thing at a time."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "choices": {"type": "array", "items": {"type": "string"},
                                "description": "Offered answers. Omit for free text."}
                },
                "required": ["question"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "record_finding".into(),
            description: "Record something you have established, citing the evidence ids it \
                          rests on. Use it as you go, not only at the end."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "detail": {"type": "string"},
                    "evidence_ids": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["title", "detail", "evidence_ids"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "propose_remedy".into(),
            description: "Finish the case with what the user should do. Must cite the evidence \
                          ids it rests on. If the evidence does not support a specific remedy, \
                          say so here plainly rather than guessing — 'the readings do not show a \
                          cause, here is what would' is a useful answer."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "rationale": {"type": "string"},
                    "steps": {"type": "array", "items": {"type": "string"}},
                    "evidence_ids": {"type": "array", "items": {"type": "string"}},
                    "confidence": {"type": "string", "enum": ["low", "medium", "high"]},
                    "reversible": {"type": "boolean",
                                   "description": "Whether the steps are easy to undo."}
                },
                "required": ["title", "rationale", "steps", "evidence_ids", "confidence"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Runs one tool call.
pub async fn run(
    name: &str,
    input: &serde_json::Value,
    case: &mut Case,
    ctx: &ToolContext,
    progress: &mut (dyn FnMut(ToolProgress) + Send),
) -> ToolOutcome {
    match name {
        "scan_summary" => scan_summary(case, ctx).await,
        "find_devices" => find_devices(input, case, ctx).await,
        "device_detail" => device_detail(input, case, ctx).await,
        "infrastructure_findings" => infrastructure_findings(case, ctx).await,
        "connectivity" => connectivity(case, ctx).await,
        "wifi_survey" => wifi_survey(case, ctx).await,
        "ping" => ping(input, case).await,
        "traceroute" => traceroute(input, case).await,
        "port_scan" => port_scan(input, case).await,
        "observe" => observe(input, case, ctx, progress).await,
        "ask_user" => ask_user(input),
        "record_finding" => record_finding(input, case),
        "propose_remedy" => propose_remedy(input, case),
        other => ToolOutcome::error(format!("There is no tool called {other}.")),
    }
}

async fn snapshot(ctx: &ToolContext) -> Option<ScanSnapshot> {
    ctx.store.load_latest().await
}

fn no_scan() -> ToolOutcome {
    ToolOutcome::error(
        "This network has no scan yet, so there is nothing recorded to read. Ask the user to run \
         a scan first — every stored-evidence tool depends on it.",
    )
}

async fn scan_summary(case: &mut Case, ctx: &ToolContext) -> ToolOutcome {
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };

    let targets: Vec<&str> = snapshot
        .host
        .scan_targets
        .iter()
        .map(|target| target.cidr.as_str())
        .collect();

    let detail = json!({
        "network": ctx.network_name,
        "scannedAt": snapshot.started_at,
        "ranges": snapshot.host.scan_targets,
        "deviceCount": snapshot.devices.len(),
        "gateway": snapshot.host.gateway,
        "warnings": snapshot.warnings,
        "hasController": snapshot.unifi.is_some(),
        // Named so the model does not silently assume a device it cannot see
        // is absent, when it may simply have been outside the sweep.
        "addressesSeenOutsideThoseRanges": snapshot.off_scope.len(),
    });

    let id = case.add_evidence(
        "scan_summary",
        format!(
            "{} device(s) across {} at {}",
            snapshot.devices.len(),
            targets.join(", "),
            snapshot.started_at
        ),
        detail.clone(),
    );
    reply(id, detail)
}

async fn find_devices(
    input: &serde_json::Value,
    case: &mut Case,
    ctx: &ToolContext,
) -> ToolOutcome {
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };
    let query = input
        .get("query")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let matches: Vec<serde_json::Value> = snapshot
        .devices
        .iter()
        .filter(|device| {
            if query.is_empty() {
                return true;
            }
            let haystack = format!(
                "{} {} {} {} {}",
                device.ip,
                device.mac.clone().unwrap_or_default(),
                device.vendor.clone().unwrap_or_default(),
                device.display_name,
                device
                    .ports
                    .iter()
                    .map(|port| port.port.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .to_ascii_lowercase();
            haystack.contains(&query)
        })
        .take(60)
        .map(|device| {
            json!({
                "ip": device.ip,
                "name": device.display_name,
                "vendor": device.vendor,
                "type": device.device_type,
                "isGateway": device.is_gateway,
                "isThisMachine": device.is_self,
                "isWired": device.is_wired,
                "switchPort": device.switch_port,
                "accessPoint": device.access_point,
                "ports": device.ports.iter().map(|port| port.port).collect::<Vec<_>>(),
            })
        })
        .collect();

    let detail = json!({"matches": matches});
    let id = case.add_evidence(
        "find_devices",
        format!("{} device(s) matching \"{query}\"", matches.len()),
        detail.clone(),
    );
    reply(id, detail)
}

async fn device_detail(
    input: &serde_json::Value,
    case: &mut Case,
    ctx: &ToolContext,
) -> ToolOutcome {
    let Some(ip) = input.get("ip").and_then(|value| value.as_str()) else {
        return ToolOutcome::error("device_detail needs an `ip`.");
    };
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };
    let Some(device) = snapshot.devices.iter().find(|device| device.ip == ip) else {
        return ToolOutcome::error(format!(
            "No device at {ip} in the last scan. Use find_devices to see what is there."
        ));
    };

    let detail = serde_json::to_value(device).unwrap_or(json!({}));
    let id = case.add_evidence(
        "device_detail",
        format!("{ip} — {}", device.display_name),
        detail.clone(),
    );
    reply(id, detail)
}

async fn infrastructure_findings(case: &mut Case, ctx: &ToolContext) -> ToolOutcome {
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };
    let Some(reconciliation) = &snapshot.reconciliation else {
        return ToolOutcome::error(
            "No controller data in the last scan, so there are no infrastructure findings. Port \
             counters, radio airtime and device load all come from the controller.",
        );
    };

    let detail = serde_json::to_value(reconciliation).unwrap_or(json!({}));
    let id = case.add_evidence(
        "infrastructure_findings",
        reconciliation.summary.clone(),
        detail.clone(),
    );
    reply(id, detail)
}

async fn connectivity(case: &mut Case, ctx: &ToolContext) -> ToolOutcome {
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };
    let detail = serde_json::to_value(&snapshot.connectivity).unwrap_or(json!({}));
    let summary = match &snapshot.connectivity.gateway {
        Some(gateway) => format!(
            "gateway {:.1} ms, {:.0}% loss",
            gateway.avg_ms.unwrap_or(0.0),
            gateway.loss_percent
        ),
        None => "no gateway measurement".to_string(),
    };
    let id = case.add_evidence("connectivity", summary, detail.clone());
    reply(id, detail)
}

async fn wifi_survey(case: &mut Case, ctx: &ToolContext) -> ToolOutcome {
    let Some(snapshot) = snapshot(ctx).await else {
        return no_scan();
    };
    let detail = serde_json::to_value(&snapshot.wifi).unwrap_or(json!({}));
    let summary = snapshot
        .wifi
        .data
        .as_ref()
        .and_then(|wifi| wifi.current.as_ref())
        .map(|current| format!("{} on channel {}", current.ssid, current.channel))
        .unwrap_or_else(|| "not on Wi-Fi".to_string());
    let id = case.add_evidence("wifi_survey", summary, detail.clone());
    reply(id, detail)
}

/// Refuses anything outside private space, exactly as the scanner does.
///
/// The assistant chooses its own targets, so this is the boundary that keeps a
/// model's suggestion from becoming a probe of somebody else's network.
fn private_target(input: &serde_json::Value, key: &str) -> Result<Ipv4Addr, ToolOutcome> {
    let Some(text) = input.get(key).and_then(|value| value.as_str()) else {
        return Err(ToolOutcome::error(format!("Missing `{key}`.")));
    };
    let Ok(addr) = text.parse::<Ipv4Addr>() else {
        return Err(ToolOutcome::error(format!(
            "`{text}` is not an IPv4 address."
        )));
    };
    if !netutil::is_private_ipv4(addr) {
        return Err(ToolOutcome::error(format!(
            "{addr} is a public address. This tool only ever probes private networks; pick a \
             local address instead."
        )));
    }
    Ok(addr)
}

async fn ping(input: &serde_json::Value, case: &mut Case) -> ToolOutcome {
    let addr = match private_target(input, "target") {
        Ok(addr) => addr,
        Err(outcome) => return outcome,
    };
    let count = input
        .get("count")
        .and_then(|value| value.as_u64())
        .unwrap_or(10)
        .clamp(3, 50) as u32;

    let stats =
        crate::scan::connectivity::measure_latency(&addr.to_string(), "assistant", count).await;
    let detail = serde_json::to_value(&stats).unwrap_or(json!({}));
    let id = case.add_evidence(
        "ping",
        format!(
            "{addr}: {:.1} ms avg, {:.0}% loss over {count}",
            stats.avg_ms.unwrap_or(0.0),
            stats.loss_percent
        ),
        detail.clone(),
    );
    reply(id, detail)
}

async fn traceroute(input: &serde_json::Value, case: &mut Case) -> ToolOutcome {
    let addr = match private_target(input, "target") {
        Ok(addr) => addr,
        Err(outcome) => return outcome,
    };

    let result = crate::platform::traceroute(&addr.to_string()).await;
    let detail = serde_json::to_value(&result).unwrap_or(json!({}));
    let hops = result
        .data
        .as_ref()
        .map(|trace| trace.hops.len())
        .unwrap_or(0);
    let id = case.add_evidence(
        "traceroute",
        format!("{addr}: {hops} hop(s)"),
        detail.clone(),
    );
    reply(id, detail)
}

async fn port_scan(input: &serde_json::Value, case: &mut Case) -> ToolOutcome {
    let addr = match private_target(input, "ip") {
        Ok(addr) => addr,
        Err(outcome) => return outcome,
    };

    let ports = crate::scan::ports::scan_host(
        addr,
        netutil::ports_for_profile(crate::types::PortProfile::Standard),
        200,
        Duration::from_millis(1200),
    )
    .await;

    let detail = serde_json::to_value(&ports).unwrap_or(json!({}));
    let id = case.add_evidence(
        "port_scan",
        format!("{addr}: {} open port(s)", ports.len()),
        detail.clone(),
    );
    reply(id, detail)
}

/// The before/after measurement.
///
/// A fault that only exists under load is invisible in an idle reading, and
/// this app runs on one machine so it cannot generate the user's traffic for
/// them. Asking them to reproduce while the counters are watched is the honest
/// substitute — and it is what turns "the network freezes" into "these counters
/// moved on this port".
async fn observe(
    input: &serde_json::Value,
    case: &mut Case,
    ctx: &ToolContext,
    progress: &mut (dyn FnMut(ToolProgress) + Send),
) -> ToolOutcome {
    let Some((config, password)) = &ctx.unifi else {
        return ToolOutcome::error(
            "No controller is configured for this network, so there are no counters to watch. \
             Fall back on ping and traceroute during the reproduction, or ask the user to \
             connect a controller.",
        );
    };

    let seconds = input
        .get("seconds")
        .and_then(|value| value.as_u64())
        .unwrap_or(30)
        .clamp(10, 180);
    let reason = input
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("Reproduce the problem now.")
        .to_string();

    let before = match crate::unifi::fetch(config, password).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return ToolOutcome::error(format!(
                "Could not read the controller: {}",
                crate::unifi::config::redact(&error.to_string())
            ))
        }
    };

    progress(ToolProgress::ReproduceNow {
        seconds,
        reason: reason.clone(),
    });
    tokio::time::sleep(Duration::from_secs(seconds)).await;

    let after = match crate::unifi::fetch(config, password).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return ToolOutcome::error(format!(
                "Could not re-read the controller: {}",
                crate::unifi::config::redact(&error.to_string())
            ))
        }
    };

    // Learned before the result is recorded, so a client that joined since the
    // last scan is hidden on the next outbound call rather than leaking.
    ctx.learn_unifi(&before);
    ctx.learn_unifi(&after);

    let deltas = super::observe::diff(&before, &after);
    let detail = serde_json::to_value(&deltas).unwrap_or(json!({}));
    let summary = if deltas.is_empty() {
        format!("nothing changed over {seconds}s")
    } else {
        format!("{} change(s) over {seconds}s", deltas.len())
    };

    let id = case.add_evidence("observe", summary, detail.clone());
    reply(
        id,
        json!({
            "watchedSeconds": seconds,
            "askedUserTo": reason,
            "changes": detail,
            "note": "Only counters that MOVED are listed. An empty list means nothing measurable \
                     changed while the user reproduced it — which is itself a finding.",
        }),
    )
}

fn ask_user(input: &serde_json::Value) -> ToolOutcome {
    let Some(text) = input.get("question").and_then(|value| value.as_str()) else {
        return ToolOutcome::error("ask_user needs a `question`.");
    };
    let choices = input
        .get("choices")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    ToolOutcome {
        content: "Asked. The user's answer will arrive as the next message.".into(),
        is_error: false,
        control: Control::Ask(Question {
            text: text.to_string(),
            choices,
        }),
    }
}

/// Collects cited ids, rejecting any that name a measurement that never
/// happened. A model that invents an id is inventing the reading behind it.
fn cited(input: &serde_json::Value, case: &Case) -> Result<Vec<String>, String> {
    let ids: Vec<String> = input
        .get("evidence_ids")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Err(
            "Cite the evidence ids this rests on — every tool result carries one. If you \
                    have not measured anything that supports it, measure first."
                .into(),
        );
    }

    let unknown: Vec<&String> = ids.iter().filter(|id| !case.has_evidence(id)).collect();
    if !unknown.is_empty() {
        return Err(format!(
            "No evidence with id(s) {:?} exists in this case. Cite only ids returned by tools you \
             actually ran.",
            unknown
        ));
    }

    Ok(ids)
}

fn record_finding(input: &serde_json::Value, case: &mut Case) -> ToolOutcome {
    let ids = match cited(input, case) {
        Ok(ids) => ids,
        Err(message) => return ToolOutcome::error(message),
    };

    let title = input
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let detail = input
        .get("detail")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    if title.trim().is_empty() {
        return ToolOutcome::error("record_finding needs a `title`.");
    }

    case.findings.push(Finding {
        title,
        detail,
        evidence_ids: ids,
    });
    ToolOutcome::ok("Recorded.")
}

fn propose_remedy(input: &serde_json::Value, case: &mut Case) -> ToolOutcome {
    let ids = match cited(input, case) {
        Ok(ids) => ids,
        Err(message) => return ToolOutcome::error(message),
    };

    let text = |key: &str| {
        input
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string()
    };

    let title = text("title");
    if title.trim().is_empty() {
        return ToolOutcome::error("propose_remedy needs a `title`.");
    }

    let steps: Vec<String> = input
        .get("steps")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let confidence = match input.get("confidence").and_then(|value| value.as_str()) {
        Some("high") => Confidence::High,
        Some("low") => Confidence::Low,
        _ => Confidence::Medium,
    };

    case.remedy = Some(Remedy {
        title,
        rationale: text("rationale"),
        steps,
        evidence_ids: ids,
        confidence,
        reversible: input
            .get("reversible")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
    });
    case.status = CaseStatus::Done;

    ToolOutcome {
        content: "Recorded. The case is complete.".into(),
        is_error: false,
        control: Control::Finish,
    }
}

/// Wraps a tool's payload with the evidence id the model must cite.
fn reply(evidence_id: String, detail: serde_json::Value) -> ToolOutcome {
    let body = json!({
        "evidenceId": evidence_id,
        "data": detail,
    });
    ToolOutcome::ok(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case() -> Case {
        Case::new("net-1", "uploads are slow")
    }

    #[test]
    fn every_tool_is_read_only() {
        // The guarantee is structural: a model cannot call a tool that is not
        // in this list, so "it only reads" is enforced by the surface rather
        // than asked for in a prompt.
        let names: Vec<String> = specs().into_iter().map(|spec| spec.name).collect();
        for name in &names {
            assert!(
                !name.contains("set")
                    && !name.contains("apply")
                    && !name.contains("write")
                    && !name.contains("reboot")
                    && !name.contains("delete"),
                "{name} looks like it mutates something"
            );
        }
        assert!(names.contains(&"propose_remedy".to_string()));
    }

    #[test]
    fn every_tool_declares_a_schema_and_a_real_description() {
        for spec in specs() {
            assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
            assert!(
                spec.description.len() > 60,
                "{} needs a description that says when to use it",
                spec.name
            );
        }
    }

    #[test]
    fn a_remedy_without_evidence_is_refused() {
        // The structural guard against a confident invention.
        let mut case = case();
        let outcome = propose_remedy(
            &json!({"title": "Replace the cable", "rationale": "seems likely",
                    "steps": ["swap it"], "evidence_ids": [], "confidence": "high"}),
            &mut case,
        );

        assert!(outcome.is_error);
        assert!(outcome.content.contains("Cite the evidence"));
        assert!(case.remedy.is_none());
        assert_eq!(case.status, CaseStatus::Running);
    }

    #[test]
    fn a_remedy_citing_an_invented_id_is_refused() {
        let mut case = case();
        case.add_evidence("ping", "1 ms", json!({}));

        let outcome = propose_remedy(
            &json!({"title": "x", "rationale": "y", "steps": [],
                    "evidence_ids": ["e1", "e9"], "confidence": "low"}),
            &mut case,
        );

        assert!(outcome.is_error);
        assert!(outcome.content.contains("e9"));
        assert!(case.remedy.is_none());
    }

    #[test]
    fn a_properly_cited_remedy_finishes_the_case() {
        let mut case = case();
        case.add_evidence("infrastructure_findings", "port 7 half duplex", json!({}));

        let outcome = propose_remedy(
            &json!({"title": "Replace the cable to port 7",
                    "rationale": "It negotiated half duplex.",
                    "steps": ["Swap the cable", "Re-run the scan"],
                    "evidence_ids": ["e1"], "confidence": "high", "reversible": true}),
            &mut case,
        );

        assert!(!outcome.is_error);
        assert!(matches!(outcome.control, Control::Finish));
        assert_eq!(case.status, CaseStatus::Done);
        let remedy = case.remedy.unwrap();
        assert_eq!(remedy.confidence, Confidence::High);
        assert_eq!(remedy.steps.len(), 2);
    }

    #[test]
    fn a_finding_is_held_to_the_same_citation_rule() {
        let mut case = case();
        assert!(
            record_finding(
                &json!({"title": "t", "detail": "d", "evidence_ids": []}),
                &mut case
            )
            .is_error
        );
        assert!(case.findings.is_empty());
    }

    #[test]
    fn public_addresses_are_refused_by_every_live_probe() {
        // The assistant picks its own targets, so this is the boundary that
        // stops a suggestion becoming a probe of somebody else's network.
        let outcome = private_target(&json!({"target": "8.8.8.8"}), "target").unwrap_err();
        assert!(outcome.is_error);
        assert!(outcome.content.contains("public address"));

        assert!(private_target(&json!({"target": "10.0.3.1"}), "target").is_ok());
        assert!(private_target(&json!({"target": "not-an-ip"}), "target").is_err());
    }

    #[test]
    fn asking_a_question_stops_the_loop_and_carries_the_choices() {
        let outcome = ask_user(&json!({
            "question": "Are both machines wired?",
            "choices": ["Both wired", "One is Wi-Fi"]
        }));

        match outcome.control {
            Control::Ask(question) => {
                assert_eq!(question.text, "Are both machines wired?");
                assert_eq!(question.choices.len(), 2);
            }
            _ => panic!("ask_user must stop the loop"),
        }
    }

    #[test]
    fn a_tool_result_always_carries_its_evidence_id() {
        let outcome = reply("e3".into(), json!({"speed": 1000}));
        let parsed: serde_json::Value = serde_json::from_str(&outcome.content).unwrap();
        assert_eq!(parsed["evidenceId"], "e3");
        assert_eq!(parsed["data"]["speed"], 1000);
    }

    #[test]
    fn an_unknown_tool_name_is_an_error_the_model_can_recover_from() {
        // Returned as a tool error rather than failing the case: the model can
        // read it and pick a real tool.
        let mut case = case();
        let ctx = ToolContext {
            store: Store::new(std::env::temp_dir().join("netdiag-assist-test")),
            network_name: "Test".into(),
            network_subnets: vec![],
            unifi: None,
            network_root: std::env::temp_dir(),
            redactor: std::sync::Mutex::new(Redactor::new(false)),
        };
        let mut progress = |_: ToolProgress| {};

        let outcome = tokio::runtime::Runtime::new().unwrap().block_on(run(
            "reboot_switch",
            &json!({}),
            &mut case,
            &ctx,
            &mut progress,
        ));

        assert!(outcome.is_error);
        assert!(outcome.content.contains("no tool called"));
    }
}
