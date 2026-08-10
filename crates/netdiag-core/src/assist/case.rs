//! One troubleshooting session, from symptom to remedy.
//!
//! A case is persisted for the same reason a scan is: the work is worth
//! keeping. It also has to survive being paused — the assistant asks questions
//! only the user can answer, and the answer may come minutes later or after a
//! restart. Holding the whole state in a file rather than in memory makes that
//! ordinary rather than special.
//!
//! Stored per network, beside that network's scans, because a symptom is
//! always a symptom *of* a network.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::provider::{Message, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaseStatus {
    /// The assistant is working.
    Running,
    /// Stopped on a question only the user can answer.
    WaitingForAnswer,
    /// Reached a remedy, or said why it could not.
    Done,
    /// The user stopped it.
    Cancelled,
    /// The model or a probe failed in a way the case could not continue past.
    Failed,
}

/// One measurement, kept so a finding can point at what it rests on.
///
/// The `id` is what the model cites. Keeping the full `detail` alongside the
/// one-line `summary` means a claim can always be checked against the reading
/// that produced it, rather than taken on trust.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub id: String,
    /// The tool that produced it.
    pub tool: String,
    /// One line, for the transcript.
    pub summary: String,
    pub detail: serde_json::Value,
    pub recorded_at: String,
}

/// Something the assistant concluded, and what it rests on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub title: String,
    pub detail: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// What to do about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Remedy {
    pub title: String,
    pub rationale: String,
    /// Steps the user performs. The assistant changes nothing itself.
    pub steps: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub confidence: Confidence,
    /// Whether following the steps is easy to undo — stated so the user can
    /// judge the risk before acting, not discovered afterwards.
    pub reversible: bool,
}

/// A question the assistant cannot answer by measuring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub text: String,
    /// Offered answers. Empty means free text.
    #[serde(default)]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Case {
    pub id: String,
    /// The network this is a symptom of.
    pub network_id: String,
    pub symptom: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: CaseStatus,
    /// The provider conversation, replayed on every resume.
    pub transcript: Vec<Message>,
    pub evidence: Vec<Evidence>,
    pub findings: Vec<Finding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remedy: Option<Remedy>,
    /// Set only while `status` is `WaitingForAnswer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    /// Why it stopped, when that needs saying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub usage: Usage,
    /// Model round trips so far. Bounded — see [`super::session::Limits`].
    #[serde(default)]
    pub steps: u32,
}

impl Case {
    pub fn new(network_id: impl Into<String>, symptom: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        let symptom = symptom.into();
        Self {
            id: make_id(now),
            network_id: network_id.into(),
            created_at: now.to_rfc3339(),
            updated_at: now.to_rfc3339(),
            status: CaseStatus::Running,
            transcript: vec![Message::User {
                text: symptom.clone(),
            }],
            symptom,
            evidence: Vec::new(),
            findings: Vec::new(),
            remedy: None,
            question: None,
            note: None,
            usage: Usage::default(),
            steps: 0,
        }
    }

    /// Records a measurement and returns the id the model must cite.
    pub fn add_evidence(
        &mut self,
        tool: impl Into<String>,
        summary: impl Into<String>,
        detail: serde_json::Value,
    ) -> String {
        let id = format!("e{}", self.evidence.len() + 1);
        self.evidence.push(Evidence {
            id: id.clone(),
            tool: tool.into(),
            summary: summary.into(),
            detail,
            recorded_at: chrono::Utc::now().to_rfc3339(),
        });
        id
    }

    pub fn has_evidence(&self, id: &str) -> bool {
        self.evidence.iter().any(|item| item.id == id)
    }

    /// Whether the assistant is finished with this case for now.
    pub fn is_settled(&self) -> bool {
        !matches!(self.status, CaseStatus::Running)
    }

    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    /* ------------------------------------------------------------- storage */

    pub fn dir(network_root: &Path) -> PathBuf {
        network_root.join("cases")
    }

    pub fn path(network_root: &Path, id: &str) -> PathBuf {
        Self::dir(network_root).join(format!("{id}.json"))
    }

