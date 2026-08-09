//! Finding the *other* subnets reachable from where this machine sits.
//!
//! A scan is deliberately confined to one network (see
//! [`ScanConfig::restrict_to_subnets`](crate::types::ScanConfig::restrict_to_subnets)),
//! which raises an obvious question the app previously left unanswered: what
//! else is out there? Evidence of neighbouring subnets already arrives on every
//! scan and was simply discarded — a device answering mDNS from an address
//! nothing local covers, a router replying two hops out, a controller listing
//! VLANs no target touched.
//!
//! # The bar for reporting something
//!
//! Only **positive proof of reachability** counts. This module never
//! extrapolates: it does not offer `10.0.4.0/24` because `10.0.3.0/24` exists,
//! and it does not walk a range guessing at neighbours. Every candidate is
//! backed by one of:
//!
//! * a route this machine's own kernel holds,
//! * a subnet the controller declares,
//! * a host at that address that answered a probe,
//! * a router at that address that answered a traceroute.
//!
//! # Where the honesty line falls
//!
//! Routes and controller records state a **prefix**; the network is known
//! exactly. A responder or a hop proves one *address* is reachable and says
//! nothing about the mask, so the /24 around it is an assumption and is flagged
//! as one. That distinction is carried through to the UI rather than smoothed
//! over, because acting on a wrong prefix is how a scan spills into a
//! neighbouring network — the failure the per-network split exists to prevent.

use crate::netutil::{cidrs_overlap, parse_cidr_any, to_slash24, MIN_PREFIX};
use crate::scan::correlate::OffScopeSighting;
use crate::types::{ConnectivityInfo, HostInfo};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// RFC1918 only — 10/8, 172.16/12, 192.168/16.
///
/// Deliberately stricter than [`crate::netutil::is_private_ipv4`], which also
/// admits carrier-grade NAT and link-local space because it answers a different
/// question: "is this safe to probe?". Neither of those is a network anyone here
/// administers, so neither belongs in a list of networks to start tracking.
/// 100.64.0.0/10 in particular is the ISP's, and it shows up on the path out of
/// a great many home connections.
fn is_site_local(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

/// Why we believe a subnet is there and reachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SubnetEvidence {
    /// This machine's routing table has a path to it.
    #[serde(rename_all = "camelCase")]
    Route { dev: String, via: Option<String> },
    /// The UniFi controller declares the network.
    #[serde(rename_all = "camelCase")]
    Controller { name: String, vlan: Option<u32> },
    /// A host at this address answered us during the scan.
    #[serde(rename_all = "camelCase")]
    Responder { ip: String, detail: String },
    /// A router at this address answered a traceroute probe.
    #[serde(rename_all = "camelCase")]
    TraceHop { ip: String, hop: u32 },
}

impl SubnetEvidence {
    /// Whether this evidence establishes the prefix, or only one address.
    fn states_prefix(&self) -> bool {
        matches!(
            self,
            SubnetEvidence::Route { .. } | SubnetEvidence::Controller { .. }
        )
    }

    pub fn explain(&self) -> String {
        match self {
            SubnetEvidence::Route { dev, via } => match via {
                Some(via) => format!("this machine routes to it via {via} on {dev}"),
                None => format!("this machine has a direct route to it on {dev}"),
            },
            SubnetEvidence::Controller { name, vlan } => match vlan {
                Some(vlan) => format!("the controller defines it as \"{name}\" on VLAN {vlan}"),
                None => format!("the controller defines it as \"{name}\""),
            },
            SubnetEvidence::Responder { ip, detail } => {
                format!("{ip} answered during the last scan ({detail})")
            }
            SubnetEvidence::TraceHop { ip, hop } => {
                format!("a router at {ip} answered at hop {hop} on the way out")
            }
        }
    }
}

/// A subnet other than the ones being tracked, and the case for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjacentSubnet {
    pub cidr: String,
    /// False when the prefix was stated by a route or the controller; true when
    /// only a single address is proven and /24 is an assumption.
    pub prefix_assumed: bool,
    pub evidence: Vec<SubnetEvidence>,
    /// A saved network already covers this range.
    pub already_tracked: bool,
    /// Ready-made one-liner, so every surface explains it the same way.
    pub summary: String,
}

