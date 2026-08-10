//! Separate scan histories per physical network.
//!
//! Without this, scanning at two sites writes into one history and the diff
//! becomes nonsense: every device at site A reads as "disappeared" and every
//! device at site B as "new". First-seen tracking is corrupted the same way.
//!
//! # Identifying a network
//!
//! The subnet cannot do it. Two different buildings routinely both use
//! `192.168.0.0/24`, and treating them as one network is exactly the bug this
//! module exists to prevent.
//!
//! The **gateway's MAC address** can: it is a specific piece of hardware, it is
//! observable without any configuration, and it survives the router's IP or SSID
//! changing. It is therefore the primary key, with SSID, subnet and resolvers as
//! weaker corroborating signals for the cases where no gateway MAC is available.

use crate::netutil::normalize_mac;
use crate::types::HostInfo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Observable facts that together identify a physical network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkFingerprint {
    /// The gateway's hardware address — the strongest signal by far.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_mac: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_ip: Option<String>,
    /// Local subnets in canonical form, e.g. `192.168.0.0/24`.
    pub subnets: Vec<String>,
    /// SSID when on Wi-Fi. Distinguishes two sites that share a subnet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssid: Option<String>,
    pub dns_servers: Vec<String>,
}

impl NetworkFingerprint {
    pub fn from_host(host: &HostInfo, gateway_mac: Option<String>, ssid: Option<String>) -> Self {
        let mut subnets: Vec<String> = host
            .scan_targets
            .iter()
            .filter(|t| t.source == crate::types::TargetSource::Local)
            .map(|t| t.cidr.clone())
            .collect();
        subnets.sort();
        subnets.dedup();

        let mut dns_servers = host.dns.servers.clone();
        dns_servers.sort();
        dns_servers.dedup();

        Self {
            gateway_mac: gateway_mac.map(|mac| normalize_mac(&mac)),
            gateway_ip: host.gateway.as_ref().map(|g| g.ip.clone()),
            subnets,
            ssid: ssid.filter(|s| !s.is_empty() && s != "(hidden)"),
            dns_servers,
        }
    }

    /// A short human description, used when naming a newly-detected network.
    pub fn describe(&self) -> String {
        if let Some(ssid) = &self.ssid {
            return ssid.clone();
        }
        if let Some(subnet) = self.subnets.first() {
            return subnet.clone();
        }
        "Unnamed network".to_string()
    }
}

/// How confident a match is.
///
/// Ordered so the best candidate is simply the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchStrength {
    None,
    /// Subnet alone. Deliberately *not* enough to switch on — this is precisely
    /// the case where two different sites look identical.
    Weak,
    /// SSID plus subnet, or matching resolvers. Good enough to act on.
    Strong,
    /// Gateway MAC. A specific piece of hardware; treat as certain.
    Definitive,
}

/// Compares an observed fingerprint against a stored one.
pub fn match_strength(stored: &NetworkFingerprint, observed: &NetworkFingerprint) -> MatchStrength {
    if let (Some(a), Some(b)) = (&stored.gateway_mac, &observed.gateway_mac) {
        if a.eq_ignore_ascii_case(b) {
            return MatchStrength::Definitive;
        }
        // Both known and different: definitively a *different* network, whatever
        // else agrees. This is what stops two sites sharing 192.168.0.0/24 from
        // being merged.
        return MatchStrength::None;
    }

    let subnet_overlap = stored
        .subnets
        .iter()
        .any(|subnet| observed.subnets.contains(subnet));

    let ssid_match = match (&stored.ssid, &observed.ssid) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };

    let dns_overlap = stored
        .dns_servers
        .iter()
        .any(|server| observed.dns_servers.contains(server));

    if ssid_match && subnet_overlap {
        return MatchStrength::Strong;
    }
    if ssid_match {
        // Same SSID on a different subnet — roaming within one site, or a
        // router that re-addressed. Still the same place.
        return MatchStrength::Strong;
    }
    if subnet_overlap && dns_overlap {
        return MatchStrength::Strong;
    }
    if subnet_overlap {
        return MatchStrength::Weak;
    }

    MatchStrength::None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProfile {
    pub id: String,
    pub name: String,
    pub fingerprint: NetworkFingerprint,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub scan_count: usize,
    /// The place this network belongs to, if any.
    ///
    /// An id rather than the name, so renaming the location does not have to
    /// touch every network in it — and so two spellings of one place cannot
    /// quietly become two groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,
}