    pub async fn save(&self, network_root: &Path) -> Result<(), String> {
        let dir = Self::dir(network_root);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        tokio::fs::write(Self::path(network_root, &self.id), json)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn load(network_root: &Path, id: &str) -> Option<Self> {
        let bytes = tokio::fs::read(Self::path(network_root, id)).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn delete(network_root: &Path, id: &str) -> bool {
        tokio::fs::remove_file(Self::path(network_root, id))
            .await
            .is_ok()
    }

    /// Newest first, so the list reads like a history.
    pub async fn list(network_root: &Path, limit: usize) -> Vec<CaseSummary> {
        let Ok(mut entries) = tokio::fs::read_dir(Self::dir(network_root)).await else {
            return Vec::new();
        };

        let mut ids = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".json") {
                ids.push(id.to_string());
            }
        }
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.truncate(limit);

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(case) = Self::load(network_root, &id).await {
                out.push(CaseSummary {
                    id: case.id,
                    symptom: case.symptom,
                    status: case.status,
                    created_at: case.created_at,
                    remedy_title: case.remedy.map(|remedy| remedy.title),
                    finding_count: case.findings.len(),
                });
            }
        }
        out
    }
}

/// A case as it appears in a list, without the transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseSummary {
    pub id: String,
    pub symptom: String,
    pub status: CaseStatus,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy_title: Option<String>,
    pub finding_count: usize,
}

/// Time-ordered, filesystem-safe. Sorting the directory listing sorts by age,
/// which is what the case list wants.
fn make_id(at: chrono::DateTime<chrono::Utc>) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("case-{}-{:05}", at.format("%Y%m%d-%H%M%S"), nanos % 100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_case_opens_with_the_symptom_as_the_first_turn() {
        let case = Case::new("net-1", "uploads freeze the network");
        assert_eq!(case.status, CaseStatus::Running);
        assert_eq!(case.transcript.len(), 1);
        assert!(matches!(&case.transcript[0], Message::User { text } if *text == case.symptom));
        assert!(!case.is_settled());
    }

    #[test]
    fn evidence_ids_are_sequential_and_citable() {
        let mut case = Case::new("net-1", "slow");
        let first = case.add_evidence("ping", "1.2 ms to gateway", serde_json::json!({}));
        let second = case.add_evidence("wifi", "channel 6, 92% busy", serde_json::json!({}));

        assert_eq!(first, "e1");
        assert_eq!(second, "e2");
        assert!(case.has_evidence("e2"));
        assert!(!case.has_evidence("e3"), "an uncited id must not validate");
    }

    #[test]
    fn a_case_round_trips_through_json() {
        // It is persisted between a question and its answer, so this is the
        // ordinary path rather than an edge case.
        let mut case = Case::new("net-1", "slow uploads");
        case.add_evidence("ping", "1 ms", serde_json::json!({"avg": 1.0}));
        case.findings.push(Finding {
            title: "Half duplex on port 7".into(),
            detail: "…".into(),
            evidence_ids: vec!["e1".into()],
        });
        case.status = CaseStatus::WaitingForAnswer;
        case.question = Some(Question {
            text: "Are both machines wired?".into(),
            choices: vec!["Both wired".into(), "One is Wi-Fi".into()],
        });

        let json = serde_json::to_string(&case).unwrap();
        let back: Case = serde_json::from_str(&json).unwrap();

        assert_eq!(back.status, CaseStatus::WaitingForAnswer);
        assert_eq!(back.question.unwrap().choices.len(), 2);
        assert_eq!(back.findings[0].evidence_ids, vec!["e1"]);
        assert_eq!(back.evidence[0].detail["avg"], 1.0);
    }

    #[test]
    fn every_status_other_than_running_is_settled() {
        let mut case = Case::new("net-1", "x");
        for status in [
            CaseStatus::WaitingForAnswer,
            CaseStatus::Done,
            CaseStatus::Cancelled,
            CaseStatus::Failed,
        ] {
            case.status = status;
            assert!(case.is_settled(), "{status:?} should stop the loop");
        }
        case.status = CaseStatus::Running;
        assert!(!case.is_settled());
    }

    #[test]
    fn ids_sort_newest_first_by_string_order() {
        let early = make_id(
            chrono::DateTime::parse_from_rfc3339("2026-01-01T10:00:00Z")
                .unwrap()
                .into(),
        );
        let later = make_id(
            chrono::DateTime::parse_from_rfc3339("2026-06-01T10:00:00Z")
                .unwrap()
                .into(),
        );
        assert!(later > early, "the case list is sorted by id, newest first");
    }
}