/// Everything the search draws on. All of it is already collected by a scan.
#[derive(Default)]
pub struct Sources<'a> {
    pub host: Option<&'a HostInfo>,
    pub connectivity: Option<&'a ConnectivityInfo>,
    /// Addresses the scan heard from but deliberately left out of its results.
    pub off_scope: &'a [OffScopeSighting],
    /// Controller-declared networks as `(name, subnet, vlan)`.
    pub controller_networks: Vec<(String, String, Option<u32>)>,
    /// Subnets already covered by a saved network.
    pub tracked: Vec<String>,
}

/// Collects candidates, strongest evidence first.
pub fn discover(sources: &Sources) -> Vec<AdjacentSubnet> {
    let local = local_subnets(sources.host);
    let mut found: Vec<AdjacentSubnet> = Vec::new();

    for (cidr, evidence) in gather(sources, &local) {
        if !is_scannable_size(&cidr) {
            continue;
        }
        match found.iter_mut().find(|entry| entry.cidr == cidr) {
            Some(entry) => {
                if !entry.evidence.contains(&evidence) {
                    entry.evidence.push(evidence);
                }
            }
            None => found.push(AdjacentSubnet {
                cidr,
                prefix_assumed: true,
                evidence: vec![evidence],
                already_tracked: false,
                summary: String::new(),
            }),
        }
    }

    for entry in &mut found {
        entry.prefix_assumed = !entry.evidence.iter().any(SubnetEvidence::states_prefix);
        entry.already_tracked = sources
            .tracked
            .iter()
            .any(|subnet| cidrs_overlap(subnet, &entry.cidr));
        entry.summary = summarize(entry);
    }

    // Strongest first: a stated prefix outranks an assumed one, then weight of
    // evidence, then address order so the list is stable between runs.
    found.sort_by(|a, b| {
        a.prefix_assumed
            .cmp(&b.prefix_assumed)
            .then(b.evidence.len().cmp(&a.evidence.len()))
            .then(a.cidr.cmp(&b.cidr))
    });
    found
}

fn summarize(entry: &AdjacentSubnet) -> String {
    let first = entry
        .evidence
        .first()
        .map(SubnetEvidence::explain)
        .unwrap_or_default();
    let rest = entry.evidence.len().saturating_sub(1);

    let mut summary = match rest {
        0 => first,
        1 => format!("{first}, and one other sign"),
        n => format!("{first}, and {n} other signs"),
    };
    if !summary.is_empty() {
        summary[..1].make_ascii_uppercase();
        summary.push('.');
    }
    if entry.prefix_assumed {
        summary.push_str(" The exact size is not known, so /24 is assumed.");
    }
    summary
}