impl NetworkProfile {
    pub fn new(name: impl Into<String>, fingerprint: NetworkFingerprint) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            fingerprint,
            created_at: chrono::Utc::now().to_rfc3339(),
            last_seen_at: None,
            scan_count: 0,
            location_id: None,
        }
    }
}

/// Time-ordered, filesystem-safe id.
///
/// Not a UUID: this only has to be unique within one installation, and a
/// sortable id makes the directory listing meaningful.
///
/// The counter is not decoration. Deriving the suffix from the clock alone
/// collides on platforms whose sub-second resolution is coarse — two profiles
/// created in the same millisecond got identical ids on macOS. Two networks
/// sharing an id share a directory, which merges their histories: precisely the
/// failure this module exists to prevent.
///
/// Locations share the generator — and therefore the one counter — so the same
/// reasoning covers both. Only the prefix differs, which makes a stale id
/// obvious to anyone reading the file.
fn timestamped_id(prefix: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    let now = chrono::Utc::now();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    format!(
        "{prefix}-{}-{:05}-{:04x}",
        now.format("%Y%m%d-%H%M%S"),
        nanos % 100_000,
        sequence & 0xffff
    )
}

fn new_id() -> String {
    timestamped_id("net")
}

fn new_location_id() -> String {
    timestamped_id("loc")
}

/// What the app should do about the network it is currently attached to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Detection {
    /// Matches the network already selected. Nothing to do.
    #[serde(rename_all = "camelCase")]
    Current { id: String, strength: MatchStrength },
    /// Matches a different saved network — offer to switch.
    #[serde(rename_all = "camelCase")]
    Switch {
        id: String,
        name: String,
        strength: MatchStrength,
    },
    /// Several saved networks look plausible and none is conclusive.
    #[serde(rename_all = "camelCase")]
    Ambiguous { candidates: Vec<NetworkCandidate> },
    /// Nothing matches — offer to create one.
    #[serde(rename_all = "camelCase")]
    Unknown { suggested_name: String },
    /// No usable network to fingerprint.
    #[serde(rename_all = "camelCase")]
    NoNetwork,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkCandidate {
    pub id: String,
    pub name: String,
    pub strength: MatchStrength,
}

/// A named place several networks belong to — a site, a building, a client.
///
/// One address often has more than one network: a main LAN, a guest SSID and a
/// lab VLAN are three separate histories that are nonetheless the same place.
/// This is a label for that, and deliberately nothing more: it never owns a
/// network, so removing it can never take a scan history with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub id: String,
    pub name: String,
}

impl Location {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_location_id(),
            name: name.into(),
        }
    }
}

/// The saved networks and which one is selected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkIndex {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// Named places, in creation order. Sorting happens at display time so the
    /// file stays stable and diffable across renames.
    #[serde(default)]
    pub locations: Vec<Location>,
    pub networks: Vec<NetworkProfile>,
}

impl NetworkIndex {
    pub fn index_path(root: &Path) -> PathBuf {
        root.join("networks.json")
    }

    /// Directory holding one network's data.
    pub fn network_dir(root: &Path, id: &str) -> PathBuf {
        root.join("networks").join(id)
    }

    pub fn scans_dir(root: &Path, id: &str) -> PathBuf {
        Self::network_dir(root, id).join("scans")
    }

    pub async fn load(root: &Path) -> Self {
        let mut index: Self = match tokio::fs::read(Self::index_path(root)).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        // In memory only — a read path does not write. The repair reaches disk
        // on whatever save comes next, and until then every caller at least
        // agrees about what the index contains.
        index.heal();
        index
    }

