//! Pseudonymising evidence on the way out, and restoring it on the way back.
//!
//! Sending a network's topology to a cloud model is a real disclosure, and the
//! honest response is not a warning label — it is to send less. What identifies
//! a household is its device names, its MACs, its Wi-Fi names and its public
//! address. What *diagnoses* a fault is vendors, models, port numbers, speeds,
//! duplex, signal levels, rates and error counts. Those two sets barely
//! overlap, so almost all of the identifying material can be replaced with
//! stable placeholders at no cost to the diagnosis:
//!
//! ```text
//!   aa:bb:cc:dd:ee:ff  ->  device-3          "Studio MacBook"  ->  host-7
//!   Ashgrove_5G       ->  wifi-1            203.0.113.9          ->  wan-ip-1
//! ```
//!
//! Placeholders are stable within a case, so the model can refer back to
//! `device-3` across turns and mean the same thing. The mapping never leaves
//! this machine: the transcript is stored with the real values and is
//! pseudonymised only at the moment it is handed to a provider, then restored
//! in what comes back — so the user always reads real names, and the model
//! never sees them.
//!
//! Two deliberate limits, because a privacy feature that overstates itself is
//! worse than none:
//!
//! - Private addresses pass through unchanged. `10.0.3.14` is meaningless
//!   outside the house that uses it, and the whole diagnosis is built on
//!   knowing which address is which.
//! - Only what has been *learned* is replaced. Vocabulary is taken from the
//!   scan and the controller; anything named nowhere in that data — a hostname
//!   the user types into their own question — is not caught. MACs and public
//!   addresses are the exception: those are recognised by shape wherever they
//!   appear.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use crate::netutil;
use crate::types::ScanSnapshot;
use crate::unifi::model::UnifiSnapshot;

/// What a placeholder stands for, which decides how it is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    /// A hardware address.
    Device,
    /// A machine or operator-assigned name.
    Host,
    /// A wireless network name.
    Wifi,
    /// A public address.
    WanIp,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Device => "device",
            Kind::Host => "host",
            Kind::Wifi => "wifi",
            Kind::WanIp => "wan-ip",
        }
    }
}

/// Public resolvers, which name a service rather than a person.
///
/// Replacing these would cost real diagnostic meaning — "your DNS is Google's"
/// is worth knowing — and conceals nothing, since millions of networks use
/// them. Every other public address is treated as the user's own.
const WELL_KNOWN_RESOLVERS: &[&str] = &[
    "8.8.8.8",
    "8.8.4.4",
    "1.1.1.1",
    "1.0.0.1",
    "9.9.9.9",
    "149.112.112.112",
    "208.67.222.222",
    "208.67.220.220",
    "76.76.2.0",
    "94.140.14.14",
];

/// A per-case substitution table.
pub struct Redactor {
    enabled: bool,
    /// Real value to placeholder, longest first so a name is replaced before
    /// any shorter name it contains.
    forward: Vec<(String, String)>,
    /// Placeholder to real value, for the return journey.
    back: HashMap<String, String>,
    counts: HashMap<Kind, usize>,
}

impl Redactor {
    /// A disabled redactor passes everything through untouched.
    ///
    /// Kept as a real object rather than an `Option` so the loop has one code
    /// path — a second path is where a "sometimes redacted" bug lives.
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            forward: Vec::new(),
            back: HashMap::new(),
            counts: HashMap::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// How many distinct identifiers are being hidden.
    pub fn len(&self) -> usize {
        self.back.len()
    }

    pub fn is_empty(&self) -> bool {
        self.back.is_empty()
    }

    /// Registers one secret, returning its placeholder.
    ///
    /// Idempotent: learning the same value twice keeps the first placeholder,
    /// which is what makes `device-3` mean the same thing all case long.
    fn learn(&mut self, kind: Kind, secret: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let secret = secret.trim();
        if !is_worth_hiding(kind, secret) {
            return None;
        }
        if let Some(existing) = self
            .forward
            .iter()
            .find(|(value, _)| value == secret)
            .map(|(_, placeholder)| placeholder.clone())
        {
            return Some(existing);
        }

        let next = self.counts.entry(kind).or_insert(0);
        *next += 1;
        let placeholder = format!("{}-{}", kind.prefix(), next);

        self.forward.push((secret.to_string(), placeholder.clone()));
        // Longest first: "printer.local" must be replaced before "printer",
        // or the tail is left dangling as "host-4.local".
        self.forward
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        self.back.insert(placeholder.clone(), secret.to_string());
        Some(placeholder)
    }

