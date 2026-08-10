//! What moved while the user reproduced the problem.
//!
//! This app runs on one machine, so it cannot generate a transfer between two
//! *other* hosts and watch it. What it can do is read the infrastructure's own
//! counters either side of the user doing it themselves. That before/after
//! difference is the single most useful measurement the assistant has:
//!
//! - A lifetime error counter tells you a port has had errors *sometime*.
//!   A counter that rose by 14,000 during a 30-second transfer tells you the
//!   port has errors *now, under this load* — which is a diagnosis.
//! - Byte counters show which ports the traffic actually crossed, so a claim
//!   about the path rests on where the bytes went rather than on the topology
//!   the model assumed.
//!
//! Only counters that moved are reported. An empty result is itself a finding:
//! whatever the user did, it did not touch the managed infrastructure.

use serde::Serialize;
use std::collections::HashMap;

use crate::unifi::model::{UnifiClientRecord, UnifiDeviceRecord, UnifiSnapshot};

/// What kind of reading changed, so the model can weigh them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    /// Frames the port could not receive or send cleanly. Never normal.
    Errors,
    /// Frames discarded, usually for want of buffer. Congestion, not damage.
    Drops,
    /// Bytes across a port — where the traffic went.
    Traffic,
    /// Radio airtime consumed.
    Airtime,
    /// CPU on a managed device.
    Load,
    /// Wireless frames that had to be sent again.
    Retries,
}

/// One counter that moved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// What it is, in the operator's terms: "USW-MINI port 7".
    pub subject: String,
    pub kind: ChangeKind,
    pub before: f64,
    pub after: f64,
    pub delta: f64,
    pub unit: &'static str,
    /// What this reading means, so a number is never presented bare.
    pub note: String,
}

/// Counters that rose by less than this are noise, not a symptom.
const TRAFFIC_FLOOR_BYTES: f64 = 1_000_000.0;
/// Airtime swings smaller than this are ordinary background variation.
const AIRTIME_FLOOR_POINTS: f64 = 5.0;
const LOAD_FLOOR_POINTS: f64 = 10.0;

/// Compares two controller readings and reports what changed.
pub fn diff(before: &UnifiSnapshot, after: &UnifiSnapshot) -> Vec<Change> {
    let mut changes = Vec::new();
    diff_devices(before, after, &mut changes);
    diff_clients(before, after, &mut changes);

    // Errors first, then drops, then everything else by size: the ordering the
    // model should read them in, rather than whatever order the controller
    // happened to list devices.
    changes.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then(b.delta.abs().total_cmp(&a.delta.abs()))
    });
    changes
}

fn rank(kind: ChangeKind) -> u8 {
    match kind {
        ChangeKind::Errors => 0,
        ChangeKind::Drops => 1,
        ChangeKind::Retries => 2,
        ChangeKind::Airtime => 3,
        ChangeKind::Load => 4,
        ChangeKind::Traffic => 5,
    }
}

fn device_label(device: &UnifiDeviceRecord) -> String {
    device
        .name
        .clone()
        .or_else(|| device.model.clone())
        .or_else(|| device.mac.clone())
        .unwrap_or_else(|| "unnamed device".to_string())
}

fn client_label(client: &UnifiClientRecord) -> String {
    client
        .name
        .clone()
        .or_else(|| client.hostname.clone())
        .or_else(|| client.ip.clone())
        .or_else(|| client.mac.clone())
        .unwrap_or_else(|| "unnamed client".to_string())
}

fn diff_devices(before: &UnifiSnapshot, after: &UnifiSnapshot, out: &mut Vec<Change>) {
    let earlier: HashMap<&str, &UnifiDeviceRecord> = before
        .raw_devices
        .iter()
        .filter_map(|device| device.mac.as_deref().map(|mac| (mac, device)))
        .collect();

    for device in &after.raw_devices {
        let Some(mac) = device.mac.as_deref() else {
            continue;
        };
        let Some(was) = earlier.get(mac) else {
            continue;
        };
        let name = device_label(device);

        diff_ports(&name, was, device, out);
        diff_radios(&name, was, device, out);

        // A gateway that pins its CPU during a transfer is the "the whole
        // network freezes" symptom, and nothing on a per-port counter shows it.
        if let (Some(then), Some(now)) = (
            was.system_stats.as_ref().and_then(|stats| stats.cpu),
            device.system_stats.as_ref().and_then(|stats| stats.cpu),
        ) {
            let delta = now - then;
            if delta.abs() >= LOAD_FLOOR_POINTS || now >= 80.0 {
                out.push(Change {
                    subject: name.clone(),
                    kind: ChangeKind::Load,
                    before: then,
                    after: now,
                    delta,
                    unit: "% CPU",
                    note: if now >= 80.0 {
                        "This device is close to its processing limit. Traffic it has to route \
                         or inspect will stall regardless of link speed."
                            .into()
                    } else {
                        "CPU moved during the reproduction.".into()
                    },
                });
            }
        }
    }
}