    pub async fn save(&self, root: &Path) -> Result<(), String> {
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        tokio::fs::write(Self::index_path(root), json)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&NetworkProfile> {
        self.networks.iter().find(|n| n.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut NetworkProfile> {
        self.networks.iter_mut().find(|n| n.id == id)
    }

    /// The selected network, falling back to the first if the id is stale.
    pub fn active_profile(&self) -> Option<&NetworkProfile> {
        self.active
            .as_ref()
            .and_then(|id| self.get(id))
            .or_else(|| self.networks.first())
    }

    /// Adds a network and selects it.
    ///
    /// Uniqueness is enforced here as well as in the generator. A counter resets
    /// when the process does, so two ids minted in the same second across a
    /// restart could still coincide — and a duplicate id would silently merge
    /// two networks' histories.
    pub fn add(&mut self, mut profile: NetworkProfile) -> String {
        if self.get(&profile.id).is_some() {
            let mut suffix = 1u32;
            let base = profile.id.clone();
            while self.get(&profile.id).is_some() {
                profile.id = format!("{base}-{suffix}");
                suffix += 1;
            }
        }

        let id = profile.id.clone();
        self.networks.push(profile);
        self.active = Some(id.clone());
        id
    }

    /* --------------------------------------------------------------- locations */

    pub fn location(&self, id: &str) -> Option<&Location> {
        self.locations.iter().find(|location| location.id == id)
    }

    /// Looks a location up by what the user typed — trimmed, and ignoring case.
    pub fn location_by_name(&self, name: &str) -> Option<&Location> {
        let name = name.trim();
        self.locations
            .iter()
            .find(|location| location.name.trim().eq_ignore_ascii_case(name))
    }

    /// Creates a location, or returns the id of the one that already has this
    /// name.
    ///
    /// Deliberately idempotent. The whole reason a location is a record rather
    /// than a free-text label is that "Office" and "office" must not become two
    /// groups, and creation is the path a typo takes.
    pub fn add_location(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A location needs a name".into());
        }

        if let Some(existing) = self.location_by_name(name) {
            return Ok(existing.id.clone());
        }

        let location = Location::new(name);
        let id = location.id.clone();
        self.locations.push(location);
        Ok(id)
    }

    /// Refused when another location already has the name.
    ///
    /// Merging the two would be the obvious alternative, and it is not what
    /// "rename" means — nor is it undoable once the networks have moved.
    pub fn rename_location(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A location needs a name".into());
        }

        if let Some(clash) = self.location_by_name(name) {
            if clash.id != id {
                return Err(format!(
                    "A location called \"{}\" already exists",
                    clash.name
                ));
            }
        }

        let location = self
            .locations
            .iter_mut()
            .find(|location| location.id == id)
            .ok_or("No such location")?;
        location.name = name.to_string();
        Ok(())
    }

    /// Removes the location and detaches the networks in it. Returns how many
    /// were detached, so the user can be told before it happens.
    ///
    /// The networks and their scan histories are untouched: a grouping label
    /// must never be able to delete the thing it labels.
    pub fn delete_location(&mut self, id: &str) -> Result<usize, String> {
        if self.location(id).is_none() {
            return Err("No such location".into());
        }

        self.locations.retain(|location| location.id != id);

        let mut detached = 0;
        for profile in &mut self.networks {
            if profile.location_id.as_deref() == Some(id) {
                profile.location_id = None;
                detached += 1;
            }
        }
        Ok(detached)
    }

    /// Puts a network in a location, or takes it out of one with `None`.
    ///
    /// An unknown location id is refused rather than stored: a network filed
    /// under a group nothing renders would simply disappear from the list.
    pub fn assign_location(
        &mut self,
        network_id: &str,
        location_id: Option<&str>,
    ) -> Result<(), String> {
        if let Some(location_id) = location_id {
            if self.location(location_id).is_none() {
                return Err("No such location".into());
            }
        }

        let profile = self.get_mut(network_id).ok_or("No such network")?;
        profile.location_id = location_id.map(|id| id.to_string());
        Ok(())
    }

    /// Display order: by name, ignoring case, with the id as a tie-break so it
    /// is deterministic. Derived once here so no two views can disagree.
    pub fn locations_sorted(&self) -> Vec<&Location> {
        let mut sorted: Vec<&Location> = self.locations.iter().collect();
        sorted.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        sorted
    }

    /// Clears location ids that point at nothing.
    fn heal(&mut self) {
        for profile in &mut self.networks {
            let dangling = profile
                .location_id
                .as_deref()
                .is_some_and(|id| !self.locations.iter().any(|location| location.id == id));
            if dangling {
                profile.location_id = None;
            }
        }
    }

    /// Decides what to do about an observed fingerprint.
    pub fn detect(&self, observed: &NetworkFingerprint) -> Detection {
        if observed.subnets.is_empty() && observed.gateway_mac.is_none() {
            return Detection::NoNetwork;
        }

        let mut scored: Vec<(MatchStrength, &NetworkProfile)> = self
            .networks
            .iter()
            .map(|profile| (match_strength(&profile.fingerprint, observed), profile))
            .filter(|(strength, _)| *strength != MatchStrength::None)
            .collect();

        scored.sort_by_key(|(strength, _)| std::cmp::Reverse(*strength));

        let Some((best_strength, best)) = scored.first().copied() else {
            return Detection::Unknown {
                suggested_name: observed.describe(),
            };
        };

        // A weak (subnet-only) match is not enough to act on — it is the very
        // ambiguity this module exists to surface rather than guess at.
        if best_strength == MatchStrength::Weak {
            return Detection::Ambiguous {
                candidates: scored
                    .iter()
                    .map(|(strength, profile)| NetworkCandidate {
                        id: profile.id.clone(),
                        name: profile.name.clone(),
                        strength: *strength,
                    })
                    .collect(),
            };
        }

        if self.active.as_deref() == Some(best.id.as_str()) {
            return Detection::Current {
                id: best.id.clone(),
                strength: best_strength,
            };
        }

        Detection::Switch {
            id: best.id.clone(),
            name: best.name.clone(),
            strength: best_strength,
        }
    }
}

/* ------------------------------------------------------- probing and migration */

/// Fingerprints the network this machine is on right now.
///
/// Deliberately cheap — this runs at launch and before every scan, so it must
/// not feel like a scan. The one non-trivial step is a single ping to the
/// gateway, which is what puts its MAC in the neighbour cache; without that the
/// strongest identifying signal is frequently unavailable on a cold start.
pub async fn probe_current() -> (HostInfo, NetworkFingerprint) {
    let (host, _warnings) = crate::scan::hostinfo::collect(&[]).await;

    let gateway_mac = match &host.gateway {
        Some(gateway) => resolve_gateway_mac(&gateway.ip).await,
        None => None,
    };

    // Best-effort: a machine with no Wi-Fi, or macOS without Location Services
    // permission, simply contributes no SSID rather than delaying startup.
    let ssid = tokio::time::timeout(std::time::Duration::from_secs(6), async {
        crate::platform::wifi_survey()
            .await
            .data
            .and_then(|wifi| wifi.current.map(|current| current.ssid))
    })
    .await
    .ok()
    .flatten();

    let fingerprint = NetworkFingerprint::from_host(&host, gateway_mac, ssid);
    (host, fingerprint)
}

async fn resolve_gateway_mac(gateway_ip: &str) -> Option<String> {
    let Ok(ip) = gateway_ip.parse::<std::net::Ipv4Addr>() else {
        return None;
    };

    // Check the cache first; only pay for a ping if the entry is missing.
    if let Some(mac) = crate::scan::sweep::read_neighbors().await.get(&ip) {
        return Some(mac.clone());
    }

    let args = crate::platform::ping_args(gateway_ip, 1, 1);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let _ = crate::exec::run("ping", &arg_refs, std::time::Duration::from_millis(2000)).await;
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    crate::scan::sweep::read_neighbors().await.get(&ip).cloned()
}

/// Moves a pre-networks installation into its first network.
///
/// Existing users have scans in a flat `scans/` directory. Those represent a
/// real network and must not be discarded, so they become the first profile,
/// fingerprinted from the newest snapshot they contain rather than from wherever
/// the machine happens to be when the upgrade is first run.
pub async fn migrate_flat_layout(root: &Path) -> Result<Option<String>, String> {
    let legacy = root.join("scans");
    if !legacy.is_dir() {
        return Ok(None);
    }

    // Already migrated, or a fresh install that happens to have the directory.
    let index = NetworkIndex::load(root).await;
    if !index.networks.is_empty() {
        return Ok(None);
    }

    let legacy_store = crate::store::Store::new(&legacy);
    let ids = legacy_store.list_ids().await;
    if ids.is_empty() {
        return Ok(None);
    }

    let newest = legacy_store.load_latest().await;
    let (name, fingerprint) = match &newest {
        Some(snapshot) => {
            let gateway_mac = snapshot
                .devices
                .iter()
                .find(|device| device.is_gateway)
                .and_then(|device| device.mac.clone());
            let ssid = snapshot
                .wifi
                .data
                .as_ref()
                .and_then(|wifi| wifi.current.as_ref().map(|c| c.ssid.clone()));

            let fingerprint = NetworkFingerprint::from_host(&snapshot.host, gateway_mac, ssid);
            (fingerprint.describe(), fingerprint)
        }
        None => ("Default network".to_string(), NetworkFingerprint::default()),
    };

    let profile = NetworkProfile {
        scan_count: ids.len(),
        last_seen_at: newest.as_ref().map(|s| s.started_at.clone()),
        ..NetworkProfile::new(name, fingerprint)
    };
    let id = profile.id.clone();

    let target = NetworkIndex::scans_dir(root, &id);
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| e.to_string())?;