    /// Registers one operator-chosen name — a network's title, say.
    pub fn learn_name(&mut self, value: &str) {
        self.learn(Kind::Host, value);
    }

    /// Takes vocabulary from a stored scan.
    pub fn learn_snapshot(&mut self, snapshot: &ScanSnapshot) {
        if !self.enabled {
            return;
        }

        self.learn(Kind::Host, &snapshot.host.hostname);
        if let Some(public) = &snapshot.connectivity.public_ip {
            self.learn(Kind::WanIp, public);
        }

        for device in &snapshot.devices {
            if let Some(mac) = &device.mac {
                self.learn(Kind::Device, mac);
            }
            for hostname in &device.hostnames {
                self.learn(Kind::Host, hostname);
            }
            for value in [
                &device.reverse_dns,
                &device.unifi_name,
                &device.unifi_note,
                &device.access_point,
            ]
            .into_iter()
            .flatten()
            {
                self.learn(Kind::Host, value);
            }
            // The display name is often derived from a hostname or an operator
            // alias, so it has to go too — but it is sometimes just the IP or a
            // vendor guess, which `is_worth_hiding` declines.
            self.learn(Kind::Host, &device.display_name);

            for service in &device.mdns {
                self.learn(Kind::Host, &service.name);
                if let Some(hostname) = &service.hostname {
                    self.learn(Kind::Host, hostname);
                }
            }
            for record in &device.ssdp {
                if let Some(friendly) = &record.friendly_name {
                    self.learn(Kind::Host, friendly);
                }
                // Serial numbers identify a unit uniquely and diagnose nothing.
                if let Some(serial) = &record.serial_number {
                    self.learn(Kind::Host, serial);
                }
            }
            if let Some(netbios) = &device.netbios {
                for name in &netbios.names {
                    self.learn(Kind::Host, name);
                }
            }
        }

        if let Some(wifi) = &snapshot.wifi.data {
            for network in wifi.networks.iter().chain(wifi.current.iter()) {
                self.learn(Kind::Wifi, &network.ssid);
                if let Some(bssid) = &network.bssid {
                    self.learn(Kind::Device, bssid);
                }
            }
        }

        if let Some(unifi) = &snapshot.unifi {
            self.learn_unifi(unifi);
        }
    }

    /// Takes vocabulary from live controller data.
    ///
    /// Called by the tools that fetch it, because a client that joined since
    /// the last scan appears nowhere in the stored snapshot and would otherwise
    /// be the one name that escapes.
    pub fn learn_unifi(&mut self, snapshot: &UnifiSnapshot) {
        if !self.enabled {
            return;
        }

        self.learn(Kind::Host, &snapshot.controller_host);

        for device in &snapshot.devices {
            self.learn(Kind::Host, &device.name);
            if let Some(mac) = &device.mac {
                self.learn(Kind::Device, mac);
            }
        }
        for device in &snapshot.raw_devices {
            if let Some(name) = &device.name {
                self.learn(Kind::Host, name);
            }
            if let Some(mac) = &device.mac {
                self.learn(Kind::Device, mac);
            }
        }
        for client in snapshot.clients.iter().chain(snapshot.known_clients.iter()) {
            for value in [&client.name, &client.hostname, &client.note]
                .into_iter()
                .flatten()
            {
                self.learn(Kind::Host, value);
            }
            for mac in [&client.mac, &client.ap_mac, &client.sw_mac]
                .into_iter()
                .flatten()
            {
                self.learn(Kind::Device, mac);
            }
            if let Some(essid) = &client.essid {
                self.learn(Kind::Wifi, essid);
            }
        }
        // Neighbouring access points belong to other people entirely.
        for neighbor in &snapshot.neighbor_aps {
            if let Some(ssid) = &neighbor.ssid {
                self.learn(Kind::Wifi, ssid);
            }
            if let Some(bssid) = &neighbor.bssid {
                self.learn(Kind::Device, bssid);
            }
        }
    }