fn diff_ports(
    device_name: &str,
    before: &UnifiDeviceRecord,
    after: &UnifiDeviceRecord,
    out: &mut Vec<Change>,
) {
    let Some(after_ports) = after.port_table.as_ref() else {
        return;
    };
    let empty = Vec::new();
    let before_ports = before.port_table.as_ref().unwrap_or(&empty);

    for port in after_ports {
        let Some(idx) = port.port_idx else { continue };
        let Some(was) = before_ports
            .iter()
            .find(|entry| entry.port_idx == Some(idx))
        else {
            continue;
        };

        let subject = match port.name.as_deref() {
            Some(name) if !name.is_empty() => format!("{device_name} port {idx} ({name})"),
            _ => format!("{device_name} port {idx}"),
        };

        // `error_total()` returns None when the controller does not report the
        // counter at all — which must not be read as zero, or a switch that
        // reports nothing would look perfectly healthy.
        if let (Some(then), Some(now)) = (was.error_total(), port.error_total()) {
            let delta = now - then;
            if delta > 0 {
                out.push(Change {
                    subject: subject.clone(),
                    kind: ChangeKind::Errors,
                    before: then as f64,
                    after: now as f64,
                    delta: delta as f64,
                    unit: "frames",
                    note: format!(
                        "{delta} frame(s) failed on this port during the reproduction. Errors \
                         under load point at the cable, the connector or the port itself — a \
                         link can show full speed and still corrupt frames.{}",
                        if port.is_half_duplex() {
                            " This port is also running half duplex, which is the usual cause."
                        } else {
                            ""
                        }
                    ),
                });
            }
        }

        if let (Some(then), Some(now)) = (was.drop_total(), port.drop_total()) {
            let delta = now - then;
            if delta > 0 {
                out.push(Change {
                    subject: subject.clone(),
                    kind: ChangeKind::Drops,
                    before: then as f64,
                    after: now as f64,
                    delta: delta as f64,
                    unit: "frames",
                    note: format!(
                        "{delta} frame(s) were discarded here. Drops without errors usually mean \
                         congestion — more arriving than the port could forward."
                    ),
                });
            }
        }

        // Where the bytes went. This is what lets a claim about the path rest
        // on measurement rather than on assumed topology.
        let moved = [
            (was.rx_bytes, port.rx_bytes, "received"),
            (was.tx_bytes, port.tx_bytes, "sent"),
        ];
        for (then, now, direction) in moved {
            let (Some(then), Some(now)) = (then, now) else {
                continue;
            };
            let delta = (now - then) as f64;
            if delta >= TRAFFIC_FLOOR_BYTES {
                out.push(Change {
                    subject: subject.clone(),
                    kind: ChangeKind::Traffic,
                    before: then as f64,
                    after: now as f64,
                    delta,
                    unit: "bytes",
                    note: format!(
                        "{} {direction} across this port during the reproduction — the traffic \
                         did cross it.",
                        human_bytes(delta)
                    ),
                });
            }
        }
    }
}

fn diff_radios(
    device_name: &str,
    before: &UnifiDeviceRecord,
    after: &UnifiDeviceRecord,
    out: &mut Vec<Change>,
) {
    let Some(after_radios) = after.radio_table_stats.as_ref() else {
        return;
    };
    let empty = Vec::new();
    let before_radios = before.radio_table_stats.as_ref().unwrap_or(&empty);

    for radio in after_radios {
        let key = radio.name.as_deref().or(radio.radio.as_deref());
        let Some(was) = before_radios
            .iter()
            .find(|entry| entry.name.as_deref().or(entry.radio.as_deref()) == key)
        else {
            continue;
        };
        let (Some(then), Some(now)) = (was.cu_total, radio.cu_total) else {
            continue;
        };

        let delta = now - then;
        if delta.abs() < AIRTIME_FLOOR_POINTS && now < 70.0 {
            continue;
        }

        let band = radio.radio.clone().unwrap_or_else(|| "radio".to_string());
        out.push(Change {
            subject: format!(
                "{device_name} {band}{}",
                radio
                    .channel
                    .map(|channel| format!(" (channel {channel})"))
                    .unwrap_or_default()
            ),
            kind: ChangeKind::Airtime,
            before: then,
            after: now,
            delta,
            unit: "% airtime",
            note: if now >= 70.0 {
                "This radio's airtime is nearly spent. Wi-Fi is one shared medium per channel, \
                 so every client on it slows down together — including ones with a strong signal."
                    .into()
            } else {
                "Airtime moved during the reproduction.".into()
            },
        });
    }
}

