//! UniFi controller integration.
//!
//! Read-only. The client issues GETs plus the login/logout pair and nothing
//! else, so a compromised or misconfigured deployment cannot alter the network.
//!
//! What this adds over the scanner alone is covered in
//! [`crate::unifi::correlate`]: the controller knows physical topology,
//! continuous history and configured intent, none of which can be observed from
//! a single host doing port scans.

pub mod client;
pub mod config;
pub mod correlate;
pub mod model;

use serde::Serialize;

pub use client::{Established, Flavour, UnifiClient, UnifiError};
pub use config::UnifiConfig;
pub use model::{UnifiClientRecord, UnifiDeviceRecord, UnifiSnapshot};

/// Endpoints fetched in one pass, with the label used if one fails.
const ENDPOINTS: &[(&str, &str)] = &[
    ("stat/sta", "active clients"),
    ("stat/device", "managed devices"),
    ("stat/alluser", "known clients"),
    ("stat/health", "site health"),
    ("rest/networkconf", "configured networks"),
    // Query strings ride along: `get_data` appends the endpoint verbatim.
    ("stat/event?_limit=200&within=24", "recent events"),
    ("list/alarm?archived=false", "active alarms"),
    ("stat/rogueap?within=24", "nearby access points"),
];

/// Keep only the strongest few neighbors; evil twins always survive the cut.
const NEIGHBOR_AP_CAP: usize = 30;

/// Signs in, fetches everything, signs out.
///
/// A failure on any single endpoint becomes a warning rather than an error: an
/// account restricted from one collection should still deliver the rest, in
/// keeping with the degrade-never-fail rule the scanner follows everywhere else.
pub async fn fetch(config: &UnifiConfig, password: &str) -> Result<UnifiSnapshot, UnifiError> {
    let mut client = UnifiClient::new(&config.host, config.port, config.fingerprint.clone())?;
    let established = client.login(&config.username, password).await?;

    let mut snapshot = UnifiSnapshot {
        controller_host: config.host.clone(),
        site: config.site.clone(),
        ..Default::default()
    };

    if established.newly_pinned {
        snapshot.warnings.push(format!(
            "Trusted this controller's certificate for the first time ({}). \
             Future connections are checked against it.",
            established.fingerprint
        ));
    }

    let mut raw_alarms: Vec<model::UnifiAlarmRecord> = Vec::new();
    let mut raw_rogues: Vec<model::UnifiRogueApRecord> = Vec::new();

    for (endpoint, label) in ENDPOINTS {
        match client.get_data(&config.site, endpoint).await {
            // Matching on the path alone keeps query strings out of the arms.
            Ok(value) => match endpoint.split('?').next().unwrap_or(endpoint) {
                "stat/sta" => snapshot.clients = parse_list(value),
                "stat/alluser" => snapshot.known_clients = parse_list(value),
                "stat/device" => {
                    let devices: Vec<model::UnifiDeviceRecord> = parse_list(value);
                    snapshot.devices = devices.iter().map(Into::into).collect();
                    snapshot.raw_devices = devices;
                }
                "stat/health" => {
                    let records: Vec<model::UnifiHealthRecord> = parse_list(value);
                    snapshot.health = records.iter().map(Into::into).collect();
                }
                "rest/networkconf" => {
                    let records: Vec<model::UnifiNetworkConf> = parse_list(value);
                    snapshot.networks = records.iter().map(Into::into).collect();
                }
                "stat/event" => snapshot.raw_events = parse_list(value),
                "list/alarm" => raw_alarms = parse_list(value),
                "stat/rogueap" => raw_rogues = parse_list(value),
                _ => {}
            },
            Err(error) => {
                let mut message = format!("Could not read {label}: {error}");
                // `rest/` is configuration, which some read-only roles cannot
                // see even though they can read every `stat/` collection. Name
                // the fix, since "forbidden" alone points nowhere.
                if error == UnifiError::Forbidden && endpoint.starts_with("rest/") {
                    message.push_str(
                        " — reading configuration needs the account's Network role \
                         to be Viewer (or higher), not a restricted read-only role.",
                    );
                }
                snapshot.warnings.push(message);
            }
        }
    }

    client.logout().await;

    snapshot.wan_triage = model::wan_triage(&snapshot.health);
    snapshot.alarms = raw_alarms
        .iter()
        .filter_map(model::UnifiAlarmRecord::summarize)
        .collect();

    // The evil-twin check compares overheard SSIDs against the ones this
    // site's own clients associate to. Client records are the best available
    // source of "our SSIDs" without fetching the WLAN configuration.
    let own_ssids: std::collections::HashSet<String> = snapshot
        .clients
        .iter()
        .chain(&snapshot.known_clients)
        .filter_map(|client| client.essid.clone())
        .filter(|ssid| !ssid.trim().is_empty())
        .collect();
    let (neighbors, total) =
        model::summarize_neighbor_aps(&raw_rogues, &own_ssids, NEIGHBOR_AP_CAP);
    snapshot.neighbor_aps = neighbors;
    snapshot.neighbor_ap_total = total;

    // Losing every collection means the credentials work but the account can see
    // nothing — worth surfacing as an error rather than an empty success.
    if snapshot.clients.is_empty()
        && snapshot.known_clients.is_empty()
        && snapshot.raw_devices.is_empty()
        && snapshot.health.is_empty()
        && !snapshot.warnings.is_empty()
    {
        return Err(UnifiError::Forbidden);
    }

    Ok(snapshot)
}