    /// Replaces every known secret, plus any MAC or public address found by
    /// shape, and returns the result.
    ///
    /// Discovery by shape is why this takes `&mut self`: an address that was
    /// never in any snapshot still must not leave, so it is learned the moment
    /// it is seen.
    pub fn redact(&mut self, text: &str) -> String {
        if !self.enabled || text.is_empty() {
            return text.to_string();
        }

        // Shape first, so a MAC embedded in a longer learned string is not
        // missed once that string has been substituted away.
        let mut out = self.hide_by_shape(text);

        // Cloned because the substitution below cannot borrow `self` while the
        // list is being walked; the table is small and this runs once per turn.
        let table = self.forward.clone();
        for (secret, placeholder) in &table {
            out = replace_delimited(&out, secret, placeholder);
        }
        out
    }

    /// Finds MACs and public addresses wherever they appear and learns them.
    fn hide_by_shape(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let bytes: Vec<char> = text.chars().collect();
        let mut index = 0;

        while index < bytes.len() {
            if let Some((length, replacement)) = self.match_at(&bytes, index) {
                out.push_str(&replacement);
                index += length;
            } else {
                out.push(bytes[index]);
                index += 1;
            }
        }
        out
    }

    /// A MAC or public address starting exactly at `index`, if one is there.
    fn match_at(&mut self, chars: &[char], index: usize) -> Option<(usize, String)> {
        if index > 0 && is_token_char(chars[index - 1]) {
            return None;
        }

        if let Some(length) = mac_length(chars, index) {
            let raw: String = chars[index..index + length].iter().collect();
            let placeholder = self.learn(Kind::Device, &raw.to_ascii_lowercase())?;
            return Some((length, placeholder));
        }

        if let Some((length, addr)) = ipv4_at(chars, index) {
            if should_hide_address(addr) {
                let placeholder = self.learn(Kind::WanIp, &addr.to_string())?;
                return Some((length, placeholder));
            }
        }

        None
    }

    /// Puts the real values back, for storage and for the user to read.
    pub fn restore(&self, text: &str) -> String {
        if !self.enabled || self.back.is_empty() {
            return text.to_string();
        }

        // Longest placeholder first, so `host-10` is not eaten by `host-1`.
        let mut pairs: Vec<(&String, &String)> = self.back.iter().collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(b.0)));

        let mut out = text.to_string();
        for (placeholder, secret) in pairs {
            out = replace_delimited(&out, placeholder, secret);
        }
        out
    }
}

/// Whether a value is both identifying and safe to substitute.
///
/// The exclusions matter more than the inclusions. Short and numeric values are
/// refused because substituting `TV` or `5` would corrupt unrelated text, and a
/// *name* that looks like an address is refused outright: a device whose
/// display name happens to be its own IP would otherwise take every mention of
/// that address with it, and private addresses are the backbone of the
/// diagnosis.
fn is_worth_hiding(kind: Kind, value: &str) -> bool {
    if value.len() < 3 || value.len() > 200 {
        return false;
    }

    // An address is only ever hidden as an address, and only when it is one
    // this machine's operator owns.
    if kind == Kind::WanIp {
        return value.parse::<Ipv4Addr>().is_ok_and(should_hide_address);
    }

    if value.parse::<Ipv4Addr>().is_ok() {
        return false;
    }
    // "10.0.3.0/24" and similar: the network itself is not a secret.
    if let Some((head, _)) = value.split_once('/') {
        if head.parse::<Ipv4Addr>().is_ok() {
            return false;
        }
    }
    if value.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return false;
    }
    // Generic labels the app itself generates carry no identity, and hiding
    // them would only make the evidence harder to read.
    const GENERIC: &[&str] = &[
        "unknown",
        "unnamed",
        "localhost",
        "gateway",
        "router",
        "default",
        "this machine",
        "unnamed device",
        "unnamed client",
    ];
    !GENERIC.contains(&value.to_ascii_lowercase().as_str())
}

/// Whether a public address should be hidden.
fn should_hide_address(addr: Ipv4Addr) -> bool {
    if netutil::is_private_ipv4(addr) || addr.is_loopback() || addr.is_link_local() {
        return false;
    }
    if addr.is_unspecified() || addr.is_broadcast() || addr.is_multicast() {
        return false;
    }
    // Netmasks are not addresses. `255.255.255.0` is public by every other
    // test and would be replaced with a placeholder that reads as nonsense.
    if is_netmask(addr) {
        return false;
    }
    !WELL_KNOWN_RESOLVERS.contains(&addr.to_string().as_str())
}

/// A contiguous run of ones followed by zeros — a mask, not a host.
fn is_netmask(addr: Ipv4Addr) -> bool {
    let bits = u32::from(addr);
    let inverted = !bits;
    inverted.wrapping_add(1) & inverted == 0
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_'
}