    for scan_id in &ids {
        let from = legacy.join(format!("{scan_id}.json"));
        let to = target.join(format!("{scan_id}.json"));
        // Rename is atomic within a filesystem; copying is the fallback for the
        // rare case the app-data directory spans mounts.
        if tokio::fs::rename(&from, &to).await.is_err() {
            tokio::fs::copy(&from, &to)
                .await
                .map_err(|e| format!("could not migrate {scan_id}: {e}"))?;
            let _ = tokio::fs::remove_file(&from).await;
        }
    }

    // Controller settings belonged to that same network.
    let legacy_unifi = root.join("unifi.json");
    if legacy_unifi.is_file() {
        let to = NetworkIndex::network_dir(root, &id).join("unifi.json");
        if tokio::fs::rename(&legacy_unifi, &to).await.is_err() {
            let _ = tokio::fs::copy(&legacy_unifi, &to).await;
            let _ = tokio::fs::remove_file(&legacy_unifi).await;
        }
    }

    let mut index = index;
    index.add(profile);
    index.save(root).await?;

    // Leave the empty directory behind rather than removing it: if anything
    // above went wrong, the user still has a directory to look in.
    Ok(Some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(
        gateway_mac: Option<&str>,
        subnet: &str,
        ssid: Option<&str>,
    ) -> NetworkFingerprint {
        NetworkFingerprint {
            gateway_mac: gateway_mac.map(|m| m.to_string()),
            gateway_ip: Some("192.168.0.1".into()),
            subnets: vec![subnet.to_string()],
            ssid: ssid.map(|s| s.to_string()),
            dns_servers: vec!["192.168.0.1".into()],
        }
    }

    #[test]
    fn the_same_subnet_at_two_places_is_not_the_same_network() {
        // The bug this module exists for: two sites both on 192.168.0.0/24.
        let home = fingerprint(
            Some("aa:aa:aa:aa:aa:aa"),
            "192.168.0.0/24",
            Some("HomeWiFi"),
        );
        let office = fingerprint(
            Some("bb:bb:bb:bb:bb:bb"),
            "192.168.0.0/24",
            Some("OfficeWiFi"),
        );

        assert_eq!(match_strength(&home, &office), MatchStrength::None);
    }

    #[test]
    fn a_differing_gateway_mac_overrides_everything_else_agreeing() {
        // Same subnet, same SSID name, same resolver — but different hardware.
        let a = fingerprint(Some("aa:aa:aa:aa:aa:aa"), "192.168.0.0/24", Some("linksys"));
        let b = fingerprint(Some("bb:bb:bb:bb:bb:bb"), "192.168.0.0/24", Some("linksys"));

        assert_eq!(
            match_strength(&a, &b),
            MatchStrength::None,
            "hardware identity must win over coincidental agreement"
        );
    }

    #[test]
    fn the_same_gateway_is_definitive_even_if_the_subnet_changed() {
        let before = fingerprint(Some("aa:aa:aa:aa:aa:aa"), "192.168.0.0/24", Some("Home"));
        let mut after = fingerprint(Some("AA:AA:AA:AA:AA:AA"), "10.0.0.0/24", Some("Home"));
        after.dns_servers = vec!["10.0.0.1".into()];

        assert_eq!(
            match_strength(&before, &after),
            MatchStrength::Definitive,
            "re-addressing the LAN does not make it a different place"
        );
    }

    #[test]
    fn ssid_identifies_a_site_when_no_gateway_mac_is_available() {
        let stored = fingerprint(None, "192.168.0.0/24", Some("Office"));
        let observed = fingerprint(None, "192.168.5.0/24", Some("Office"));
        assert_eq!(match_strength(&stored, &observed), MatchStrength::Strong);
    }

    #[test]
    fn subnet_alone_is_only_a_weak_match() {
        let stored = fingerprint(None, "192.168.0.0/24", None);
        let mut observed = fingerprint(None, "192.168.0.0/24", None);
        observed.dns_servers = vec!["1.1.1.1".into()];

        assert_eq!(
            match_strength(&stored, &observed),
            MatchStrength::Weak,
            "a shared subnet is exactly the ambiguous case, not a match"
        );
    }

    #[test]
    fn an_unrecognised_network_is_reported_as_unknown() {
        let index = NetworkIndex::default();
        let observed = fingerprint(Some("cc:cc:cc:cc:cc:cc"), "172.16.0.0/24", Some("Cafe"));

        match index.detect(&observed) {
            Detection::Unknown { suggested_name } => assert_eq!(suggested_name, "Cafe"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_definitive_match_on_another_network_offers_a_switch() {
        let mut index = NetworkIndex::default();
        let home = NetworkProfile::new(
            "Home",
            fingerprint(Some("aa:aa:aa:aa:aa:aa"), "192.168.0.0/24", Some("Home")),
        );
        let office = NetworkProfile::new(
            "Office",
            fingerprint(Some("bb:bb:bb:bb:bb:bb"), "192.168.0.0/24", Some("Office")),
        );
        index.networks.push(home);
        let office_id = office.id.clone();
        index.networks.push(office);
        index.active = Some(index.networks[0].id.clone());

        let observed = fingerprint(Some("bb:bb:bb:bb:bb:bb"), "192.168.0.0/24", Some("Office"));
        match index.detect(&observed) {
            Detection::Switch { id, name, strength } => {
                assert_eq!(id, office_id);
                assert_eq!(name, "Office");
                assert_eq!(strength, MatchStrength::Definitive);
            }
            other => panic!("expected Switch, got {other:?}"),
        }
    }

    #[test]
    fn staying_on_the_same_network_is_a_no_op() {
        let mut index = NetworkIndex::default();
        let profile = NetworkProfile::new(
            "Home",
            fingerprint(Some("aa:aa:aa:aa:aa:aa"), "192.168.0.0/24", Some("Home")),
        );
        let id = profile.id.clone();
        index.networks.push(profile);
        index.active = Some(id.clone());

        let observed = fingerprint(Some("aa:aa:aa:aa:aa:aa"), "192.168.0.0/24", Some("Home"));
        match index.detect(&observed) {
            Detection::Current { id: found, .. } => assert_eq!(found, id),
            other => panic!("expected Current, got {other:?}"),
        }
    }

    #[test]
    fn two_gateway_less_networks_sharing_a_subnet_are_reported_ambiguous() {
        // Never guess here — guessing is how histories get merged.
        let mut index = NetworkIndex::default();
        index.networks.push(NetworkProfile::new(
            "Site A",
            fingerprint(None, "192.168.0.0/24", None),
        ));
        index.networks.push(NetworkProfile::new(
            "Site B",
            fingerprint(None, "192.168.0.0/24", None),
        ));

        let mut observed = fingerprint(None, "192.168.0.0/24", None);
        observed.dns_servers = vec!["8.8.8.8".into()];

        match index.detect(&observed) {
            Detection::Ambiguous { candidates } => assert_eq!(candidates.len(), 2),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn no_addresses_at_all_reports_no_network() {
        let index = NetworkIndex::default();
        assert!(matches!(
            index.detect(&NetworkFingerprint::default()),
            Detection::NoNetwork
        ));
    }

    #[test]
    fn ids_are_unique_even_when_minted_in_the_same_instant() {
        // A clock-only suffix collided here on macOS, and two networks sharing
        // an id share a directory — merging exactly the histories this module
        // keeps apart.
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_id()).collect();
        assert_eq!(
            ids.len(),
            1000,
            "id generation must not collide under speed"
        );

        for id in &ids {
            assert!(
                id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "ids become directory names: {id}"
            );
        }
    }

    #[test]
    fn adding_a_duplicate_id_disambiguates_rather_than_merging() {
        let mut index = NetworkIndex::default();
        let first = NetworkProfile::new("A", NetworkFingerprint::default());

        // Force the collision the generator is supposed to make impossible.
        let mut second = NetworkProfile::new("B", NetworkFingerprint::default());
        second.id = first.id.clone();

        let first_id = index.add(first);
        let second_id = index.add(second);

        assert_ne!(
            first_id, second_id,
            "two networks must never share a directory"
        );
        assert_eq!(index.networks.len(), 2);
    }

    /* ------------------------------------------------------------- locations */

    fn with_networks(names: &[&str]) -> NetworkIndex {
        let mut index = NetworkIndex::default();
        for name in names {
            index
                .networks
                .push(NetworkProfile::new(*name, NetworkFingerprint::default()));
        }
        index
    }

    #[test]
    fn creating_a_location_that_differs_only_in_case_reuses_the_existing_one() {
        // The guarantee that makes a location a record rather than a label: one
        // place cannot become two groups because of how it was typed.
        let mut index = NetworkIndex::default();
        let first = index.add_location("Head office").unwrap();
        let second = index.add_location("  head OFFICE ").unwrap();

        assert_eq!(first, second);
        assert_eq!(index.locations.len(), 1);
        assert_eq!(
            index.locations[0].name, "Head office",
            "the first spelling wins"
        );
    }

    #[test]
    fn a_blank_location_name_is_refused() {
        let mut index = NetworkIndex::default();
        assert!(index.add_location("   ").is_err());
        assert!(index.locations.is_empty());

        let id = index.add_location("Home").unwrap();
        assert!(index.rename_location(&id, "\t").is_err());
        assert_eq!(index.locations[0].name, "Home");
    }

    #[test]
    fn renaming_a_location_to_a_name_already_taken_is_refused() {
        let mut index = NetworkIndex::default();
        let home = index.add_location("Home").unwrap();
        index.add_location("Office").unwrap();

        // Merging is the tempting alternative, and it cannot be undone.
        assert!(index.rename_location(&home, "office").is_err());
        assert_eq!(index.locations.len(), 2);
        assert_eq!(index.location(&home).unwrap().name, "Home");
    }

    #[test]
    fn renaming_a_location_leaves_its_networks_alone() {
        let mut index = with_networks(&["LAN", "Guest"]);
        let id = index.add_location("Office").unwrap();
        let lan = index.networks[0].id.clone();
        index.assign_location(&lan, Some(&id)).unwrap();

        index.rename_location(&id, "Head office").unwrap();

        // Nothing stores the name, so one edit renames the group everywhere.
        assert_eq!(
            index.get(&lan).unwrap().location_id.as_deref(),
            Some(id.as_str())
        );
        assert_eq!(index.location(&id).unwrap().name, "Head office");
    }

    #[test]
    fn deleting_a_location_keeps_the_networks_in_it() {
        let mut index = with_networks(&["LAN", "Guest", "Elsewhere"]);
        let id = index.add_location("Office").unwrap();
        let ids: Vec<String> = index
            .networks
            .iter()
            .take(2)
            .map(|n| n.id.clone())
            .collect();
        for network in &ids {
            index.assign_location(network, Some(&id)).unwrap();
        }

        assert_eq!(index.delete_location(&id).unwrap(), 2);

        assert_eq!(
            index.networks.len(),
            3,
            "a label must not delete what it labels"
        );
        assert!(index.networks.iter().all(|n| n.location_id.is_none()));
        assert!(index.location(&id).is_none());
    }

    #[test]
    fn assigning_an_unknown_location_is_refused() {
        let mut index = with_networks(&["LAN"]);
        let lan = index.networks[0].id.clone();

        assert!(index.assign_location(&lan, Some("loc-nope")).is_err());
        assert!(
            index.get(&lan).unwrap().location_id.is_none(),
            "a network filed under a group nothing renders would vanish from the list"
        );
    }

    #[test]
    fn a_location_id_pointing_at_nothing_is_cleared_on_load() {
        let mut index = with_networks(&["LAN"]);
        index.networks[0].location_id = Some("loc-deleted".into());

        index.heal();

        assert!(index.networks[0].location_id.is_none());
    }

    #[test]
    fn location_ids_do_not_collide_when_minted_in_the_same_instant() {
        let locations: std::collections::HashSet<String> =
            (0..1000).map(|_| new_location_id()).collect();
        assert_eq!(locations.len(), 1000);

        // The shared counter is what makes one generator safe for both kinds;
        // the prefix is what keeps a misplaced id obvious.
        let networks: std::collections::HashSet<String> = (0..1000).map(|_| new_id()).collect();
        assert!(locations.is_disjoint(&networks));
    }

    #[test]
    fn an_index_written_before_locations_existed_still_loads() {
        // Frozen on purpose: a round-trip through the current structs cannot
        // catch a field that stopped being optional.
        let legacy = r#"{
            "active": "net-20240101-000000-00000-0000",
            "networks": [
                {
                    "id": "net-20240101-000000-00000-0000",
                    "name": "Home",
                    "fingerprint": { "subnets": ["192.168.0.0/24"], "dnsServers": [] },
                    "createdAt": "2024-01-01T00:00:00Z",
                    "scanCount": 7
                }
            ]
        }"#;

        let index: NetworkIndex = serde_json::from_str(legacy).unwrap();
        assert!(index.locations.is_empty());
        assert_eq!(index.networks.len(), 1);
        assert_eq!(index.networks[0].scan_count, 7);
        assert!(index.networks[0].location_id.is_none());
    }

    #[test]
    fn a_network_with_no_location_writes_no_location_id_key() {
        let mut profile = NetworkProfile::new("Home", NetworkFingerprint::default());
        let json = serde_json::to_string(&profile).unwrap();
        assert!(!json.contains("locationId"));

        profile.location_id = Some("loc-1".into());
        let json = serde_json::to_string(&profile).unwrap();
        // The TypeScript type depends on this exact spelling.
        assert!(json.contains("\"locationId\":\"loc-1\""));
    }

    #[test]
    fn locations_are_listed_by_name_regardless_of_how_they_were_typed() {
        let mut index = NetworkIndex::default();
        index.add_location("office").unwrap();
        index.add_location("Attic").unwrap();
        index.add_location("Home").unwrap();

        let names: Vec<&str> = index
            .locations_sorted()
            .iter()
            .map(|location| location.name.as_str())
            .collect();
        assert_eq!(names, vec!["Attic", "Home", "office"]);
    }

    #[test]
    fn the_active_profile_falls_back_when_the_id_is_stale() {
        let mut index = NetworkIndex::default();
        index
            .networks
            .push(NetworkProfile::new("Home", NetworkFingerprint::default()));
        index.active = Some("net-does-not-exist".into());

        assert_eq!(
            index.active_profile().map(|p| p.name.as_str()),
            Some("Home"),
            "a dangling id must not leave the app with no network"
        );
    }
}
