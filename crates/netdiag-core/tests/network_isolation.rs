//! Two sites must never share a scan history.
//!
//! Reproduces the reported failure end-to-end: the *same* subnet at two
//! different places. Before per-network storage, scanning at the second site
//! wrote into the first site's history, and the diff then reported every device
//! at site A as "disappeared" and every device at site B as "new".

use netdiag_core::networks::{
    Detection, MatchStrength, NetworkFingerprint, NetworkIndex, NetworkProfile,
};
use netdiag_core::store::Store;
use netdiag_core::types::*;

fn temp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "netdiag-isolation-{tag}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Both sites use 192.168.0.0/24; only the gateway hardware differs.
fn site(gateway_mac: &str, ssid: &str) -> NetworkFingerprint {
    NetworkFingerprint {
        gateway_mac: Some(gateway_mac.to_string()),
        gateway_ip: Some("192.168.0.1".into()),
        subnets: vec!["192.168.0.0/24".into()],
        ssid: Some(ssid.to_string()),
        dns_servers: vec!["192.168.0.1".into()],
    }
}

fn snapshot(id: &str, device_ips: &[&str]) -> ScanSnapshot {
    ScanSnapshot {
        id: id.into(),
        started_at: format!("2026-08-04T10:00:0{}Z", id.len() % 10),
        finished_at: "2026-08-04T10:01:00Z".into(),
        duration_ms: 1000,
        host: HostInfo {
            hostname: "test".into(),
            platform: "test".into(),
            os: "Linux".into(),
            arch: "x86_64".into(),
            app_version: "1.0.0".into(),
            interfaces: Vec::new(),
            routes: Vec::new(),
            gateway: None,
            dns: DnsConfig::default(),
            scan_targets: Vec::new(),
        },
        devices: device_ips
            .iter()
            .map(|ip| Device {
                ip: (*ip).into(),
                mac: Some(format!("aa:bb:cc:00:00:{:02x}", ip.len())),
                vendor: None,
                mac_randomized: Some(false),
                hostnames: Vec::new(),
                display_name: (*ip).into(),
                device_type: DeviceType::Unknown,
                type_evidence: Vec::new(),
                is_gateway: false,
                is_self: false,
                responded_to_ping: true,
                discovered_by: vec!["icmp".into()],
                latency_ms: None,
                ports: Vec::new(),
                mdns: Vec::new(),
                ssdp: Vec::new(),
                netbios: None,
                reverse_dns: None,
                source_range: None,
                off_subnet: false,
                first_seen: None,
                last_seen: "t".into(),
                unifi_name: None,
                unifi_fingerprint: None,
                unifi_network: None,
                switch_port: None,
                access_point: None,
                vlan: None,
                rssi: None,
                is_wired: None,
                satisfaction: None,
                channel: None,
                wifi_generation: None,
                tx_bytes: None,
                rx_bytes: None,
                unifi_uptime: None,
                unifi_first_seen: None,
                is_guest: None,
                unifi_note: None,
            })
            .collect(),
        connectivity: ConnectivityInfo {
            gateway: None,
            wan: Vec::new(),
            dns: Vec::new(),
            public_ip: None,
            wan_reachable: true,
            trace: ProbeResult::unavailable("test"),
        },
        wifi: ProbeResult::unavailable("test"),
        phases: Vec::new(),
        warnings: Vec::new(),
        config: ScanConfig::default(),
        capabilities: Vec::new(),
        off_scope: Vec::new(),
        baseline: false,
        unifi: None,
        reconciliation: None,
    }
}