/// The length of a MAC starting at `index`, if one starts there.
fn mac_length(chars: &[char], index: usize) -> Option<usize> {
    const OCTETS: usize = 6;
    const LENGTH: usize = OCTETS * 3 - 1;

    if index + LENGTH > chars.len() {
        return None;
    }
    let separator = chars[index + 2];
    if separator != ':' && separator != '-' {
        return None;
    }
    for octet in 0..OCTETS {
        let at = index + octet * 3;
        if !chars[at].is_ascii_hexdigit() || !chars[at + 1].is_ascii_hexdigit() {
            return None;
        }
        if octet < OCTETS - 1 && chars[at + 2] != separator {
            return None;
        }
    }
    // Reject a longer hex run that merely begins with something MAC-shaped.
    if chars.get(index + LENGTH).is_some_and(|&c| is_token_char(c)) {
        return None;
    }
    Some(LENGTH)
}

/// An IPv4 address starting at `index`, with the length of its text.
fn ipv4_at(chars: &[char], index: usize) -> Option<(usize, Ipv4Addr)> {
    let mut end = index;
    while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == '.') {
        end += 1;
    }
    if end == index {
        return None;
    }
    // A trailing letter means this was part of something else entirely.
    if chars.get(end).is_some_and(|&c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let text: String = chars[index..end].iter().collect();
    let addr = text.parse::<Ipv4Addr>().ok()?;
    Some((end - index, addr))
}