fn diff_clients(before: &UnifiSnapshot, after: &UnifiSnapshot, out: &mut Vec<Change>) {
    let earlier: HashMap<&str, &UnifiClientRecord> = before
        .clients
        .iter()
        .filter_map(|client| client.mac.as_deref().map(|mac| (mac, client)))
        .collect();

    for client in &after.clients {
        let Some(mac) = client.mac.as_deref() else {
            continue;
        };
        let Some(was) = earlier.get(mac) else {
            continue;
        };

        // Retries are counted as a share of frames sent, not as a raw total: a
        // busy client naturally retries more often than an idle one, and only
        // the proportion says whether the link is struggling.
        let (Some(then), Some(now)) = (was.retry_percent(), client.retry_percent()) else {
            continue;
        };
        let delta = now - then;
        if delta.abs() < 3.0 && now < 20.0 {
            continue;
        }

        out.push(Change {
            subject: client_label(client),
            kind: ChangeKind::Retries,
            before: then,
            after: now,
            delta,
            unit: "% of frames retried",
            note: if now >= 20.0 {
                "A fifth or more of this client's frames had to be sent again. Each retry \
                 occupies the channel, so a struggling client slows the whole cell."
                    .into()
            } else {
                "Retry rate moved during the reproduction.".into()
            },
        });
    }
}