fn gather(sources: &Sources, local: &[String]) -> Vec<(String, SubnetEvidence)> {
    let mut out = Vec::new();

    // ---- the kernel's own routing table: a stated prefix, no inference at all
    if let Some(host) = sources.host {
        for route in &host.routes {
            // A default route leads to the internet, not to a local subnet.
            if route.destination == "default" || route.destination.starts_with("0.0.0.0/0") {
                continue;
            }
            // A container bridge, a VPN's own transport, a VM host-only net:
            // real routes, but not networks anyone is diagnosing. The scanner
            // already declines to sweep these interfaces, so offering them here
            // would only produce a network that scans nothing.
            if crate::scan::hostinfo::is_virtual_interface(&route.dev) {
                continue;
            }
            let Ok(parsed) = parse_cidr_any(&route.destination) else {
                continue;
            };
            if !is_site_local(parsed.network) {
                continue;
            }
            let cidr = parsed.canonical();
            if is_known(&cidr, local) {
                continue;
            }
            out.push((
                cidr,
                SubnetEvidence::Route {
                    dev: route.dev.clone(),
                    via: route.via.clone(),
                },
            ));
        }
    }

    // ---- the controller's own configuration: also a stated prefix
    for (name, subnet, vlan) in &sources.controller_networks {
        let Ok(parsed) = parse_cidr_any(subnet) else {
            continue;
        };
        if !is_site_local(parsed.network) {
            continue;
        }
        let cidr = parsed.canonical();
        if is_known(&cidr, local) {
            continue;
        }
        out.push((
            cidr,
            SubnetEvidence::Controller {
                name: name.clone(),
                vlan: *vlan,
            },
        ));
    }

    // ---- hosts that answered from outside every scanned range
    //
    // These are recorded by the scan precisely because they are *not* this
    // network's devices; they never enter `devices`, so reading them from there
    // would find nothing.
    for sighting in sources.off_scope {
        let Ok(ip) = sighting.ip.parse::<Ipv4Addr>() else {
            continue;
        };
        if !is_site_local(ip) {
            continue;
        }
        let cidr = to_slash24(ip);
        if is_known(&cidr, local) {
            continue;
        }
        let detail = if sighting.sources.is_empty() {
            "responded".to_string()
        } else {
            format!("via {}", sighting.sources.join(", "))
        };
        out.push((
            cidr,
            SubnetEvidence::Responder {
                ip: sighting.ip.clone(),
                detail,
            },
        ));
    }

    // ---- routers on the way out
    for (ip, hop) in transit_hops(sources.connectivity) {
        let cidr = to_slash24(ip);
        if is_known(&cidr, local) {
            continue;
        }
        out.push((
            cidr,
            SubnetEvidence::TraceHop {
                ip: ip.to_string(),
                hop,
            },
        ));
    }

    out
}

/// Private routers between this machine and the first public hop.
///
/// The walk **stops at the first non-private reply**: past that point the path
/// is the ISP's, and a private address appearing there is carrier equipment,
/// not a network anyone here can scan. That single rule is what keeps
/// `100.64.0.0/10` carrier NAT and the `192.0.0.0/24` protocol-assignment block
/// used by DS-Lite out of the results.
fn transit_hops(connectivity: Option<&ConnectivityInfo>) -> Vec<(Ipv4Addr, u32)> {
    let Some(trace) = connectivity.and_then(|c| c.trace.data.as_ref()) else {
        return Vec::new();
    };

    let mut hops = Vec::new();
    for hop in &trace.hops {
        let Some(host) = hop.host.as_deref() else {
            // A silent hop hides nothing we can act on, but it does not mean we
            // have left the private path yet — keep walking.
            continue;
        };
        let Ok(ip) = host.parse::<Ipv4Addr>() else {
            continue;
        };
        if !is_site_local(ip) {
            break;
        }
        hops.push((ip, hop.hop));
    }
    hops
}