/// Replaces `needle` with `replacement`, but only where the match is a whole
/// token rather than part of a longer word.
///
/// Without this, a device named `mac` rewrites every `machine` in the evidence.
fn replace_delimited(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || !haystack.contains(needle) {
        return haystack.to_string();
    }

    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    let mut consumed = 0usize;

    while let Some(offset) = rest.find(needle) {
        let absolute = consumed + offset;
        // Written as matches rather than `is_none_or`, which needs a newer
        // toolchain than this crate's MSRV.
        let before_ok = match haystack[..absolute].chars().next_back() {
            Some(c) => !is_token_char(c),
            None => true,
        };
        let after_ok = match haystack[absolute + needle.len()..].chars().next() {
            Some(c) => !is_token_char(c),
            None => true,
        };

        out.push_str(&rest[..offset]);
        if before_ok && after_ok {
            out.push_str(replacement);
        } else {
            out.push_str(needle);
        }
        let advance = offset + needle.len();
        rest = &rest[advance..];
        consumed += advance;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Redactor {
        let mut redactor = Redactor::new(true);
        redactor.learn(Kind::Host, "studio-macbook");
        redactor.learn(Kind::Wifi, "Ashgrove_5G");
        redactor.learn(Kind::Device, "aa:bb:cc:dd:ee:ff");
        redactor
    }

    #[test]
    fn identifiers_become_stable_placeholders() {
        let mut redactor = seeded();
        let out = redactor.redact("studio-macbook joined Ashgrove_5G via aa:bb:cc:dd:ee:ff");
        assert_eq!(out, "host-1 joined wifi-1 via device-1");

        // Stable across calls: the model must be able to refer back.
        let again = redactor.redact("studio-macbook again");
        assert_eq!(again, "host-1 again");
    }

    #[test]
    fn redaction_round_trips_to_exactly_what_went_in() {
        // The transcript is stored with real values and the user reads real
        // names, so anything lost here is lost from the app's own record.
        let mut redactor = seeded();
        let original =
            "studio-macbook on Ashgrove_5G (aa:bb:cc:dd:ee:ff) at 10.0.3.14, wan 203.0.113.9";

        let hidden = redactor.redact(original);
        assert!(!hidden.contains("studio-macbook"));
        assert!(!hidden.contains("Ashgrove_5G"));
        assert!(!hidden.contains("aa:bb:cc:dd:ee:ff"));
        assert!(!hidden.contains("203.0.113.9"));

        assert_eq!(redactor.restore(&hidden), original);
    }

    #[test]
    fn private_addresses_pass_through_untouched() {
        // The whole diagnosis is built on knowing which address is which, and
        // 10.0.3.14 identifies nobody outside the house that uses it.
        let mut redactor = Redactor::new(true);
        let text = "10.0.3.14 -> 192.168.1.1 -> 172.16.0.5 (mask 255.255.255.0)";
        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn the_diagnostic_payload_is_left_alone() {
        // Vendors, models, ports, speeds, duplex, signal and error counts are
        // what actually identify the fault, and none of them identify a person.
        let mut redactor = seeded();
        let text = "Ubiquiti USW-MINI port 7: 1000 Mbps half duplex, 14200 rx_errors, -67 dBm, \
                    tx_rate 6000 kbps";
        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn a_public_address_is_hidden_but_a_public_resolver_is_not() {
        // "Your DNS is Google's" is worth knowing and conceals nothing;
        // the user's own public address geolocates them.
        let mut redactor = Redactor::new(true);
        let out = redactor.redact("wan 203.0.113.9, dns 8.8.8.8 and 1.1.1.1");
        assert_eq!(out, "wan wan-ip-1, dns 8.8.8.8 and 1.1.1.1");
    }

    #[test]
    fn an_unlearned_mac_is_still_caught_by_shape() {
        // A client that joined since the last scan appears in no vocabulary,
        // and is exactly the one that must not slip out.
        let mut redactor = Redactor::new(true);
        let out = redactor.redact("new client 11:22:33:44:55:66 appeared");
        assert_eq!(out, "new client device-1 appeared");
        assert_eq!(
            redactor.restore(&out),
            "new client 11:22:33:44:55:66 appeared"
        );
    }

    #[test]
    fn a_name_inside_a_longer_word_is_not_rewritten() {
        let mut redactor = Redactor::new(true);
        redactor.learn(Kind::Host, "mac");
        assert_eq!(
            redactor.redact("mac is on the machine, mac-mini too"),
            "host-1 is on the machine, mac-mini too"
        );
    }

    #[test]
    fn a_longer_name_is_replaced_before_the_shorter_one_it_contains() {
        let mut redactor = Redactor::new(true);
        redactor.learn(Kind::Host, "printer");
        redactor.learn(Kind::Host, "printer.local");
        // Learned second but longer, so it must win — otherwise the tail is
        // left dangling as "host-1.local".
        assert_eq!(redactor.redact("printer.local"), "host-2");
    }

    #[test]
    fn placeholders_past_ten_restore_correctly() {
        // "host-1" is a prefix of "host-10"; replacing shortest-first would
        // corrupt every two-digit placeholder.
        let mut redactor = Redactor::new(true);
        for index in 1..=12 {
            redactor.learn(Kind::Host, &format!("machine-{index}"));
        }
        let hidden = redactor.redact("machine-1 talks to machine-10");
        assert_eq!(hidden, "host-1 talks to host-10");
        assert_eq!(redactor.restore(&hidden), "machine-1 talks to machine-10");
    }

    #[test]
    fn a_device_named_after_its_own_address_does_not_take_the_address_with_it() {
        // Learning "10.0.3.14" as a secret would replace every mention of that
        // address in the evidence — the diagnosis depends on those.
        let mut redactor = Redactor::new(true);
        assert!(redactor.learn(Kind::Host, "10.0.3.14").is_none());
        assert!(redactor.learn(Kind::Host, "10.0.3.0/24").is_none());
        assert_eq!(redactor.redact("10.0.3.14 is slow"), "10.0.3.14 is slow");
    }

    #[test]
    fn generic_labels_are_left_readable() {
        let mut redactor = Redactor::new(true);
        assert!(redactor.learn(Kind::Host, "Gateway").is_none());
        assert!(redactor.learn(Kind::Host, "unknown").is_none());
        assert!(
            redactor.learn(Kind::Host, "TV").is_none(),
            "too short to substitute safely"
        );
    }

    #[test]
    fn a_disabled_redactor_is_a_pass_through() {
        let mut redactor = Redactor::new(false);
        redactor.learn(Kind::Host, "studio-macbook");
        let text = "studio-macbook at 203.0.113.9 via aa:bb:cc:dd:ee:ff";
        assert_eq!(redactor.redact(text), text);
        assert_eq!(redactor.restore(text), text);
        assert!(redactor.is_empty());
    }

    #[test]
    fn learning_the_same_value_twice_keeps_one_placeholder() {
        let mut redactor = Redactor::new(true);
        let first = redactor.learn(Kind::Host, "printer").unwrap();
        let second = redactor.learn(Kind::Host, "printer").unwrap();
        assert_eq!(first, second);
        assert_eq!(redactor.len(), 1);
    }

    #[test]
    fn a_hex_run_longer_than_a_mac_is_not_mistaken_for_one() {
        let mut redactor = Redactor::new(true);
        let text = "hash aa:bb:cc:dd:ee:ff:00:11 stays";
        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn netmasks_survive() {
        assert!(is_netmask("255.255.255.0".parse().unwrap()));
        assert!(is_netmask("255.255.0.0".parse().unwrap()));
        assert!(!is_netmask("203.0.113.9".parse().unwrap()));
    }

    /// A snapshot carrying one device with every kind of identifier on it.
    fn snapshot_with_one_device() -> ScanSnapshot {
        use crate::types::*;

        ScanSnapshot {
            id: "s1".into(),
            started_at: "2026-08-09T10:00:00Z".into(),
            finished_at: "2026-08-09T10:01:00Z".into(),
            duration_ms: 60_000,
            host: HostInfo {
                hostname: "marek-desktop".into(),
                platform: "test".into(),
                os: "Linux".into(),
                arch: "x86_64".into(),
                app_version: "1.5.0".into(),
                interfaces: Vec::new(),
                routes: Vec::new(),
                gateway: None,
                dns: DnsConfig::default(),
                scan_targets: Vec::new(),
            },
            devices: vec![Device {
                ip: "10.0.3.14".into(),
                mac: Some("aa:bb:cc:dd:ee:ff".into()),
                vendor: Some("Apple".into()),
                mac_randomized: None,
                hostnames: vec!["studio-macbook.local".into()],
                display_name: "Studio MacBook".into(),
                device_type: DeviceType::Computer,
                type_evidence: Vec::new(),
                is_gateway: false,
                is_self: false,
                responded_to_ping: true,
                discovered_by: Vec::new(),
                latency_ms: Some(1.2),
                ports: Vec::new(),
                mdns: Vec::new(),
                ssdp: Vec::new(),
                netbios: None,
                reverse_dns: Some("studio-macbook.lan".into()),
                source_range: None,
                off_subnet: false,
                first_seen: None,
                last_seen: "2026-08-09T10:01:00Z".into(),
                unifi_name: None,
                unifi_fingerprint: None,
                unifi_network: None,
                switch_port: None,
                access_point: None,
                vlan: None,
                rssi: Some(-67),
                is_wired: Some(false),
                satisfaction: None,
                channel: None,
                wifi_generation: None,
                tx_bytes: None,
                rx_bytes: None,
                unifi_uptime: None,
                unifi_first_seen: None,
                is_guest: None,
                unifi_note: None,
            }],
            connectivity: ConnectivityInfo {
                gateway: None,
                wan: Vec::new(),
                dns: Vec::new(),
                public_ip: Some("203.0.113.9".into()),
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

    #[test]
    fn vocabulary_is_taken_from_a_real_snapshot() {
        let mut redactor = Redactor::new(true);
        redactor.learn_snapshot(&snapshot_with_one_device());

        let out = redactor.redact(
            "marek-desktop reached Studio MacBook (studio-macbook.local / studio-macbook.lan, \
             aa:bb:cc:dd:ee:ff) at 10.0.3.14; vendor Apple, -67 dBm; wan 203.0.113.9",
        );

        assert!(
            !out.to_lowercase().contains("marek"),
            "no name should survive: {out}"
        );
        assert!(!out.contains("aa:bb:cc:dd:ee:ff"));
        assert!(!out.contains("203.0.113.9"));

        // What is left is what actually diagnoses the fault.
        assert!(
            out.contains("10.0.3.14"),
            "the private address is the diagnosis"
        );
        assert!(out.contains("Apple"), "the vendor identifies nobody");
        assert!(out.contains("-67 dBm"));
    }

    #[test]
    fn nothing_identifying_survives_a_full_snapshot_pass() {
        // The privacy check from the plan, as a test rather than a manual step:
        // serialise everything the assistant could quote and confirm the
        // outbound form carries no name, MAC or public address.
        let snapshot = snapshot_with_one_device();
        let mut redactor = Redactor::new(true);
        redactor.learn_snapshot(&snapshot);

        let payload = serde_json::to_string(&snapshot).unwrap();
        let hidden = redactor.redact(&payload);

        for secret in [
            "marek-desktop",
            "studio-macbook.local",
            "studio-macbook.lan",
            "Studio MacBook",
            "aa:bb:cc:dd:ee:ff",
            "203.0.113.9",
        ] {
            assert!(
                !hidden.contains(secret),
                "{secret} leaked in the outbound payload"
            );
        }
        assert!(hidden.contains("10.0.3.14"));
    }
}