fn human_bytes(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unifi::model::{PortEntry, RadioStats, SystemStats};

    fn snapshot(devices: Vec<UnifiDeviceRecord>) -> UnifiSnapshot {
        UnifiSnapshot {
            controller_host: "gateway".into(),
            site: "default".into(),
            devices: Vec::new(),
            health: Vec::new(),
            networks: Vec::new(),
            alarms: Vec::new(),
            neighbor_aps: Vec::new(),
            neighbor_ap_total: 0,
            wan_triage: None,
            warnings: Vec::new(),
            clients: Vec::new(),
            known_clients: Vec::new(),
            raw_devices: devices,
            raw_events: Vec::new(),
        }
    }

    /// `errors: None` is a controller that reports no error counter at all —
    /// which is why both directions go unset, not just one.
    fn port(idx: u32, errors: Option<i64>, rx_bytes: Option<i64>) -> PortEntry {
        PortEntry {
            port_idx: Some(idx),
            name: None,
            up: Some(true),
            speed: Some(1000),
            full_duplex: Some(true),
            poe_enable: None,
            poe_power: None,
            rx_errors: errors,
            tx_errors: errors.map(|_| 0),
            rx_dropped: None,
            tx_dropped: None,
            rx_bytes,
            tx_bytes: None,
            stp_state: None,
            is_uplink: Some(false),
            mac_table: None,
        }
    }

    fn switch(ports: Vec<PortEntry>) -> UnifiDeviceRecord {
        UnifiDeviceRecord {
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            ip: None,
            name: Some("USW-MINI".into()),
            model: None,
            kind: Some("usw".into()),
            version: None,
            adopted: Some(true),
            state: Some(1),
            uptime: None,
            upgradable: None,
            port_table: Some(ports),
            system_stats: None,
            radio_table_stats: None,
            uplink: None,
        }
    }

    #[test]
    fn a_port_that_starts_erroring_under_load_is_reported_first() {
        // The scenario the whole tool exists for: the counter was flat, the
        // user reproduced the problem, and it moved.
        let before = snapshot(vec![switch(vec![port(7, Some(100), None)])]);
        let after = snapshot(vec![switch(vec![port(7, Some(14_300), None)])]);

        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Errors);
        assert_eq!(changes[0].delta, 14_200.0);
        assert!(changes[0].subject.contains("port 7"));
        assert!(changes[0].note.contains("14200 frame(s) failed"));
    }

    #[test]
    fn a_flat_counter_is_not_reported_at_all() {
        // Only what moved. A list of unchanged readings would bury the signal.
        let before = snapshot(vec![switch(vec![port(1, Some(5), Some(1000))])]);
        let after = snapshot(vec![switch(vec![port(1, Some(5), Some(1200))])]);
        assert!(diff(&before, &after).is_empty());
    }

    #[test]
    fn a_counter_the_controller_never_reports_is_not_treated_as_zero() {
        // Otherwise a switch that reports nothing looks perfectly healthy,
        // which is exactly the wrong answer to give someone.
        let before = snapshot(vec![switch(vec![port(1, None, None)])]);
        let after = snapshot(vec![switch(vec![port(1, None, None)])]);
        assert!(diff(&before, &after).is_empty());

        // And an appearing counter is not read as a delta from zero either.
        let after_with = snapshot(vec![switch(vec![port(1, Some(9_000), None)])]);
        assert!(diff(&before, &after_with).is_empty());
    }

    #[test]
    fn traffic_shows_which_port_carried_the_transfer() {
        let before = snapshot(vec![switch(vec![
            port(1, Some(0), Some(0)),
            port(2, Some(0), Some(0)),
        ])]);
        let after = snapshot(vec![switch(vec![
            port(1, Some(0), Some(31_000_000)),
            port(2, Some(0), Some(4_000)), // below the floor: noise
        ])]);

        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 1, "only the port that carried it");
        assert_eq!(changes[0].kind, ChangeKind::Traffic);
        assert!(changes[0].subject.contains("port 1"));
        assert!(changes[0].note.contains("29.6 MB"));
    }

    #[test]
    fn errors_outrank_traffic_however_much_larger_the_traffic_number_is() {
        let before = snapshot(vec![switch(vec![
            port(1, Some(0), Some(0)),
            port(7, Some(0), None),
        ])]);
        let after = snapshot(vec![switch(vec![
            port(1, Some(0), Some(900_000_000)),
            port(7, Some(12), None),
        ])]);

        let changes = diff(&before, &after);
        assert_eq!(changes[0].kind, ChangeKind::Errors, "12 errors beat 900 MB");
    }

    #[test]
    fn a_saturated_radio_is_reported_even_when_it_barely_moved() {
        // Airtime already spent before the reproduction started is still the
        // cause; a small delta must not hide it.
        let mut radio = RadioStats {
            name: Some("wifi0".into()),
            radio: Some("ng".into()),
            channel: Some(6),
            cu_total: Some(88.0),
            cu_self_rx: None,
            cu_self_tx: None,
            num_sta: None,
            satisfaction: None,
        };
        let mut ap = switch(vec![]);
        ap.port_table = None;
        ap.radio_table_stats = Some(vec![radio.clone()]);
        let before = snapshot(vec![ap.clone()]);

        radio.cu_total = Some(89.0);
        ap.radio_table_stats = Some(vec![radio]);
        let after = snapshot(vec![ap]);

        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Airtime);
        assert!(changes[0].note.contains("airtime is nearly spent"));
        assert!(changes[0].subject.contains("channel 6"));
    }

    #[test]
    fn a_gateway_pinning_its_cpu_is_reported() {
        let mut gateway = switch(vec![]);
        gateway.port_table = None;
        gateway.name = Some("UDM-Pro".into());
        gateway.system_stats = Some(SystemStats {
            cpu: Some(11.0),
            mem: None,
        });
        let before = snapshot(vec![gateway.clone()]);

        gateway.system_stats = Some(SystemStats {
            cpu: Some(97.0),
            mem: None,
        });
        let after = snapshot(vec![gateway]);

        let changes = diff(&before, &after);
        assert_eq!(changes[0].kind, ChangeKind::Load);
        assert_eq!(changes[0].delta, 86.0);
        assert!(changes[0].note.contains("processing limit"));
    }

    #[test]
    fn a_device_that_appeared_between_readings_is_skipped_rather_than_guessed_at() {
        // With no earlier reading there is no delta, and inventing one from
        // zero would report a lifetime counter as if it happened just now.
        let before = snapshot(vec![]);
        let after = snapshot(vec![switch(vec![port(1, Some(50_000), None)])]);
        assert!(diff(&before, &after).is_empty());
    }
}