/// Fetches one endpoint verbatim, for checking what a controller actually sends.
///
/// Field availability varies by UniFi OS release: a parser written against the
/// documentation can silently deserialize to `None` forever, and a diagnosis
/// built on a field that is never populated is worse than one that admits it
/// cannot tell. This exists so the structs can be checked against the real
/// payload rather than assumed.
///
/// The result is the raw `data` array. Callers are expected to redact before
/// showing or storing it — it contains MACs, hostnames and SSIDs.
pub async fn dump_endpoint(
    config: &UnifiConfig,
    password: &str,
    endpoint: &str,
) -> Result<serde_json::Value, UnifiError> {
    let mut client = UnifiClient::new(&config.host, config.port, config.fingerprint.clone())?;
    client.login(&config.username, password).await?;
    let data = client.get_data(&config.site, endpoint).await;
    client.logout().await;
    data
}

/// What one diagnostic needs from the controller in order to say anything.
///
/// Declared beside the detectors rather than in the desktop shell, so adding a
/// finding cannot leave a separately-maintained list behind. This is what keeps
/// the app honest across UniFi releases without being tuned to any one of them:
/// a controller that does not report a field makes the finding *unavailable*,
/// which is a different answer from "healthy".
pub struct DiagnosticRequirement {
    /// The finding, named as the user sees it.
    pub finding: &'static str,
    /// Which collection carries the evidence.
    pub endpoint: &'static str,
    pub fields: &'static [&'static str],
}

pub const DIAGNOSTIC_REQUIREMENTS: &[DiagnosticRequirement] = &[
    DiagnosticRequirement {
        finding: "Half-duplex links",
        endpoint: "stat/device",
        fields: &["port_table.full_duplex"],
    },
    DiagnosticRequirement {
        finding: "Port errors and drops",
        endpoint: "stat/device",
        fields: &["port_table.rx_errors", "port_table.tx_errors"],
    },
    DiagnosticRequirement {
        finding: "Spanning-tree blocked ports",
        endpoint: "stat/device",
        fields: &["port_table.stp_state"],
    },
    DiagnosticRequirement {
        finding: "Devices under load",
        endpoint: "stat/device",
        fields: &["system-stats.cpu"],
    },
    DiagnosticRequirement {
        finding: "Saturated radios",
        endpoint: "stat/device",
        fields: &["radio_table_stats.cu_total"],
    },
    DiagnosticRequirement {
        finding: "Interference share",
        endpoint: "stat/device",
        fields: &[
            "radio_table_stats.cu_self_rx",
            "radio_table_stats.cu_self_tx",
        ],
    },
    DiagnosticRequirement {
        finding: "Wireless backhaul",
        endpoint: "stat/device",
        fields: &["uplink.type"],
    },
    DiagnosticRequirement {
        finding: "Client PHY rates",
        endpoint: "stat/sta",
        fields: &["tx_rate", "rx_rate"],
    },
    DiagnosticRequirement {
        finding: "Client retry rate",
        endpoint: "stat/sta",
        fields: &["tx_retries", "tx_packets"],
    },
];

/// Whether a diagnostic can run against this controller, and what is missing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCoverage {
    pub finding: String,
    pub endpoint: String,
    /// Every field it needs is reported by at least one record.
    pub available: bool,
    pub missing: Vec<String>,
}

/// Reports which diagnostics this controller's payload can support.
///
/// Only the requirements naming `endpoint` are judged; the rest are left for a
/// call against their own collection, so one payload never marks another
/// endpoint's findings unavailable.
pub fn report_diagnostic_coverage(
    data: &serde_json::Value,
    endpoint: &str,
) -> Vec<DiagnosticCoverage> {
    let serde_json::Value::Array(items) = data else {
        return Vec::new();
    };

    DIAGNOSTIC_REQUIREMENTS
        .iter()
        .filter(|requirement| requirement.endpoint == endpoint)
        .map(|requirement| {
            let missing: Vec<String> = requirement
                .fields
                .iter()
                .filter(|field| !items.iter().any(|item| field_present(item, field)))
                .map(|field| (*field).to_string())
                .collect();

            DiagnosticCoverage {
                finding: requirement.finding.to_string(),
                endpoint: requirement.endpoint.to_string(),
                available: missing.is_empty(),
                missing,
            }
        })
        .collect()
}

/// Whether a record carries a field, looking one level into arrays so
/// `port_table.rx_errors` can be asked about directly.
fn field_present(item: &serde_json::Value, field: &str) -> bool {
    match field.split_once('.') {
        None => item.get(field).is_some_and(|v| !v.is_null()),
        Some((outer, inner)) => match item.get(outer) {
            Some(serde_json::Value::Array(entries)) => {
                entries.iter().any(|entry| field_present(entry, inner))
            }
            Some(nested) => field_present(nested, inner),
            None => false,
        },
    }
}