fn local_subnets(host: Option<&HostInfo>) -> Vec<String> {
    host.map(|host| {
        host.scan_targets
            .iter()
            .map(|target| target.cidr.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// Already part of what was scanned — not news.
fn is_known(cidr: &str, local: &[String]) -> bool {
    local.iter().any(|known| cidrs_overlap(known, cidr))
}

/// Small enough that the app would actually agree to scan it.
///
/// A summary route such as `10.0.0.0/8`, or a container bridge's `/16`, is a
/// real route to real address space and still the wrong thing to offer: adding
/// it is refused by [`validate_range`](crate::scan::hostinfo::validate_range),
/// so the button would simply fail. Filtering here keeps the list to things that
/// can be acted on.
fn is_scannable_size(cidr: &str) -> bool {
    parse_cidr_any(cidr)
        .map(|parsed| parsed.prefix >= MIN_PREFIX)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ProbeResult, ProbeStatus, RouteInfo, ScanTarget, TargetSource, TraceHop, TraceResult,
    };

    fn host_on(subnet: &str, routes: Vec<RouteInfo>) -> HostInfo {
        HostInfo {
            hostname: "test".into(),
            platform: "test".into(),
            os: "Linux".into(),
            arch: "x86_64".into(),
            app_version: "0".into(),
            interfaces: Vec::new(),
            routes,
            gateway: None,
            dns: Default::default(),
            scan_targets: vec![ScanTarget {
                cidr: subnet.into(),
                source: TargetSource::Local,
                host_count: 254,
                note: None,
            }],
        }
    }

    fn route(destination: &str, dev: &str, via: Option<&str>) -> RouteInfo {
        RouteInfo {
            destination: destination.into(),
            via: via.map(str::to_string),
            dev: dev.into(),
            metric: None,
            raw: String::new(),
        }
    }

    fn trace(hosts: &[Option<&str>]) -> ConnectivityInfo {
        ConnectivityInfo {
            gateway: None,
            wan: Vec::new(),
            dns: Vec::new(),
            public_ip: None,
            wan_reachable: true,
            trace: ProbeResult {
                status: ProbeStatus::Ok,
                data: Some(TraceResult {
                    tool: "mtr".into(),
                    hops: hosts
                        .iter()
                        .enumerate()
                        .map(|(index, host)| TraceHop {
                            hop: index as u32 + 1,
                            host: host.map(str::to_string),
                            rtt_ms: None,
                            loss_percent: None,
                            timeout: host.is_none(),
                        })
                        .collect(),
                }),
                detail: None,
            },
        }
    }

    #[test]
    fn a_private_router_on_the_way_out_is_a_neighbouring_network() {
        // The reported path: 10.0.3.1 (own gateway) → 10.0.2.1 → … → 1.1.1.1.
        let host = host_on("10.0.3.0/24", Vec::new());
        let connectivity = trace(&[
            Some("10.0.3.1"),
            Some("10.0.2.1"),
            None,
            Some("192.0.0.1"),
            Some("1.1.1.1"),
        ]);

        let found = discover(&Sources {
            host: Some(&host),
            connectivity: Some(&connectivity),
            ..Default::default()
        });

        assert_eq!(
            found.iter().map(|s| s.cidr.as_str()).collect::<Vec<_>>(),
            vec!["10.0.2.0/24"],
            "the own subnet, the protocol-assignment block and the public hop must all be excluded"
        );
        assert!(
            found[0].prefix_assumed,
            "a hop proves one address, not a mask"
        );
    }

    #[test]
    fn the_walk_stops_at_the_first_public_hop() {
        // Anything private appearing beyond the ISP edge is carrier equipment.
        let host = host_on("192.168.1.0/24", Vec::new());
        let connectivity = trace(&[
            Some("192.168.1.1"),
            Some("203.0.113.1"),
            Some("10.255.0.1"),
            Some("100.64.0.1"),
        ]);

        let found = discover(&Sources {
            host: Some(&host),
            connectivity: Some(&connectivity),
            ..Default::default()
        });
        assert!(
            found.is_empty(),
            "nothing past the first public hop is local, got {found:?}"
        );
    }

    #[test]
    fn carrier_grade_nat_is_never_offered() {
        let host = host_on("192.168.1.0/24", Vec::new());
        let connectivity = trace(&[Some("192.168.1.1"), Some("100.64.0.1"), Some("1.1.1.1")]);

        let found = discover(&Sources {
            host: Some(&host),
            connectivity: Some(&connectivity),
            ..Default::default()
        });
        assert!(
            found.is_empty(),
            "100.64/10 is the carrier's, not a scannable local network"
        );
    }

    #[test]
    fn a_kernel_route_states_the_prefix_rather_than_assuming_it() {
        let host = host_on(
            "10.0.3.0/24",
            vec![
                route("default", "wlo1", Some("10.0.3.1")),
                route("10.0.9.0/25", "wlo1", Some("10.0.3.1")),
            ],
        );

        let found = discover(&Sources {
            host: Some(&host),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cidr, "10.0.9.0/25");
        assert!(
            !found[0].prefix_assumed,
            "the routing table states the mask outright"
        );
    }

    #[test]
    fn summary_routes_and_public_destinations_are_not_subnets() {
        let host = host_on(
            "10.0.3.0/24",
            vec![
                route("10.0.0.0/8", "wlo1", Some("10.0.3.1")),
                route("8.8.8.0/24", "wlo1", Some("10.0.3.1")),
                route("0.0.0.0/0", "wlo1", Some("10.0.3.1")),
            ],
        );

        let found = discover(&Sources {
            host: Some(&host),
            ..Default::default()
        });
        assert!(found.is_empty(), "got {found:?}");
    }

    #[test]
    fn container_bridges_and_link_local_are_not_offered() {
        // Taken from a real routing table. Both are genuine routes; neither is
        // a network anyone is diagnosing, and the /16 could not be scanned even
        // if it were added.
        let host = host_on(
            "10.0.2.0/24",
            vec![
                route("default", "wlo1", Some("10.0.3.1")),
                route("10.0.3.0/24", "wlo1", None),
                route("169.254.0.0/16", "wlo1", None),
                route("172.17.0.0/16", "docker0", None),
            ],
        );

        let found = discover(&Sources {
            host: Some(&host),
            tracked: vec!["10.0.2.0/24".into()],
            ..Default::default()
        });

        assert_eq!(
            found.iter().map(|s| s.cidr.as_str()).collect::<Vec<_>>(),
            vec!["10.0.3.0/24"],
            "only the real neighbouring LAN survives"
        );
    }

    #[test]
    fn a_responder_in_a_range_too_large_to_scan_is_not_offered() {
        // The /24 around a responder is always scannable, but a route or
        // controller record need not be — and offering one produces a button
        // that can only fail.
        assert!(!is_scannable_size("10.0.0.0/8"));
        assert!(is_scannable_size("10.0.107.0/24"));
    }

    #[test]
    fn an_off_scope_responder_proposes_the_slash_24_around_it() {
        // 10.0.107.110 answered mDNS while only 10.0.3.0/24 was scanned. It is
        // deliberately not a device of that scan, so the sighting — not the
        // device list — is what this has to read.
        let host = host_on("10.0.3.0/24", Vec::new());
        let sighting = OffScopeSighting {
            ip: "10.0.107.110".into(),
            sources: vec!["mdns".into()],
        };

        let found = discover(&Sources {
            host: Some(&host),
            off_scope: std::slice::from_ref(&sighting),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].cidr, "10.0.107.0/24");
        assert!(found[0].prefix_assumed);
        assert!(found[0].summary.contains("10.0.107.110"));
    }

    #[test]
    fn a_stated_prefix_outranks_an_assumed_one_and_evidence_merges() {
        let host = host_on(
            "10.0.3.0/24",
            vec![route("10.0.2.0/24", "wlo1", Some("10.0.3.1"))],
        );
        let connectivity = trace(&[Some("10.0.3.1"), Some("10.0.2.1"), Some("1.1.1.1")]);

        let found = discover(&Sources {
            host: Some(&host),
            connectivity: Some(&connectivity),
            controller_networks: vec![("Guest".into(), "10.0.50.1/24".into(), Some(50))],
            ..Default::default()
        });

        assert_eq!(
            found.iter().map(|s| s.cidr.as_str()).collect::<Vec<_>>(),
            vec!["10.0.2.0/24", "10.0.50.0/24"],
            "two independent signs for 10.0.2.0/24 rank it first"
        );
        assert_eq!(found[0].evidence.len(), 2, "route and hop agree, and merge");
        assert!(!found[0].prefix_assumed);
        assert!(!found[1].prefix_assumed, "the controller states its prefix");
    }

    #[test]
    fn a_subnet_already_tracked_is_reported_but_flagged() {
        let host = host_on("10.0.3.0/24", Vec::new());
        let connectivity = trace(&[Some("10.0.3.1"), Some("10.0.2.1"), Some("1.1.1.1")]);

        let found = discover(&Sources {
            host: Some(&host),
            connectivity: Some(&connectivity),
            tracked: vec!["10.0.2.0/24".into()],
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert!(
            found[0].already_tracked,
            "offering to add it twice would create a duplicate network"
        );
    }

    #[test]
    fn nothing_is_invented_without_evidence() {
        // Neighbouring-by-arithmetic is exactly what this must never do.
        let host = host_on("10.0.3.0/24", Vec::new());
        let found = discover(&Sources {
            host: Some(&host),
            ..Default::default()
        });
        assert!(found.is_empty(), "got {found:?}");
    }
}