#[tokio::test]
async fn the_same_subnet_at_two_sites_keeps_two_separate_histories() {
    let root = temp_root("two-sites");

    let mut index = NetworkIndex::default();
    let home = NetworkProfile::new("Home", site("aa:aa:aa:aa:aa:aa", "HomeWiFi"));
    let office = NetworkProfile::new("Office", site("bb:bb:bb:bb:bb:bb", "OfficeWiFi"));
    let (home_id, office_id) = (home.id.clone(), office.id.clone());
    index.networks.push(home);
    index.networks.push(office);
    index.save(&root).await.unwrap();

    // Scan at home, then at the office — the sequence that used to corrupt.
    let home_store = Store::new(NetworkIndex::scans_dir(&root, &home_id));
    home_store
        .save(&snapshot(
            "2026-08-04T10-00-00-000Z",
            &["192.168.0.10", "192.168.0.11"],
        ))
        .await
        .unwrap();

    let office_store = Store::new(NetworkIndex::scans_dir(&root, &office_id));
    office_store
        .save(&snapshot("2026-08-04T11-00-00-000Z", &["192.168.0.50"]))
        .await
        .unwrap();

    // Each history contains only its own run.
    assert_eq!(home_store.list_ids().await.len(), 1);
    assert_eq!(office_store.list_ids().await.len(), 1);

    let home_latest = home_store.load_latest().await.unwrap();
    let office_latest = office_store.load_latest().await.unwrap();

    assert_eq!(
        home_latest.devices.len(),
        2,
        "home must keep its own devices"
    );
    assert_eq!(
        office_latest.devices.len(),
        1,
        "office must keep its own devices"
    );
    assert!(home_latest.devices.iter().all(|d| d.ip != "192.168.0.50"));

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn moving_between_sites_does_not_produce_a_bogus_diff() {
    let root = temp_root("no-bogus-diff");

    let mut index = NetworkIndex::default();
    let home = NetworkProfile::new("Home", site("aa:aa:aa:aa:aa:aa", "HomeWiFi"));
    let home_id = home.id.clone();
    index.networks.push(home);
    index.networks.push(NetworkProfile::new(
        "Office",
        site("bb:bb:bb:bb:bb:bb", "OfficeWiFi"),
    ));
    index.save(&root).await.unwrap();

    let store = Store::new(NetworkIndex::scans_dir(&root, &home_id));

    // Two runs at the *same* site, with the office visit in between going to its
    // own store. The home diff must not notice the trip at all.
    let first = snapshot(
        "2026-08-04T10-00-00-000Z",
        &["192.168.0.10", "192.168.0.11"],
    );
    store.save(&first).await.unwrap();

    let office_id = index.networks[1].id.clone();
    Store::new(NetworkIndex::scans_dir(&root, &office_id))
        .save(&snapshot("2026-08-04T11-00-00-000Z", &["192.168.0.50"]))
        .await
        .unwrap();

    let second = snapshot(
        "2026-08-04T12-00-00-000Z",
        &["192.168.0.10", "192.168.0.11"],
    );
    store.save(&second).await.unwrap();

    let diff = netdiag_core::store::diff(&first, &second);
    assert!(
        diff.changes.is_empty(),
        "an unrelated site visit must not appear as churn at home: {:?}",
        diff.changes.iter().map(|c| &c.detail).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn arriving_at_the_second_site_is_detected_as_a_switch_not_a_match() {
    let mut index = NetworkIndex::default();
    let home = NetworkProfile::new("Home", site("aa:aa:aa:aa:aa:aa", "HomeWiFi"));
    let home_id = home.id.clone();
    index.networks.push(home);
    index.networks.push(NetworkProfile::new(
        "Office",
        site("bb:bb:bb:bb:bb:bb", "OfficeWiFi"),
    ));
    index.active = Some(home_id);

    // Now standing in the office: same subnet, different gateway.
    let observed = site("bb:bb:bb:bb:bb:bb", "OfficeWiFi");

    match index.detect(&observed) {
        Detection::Switch { name, strength, .. } => {
            assert_eq!(name, "Office");
            assert_eq!(strength, MatchStrength::Definitive);
        }
        other => panic!("the shared subnet must not read as the same network: {other:?}"),
    }
}

#[tokio::test]
async fn an_existing_flat_installation_is_migrated_without_losing_scans() {
    let root = temp_root("migration");
    let legacy = root.join("scans");
    std::fs::create_dir_all(&legacy).unwrap();

    let legacy_store = Store::new(&legacy);
    for id in ["2026-08-01T10-00-00-000Z", "2026-08-02T10-00-00-000Z"] {
        legacy_store
            .save(&snapshot(id, &["10.0.0.5"]))
            .await
            .unwrap();
    }

    let id = netdiag_core::networks::migrate_flat_layout(&root)
        .await
        .unwrap()
        .expect("a populated legacy directory should migrate");

    let migrated = Store::new(NetworkIndex::scans_dir(&root, &id));
    assert_eq!(migrated.list_ids().await.len(), 2, "no scan may be lost");

    let index = NetworkIndex::load(&root).await;
    assert_eq!(index.networks.len(), 1);
    assert_eq!(index.active.as_deref(), Some(id.as_str()));
    assert_eq!(index.networks[0].scan_count, 2);

    // Running again must be a no-op rather than creating a duplicate.
    assert!(netdiag_core::networks::migrate_flat_layout(&root)
        .await
        .unwrap()
        .is_none());
    assert_eq!(NetworkIndex::load(&root).await.networks.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_fresh_install_has_nothing_to_migrate() {
    let root = temp_root("fresh");
    assert!(netdiag_core::networks::migrate_flat_layout(&root)
        .await
        .unwrap()
        .is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn locations_survive_a_save_and_load_round_trip() {
    let root = temp_root("locations-round-trip");

    let mut index = NetworkIndex::default();
    index.networks.push(NetworkProfile::new(
        "LAN",
        site("aa:aa:aa:aa:aa:aa", "Office"),
    ));
    index.networks.push(NetworkProfile::new(
        "Guest",
        site("aa:aa:aa:aa:aa:ab", "Office guest"),
    ));
    index.networks.push(NetworkProfile::new(
        "Home",
        site("bb:bb:bb:bb:bb:bb", "HomeWiFi"),
    ));

    let office = index.add_location("Head office").unwrap();
    index.add_location("Attic").unwrap();
    let (lan, guest) = (index.networks[0].id.clone(), index.networks[1].id.clone());
    index.assign_location(&lan, Some(&office)).unwrap();
    index.assign_location(&guest, Some(&office)).unwrap();
    index.save(&root).await.unwrap();

    let loaded = NetworkIndex::load(&root).await;
    assert_eq!(loaded.locations.len(), 2);
    assert_eq!(
        loaded.get(&lan).unwrap().location_id.as_deref(),
        Some(office.as_str())
    );
    assert_eq!(
        loaded.get(&guest).unwrap().location_id.as_deref(),
        Some(office.as_str())
    );
    assert!(loaded.networks[2].location_id.is_none());

    // The one ordering rule, applied to what actually came off disk.
    let names: Vec<&str> = loaded
        .locations_sorted()
        .iter()
        .map(|location| location.name.as_str())
        .collect();
    assert_eq!(names, vec!["Attic", "Head office"]);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_location_id_pointing_at_a_deleted_location_self_heals_on_load() {
    let root = temp_root("locations-heal");

    let mut index = NetworkIndex::default();
    index.networks.push(NetworkProfile::new(
        "LAN",
        site("aa:aa:aa:aa:aa:aa", "Office"),
    ));
    let office = index.add_location("Head office").unwrap();
    let lan = index.networks[0].id.clone();
    index.assign_location(&lan, Some(&office)).unwrap();
    index.save(&root).await.unwrap();

    // Drop the location behind the index's back — a half-completed write, or a
    // file edited by hand.
    index.locations.clear();
    index.save(&root).await.unwrap();

    let loaded = NetworkIndex::load(&root).await;
    assert_eq!(
        loaded.networks.len(),
        1,
        "a dangling location must not make a network disappear"
    );
    assert!(loaded.networks[0].location_id.is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn deleting_a_location_never_deletes_a_networks_history() {
    let root = temp_root("locations-delete");

    let mut index = NetworkIndex::default();
    index.networks.push(NetworkProfile::new(
        "LAN",
        site("aa:aa:aa:aa:aa:aa", "Office"),
    ));
    index.networks.push(NetworkProfile::new(
        "Guest",
        site("aa:aa:aa:aa:aa:ab", "Office guest"),
    ));
    let office = index.add_location("Head office").unwrap();
    let (lan, guest) = (index.networks[0].id.clone(), index.networks[1].id.clone());
    index.assign_location(&lan, Some(&office)).unwrap();
    index.assign_location(&guest, Some(&office)).unwrap();
    index.save(&root).await.unwrap();

    let lan_store = Store::new(NetworkIndex::scans_dir(&root, &lan));
    lan_store
        .save(&snapshot("2026-08-04T10-00-00-000Z", &["192.168.0.10"]))
        .await
        .unwrap();
    let guest_store = Store::new(NetworkIndex::scans_dir(&root, &guest));
    guest_store
        .save(&snapshot("2026-08-04T11-00-00-000Z", &["192.168.1.10"]))
        .await
        .unwrap();

    assert_eq!(index.delete_location(&office).unwrap(), 2);
    index.save(&root).await.unwrap();

    let loaded = NetworkIndex::load(&root).await;
    assert_eq!(loaded.networks.len(), 2, "a grouping label owns nothing");
    assert!(loaded.locations.is_empty());
    assert_eq!(lan_store.list_ids().await.len(), 1);
    assert_eq!(guest_store.list_ids().await.len(), 1);

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn an_index_written_before_locations_existed_loads_from_disk() {
    let root = temp_root("locations-upgrade");

    // The real upgrade risk is at the file level, so write the old shape.
    let legacy = r#"{
        "active": "net-20240101-000000-00000-0000",
        "networks": [
            {
                "id": "net-20240101-000000-00000-0000",
                "name": "Home",
                "fingerprint": { "subnets": ["192.168.0.0/24"], "dnsServers": [] },
                "createdAt": "2024-01-01T00:00:00Z",
                "scanCount": 3
            }
        ]
    }"#;
    std::fs::write(NetworkIndex::index_path(&root), legacy).unwrap();

    let loaded = NetworkIndex::load(&root).await;
    assert_eq!(loaded.networks.len(), 1);
    assert_eq!(loaded.networks[0].name, "Home");
    assert_eq!(loaded.networks[0].scan_count, 3);
    assert!(loaded.locations.is_empty());
    assert!(loaded.networks[0].location_id.is_none());

    let _ = std::fs::remove_dir_all(&root);
}