/// Deserializes an array, skipping entries that fail rather than the whole set.
fn parse_list<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Vec<T> {
    let serde_json::Value::Array(items) = value else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect()
}

/// Confirms the controller is reachable, pinned and readable. Used by the doctor
/// and by the "Test connection" button.
pub async fn verify(config: &UnifiConfig, password: &str) -> Result<VerifyReport, UnifiError> {
    let mut client = UnifiClient::new(&config.host, config.port, config.fingerprint.clone())?;
    let established = client.login(&config.username, password).await?;

    let clients = client.get_data(&config.site, "stat/sta").await?;
    let count = clients.as_array().map(|a| a.len()).unwrap_or(0);

    client.logout().await;

    Ok(VerifyReport {
        flavour: established.flavour,
        fingerprint: established.fingerprint,
        newly_pinned: established.newly_pinned,
        client_count: count,
    })
}

#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub flavour: Flavour,
    pub fingerprint: String,
    pub newly_pinned: bool,
    pub client_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn availability(coverage: &[DiagnosticCoverage], finding: &str) -> bool {
        coverage
            .iter()
            .find(|entry| entry.finding == finding)
            .unwrap_or_else(|| panic!("no requirement declared for {finding}"))
            .available
    }

    #[test]
    fn a_diagnostic_is_available_when_any_record_carries_its_evidence() {
        // A modern switch reporting counters alongside an older one that does
        // not: the finding can still be produced, for the device that reports.
        let data = serde_json::json!([
            {"name":"USW-24","port_table":[
                {"port_idx":1,"rx_errors":0,"tx_errors":0,"full_duplex":true}
            ], "system-stats":{"cpu":"12"}},
            {"name":"USW-MINI","port_table":[{"port_idx":1}]}
        ]);

        let coverage = report_diagnostic_coverage(&data, "stat/device");
        assert!(availability(&coverage, "Port errors and drops"));
        assert!(availability(&coverage, "Half-duplex links"));
        assert!(availability(&coverage, "Devices under load"));
        assert!(!availability(&coverage, "Saturated radios"));
        assert!(!availability(&coverage, "Wireless backhaul"));
    }

    #[test]
    fn a_diagnostic_needing_two_fields_is_unavailable_when_either_is_missing() {
        // Interference needs both self-utilisation figures; with only the total
        // it cannot be split, and reporting it anyway would blame neighbours
        // for this network's own traffic.
        let data = serde_json::json!([
            {"radio_table_stats":[{"radio":"ng","cu_total":80,"cu_self_rx":10}]}
        ]);

        let coverage = report_diagnostic_coverage(&data, "stat/device");
        assert!(availability(&coverage, "Saturated radios"));

        let interference = coverage
            .iter()
            .find(|entry| entry.finding == "Interference share")
            .unwrap();
        assert!(!interference.available);
        assert_eq!(interference.missing, vec!["radio_table_stats.cu_self_tx"]);
    }

    #[test]
    fn only_the_requirements_for_the_queried_endpoint_are_judged() {
        // A device payload says nothing about client-side findings; marking
        // them unavailable from it would be wrong.
        let data = serde_json::json!([{"name":"USW-24"}]);
        let coverage = report_diagnostic_coverage(&data, "stat/device");
        assert!(
            coverage.iter().all(|entry| entry.endpoint == "stat/device"),
            "a payload must not judge another collection's findings"
        );
    }

    #[test]
    fn a_null_field_counts_as_absent() {
        // The controller sends explicit nulls for things it has no value for;
        // treating those as present would report a diagnostic as available when
        // it can never fire.
        let nulls = serde_json::json!([{"uplink":{"type":null}}]);
        let values = serde_json::json!([{"uplink":{"type":"wireless"}}]);

        assert!(!availability(
            &report_diagnostic_coverage(&nulls, "stat/device"),
            "Wireless backhaul"
        ));
        assert!(availability(
            &report_diagnostic_coverage(&values, "stat/device"),
            "Wireless backhaul"
        ));
    }

    #[test]
    fn a_non_array_payload_yields_no_coverage_rather_than_panicking() {
        let data = serde_json::json!({"meta": {"rc": "ok"}});
        assert!(report_diagnostic_coverage(&data, "stat/device").is_empty());
    }

    #[test]
    fn a_malformed_entry_does_not_discard_the_rest() {
        let value = serde_json::json!([
            {"mac":"aa:bb:cc:dd:ee:ff","ip":"10.0.3.5"},
            "not an object",
            {"mac":"11:22:33:44:55:66","ip":"10.0.3.6"}
        ]);
        let records: Vec<model::UnifiClientRecord> = parse_list(value);
        assert_eq!(
            records.len(),
            2,
            "one bad entry must not lose the good ones"
        );
    }

    #[test]
    fn a_non_array_payload_yields_nothing_rather_than_panicking() {
        let records: Vec<model::UnifiClientRecord> = parse_list(serde_json::json!({"rc":"ok"}));
        assert!(records.is_empty());
    }
}
