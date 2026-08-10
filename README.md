# Transom

[![CI](https://github.com/MarkoVcode/transom/actions/workflows/ci.yml/badge.svg)](https://github.com/MarkoVcode/transom/actions/workflows/ci.yml)
[![Release](https://github.com/MarkoVcode/transom/actions/workflows/release.yml/badge.svg)](https://github.com/MarkoVcode/transom/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)](https://github.com/MarkoVcode/transom/releases)

*Over the transom*: something that arrived unannounced, without introduction —
from the small window above a door, which is what a transom is.

That is how every device joins your network. **Transom** is the app that notices
what came through. It discovers every device on your local network, identifies
it, measures connectivity and Wi-Fi health, and tracks what changes between
scans. Optionally, it will read a UniFi controller for what a scan cannot see
from outside a device, and — if you connect a model — investigate a symptom you
describe in your own words.

Runs on **Windows, macOS and Linux**. Needs **no administrator privileges** and
**no extra tools installed**.

---

## Install

Download the installer for your platform from the
[Releases](https://github.com/MarkoVcode/transom/releases) page:

| Platform | File |
| --- | --- |
| Windows | `.msi` or `.exe` (NSIS setup) |
| macOS (Apple silicon) | `_aarch64.dmg` |
| macOS (Intel) | `_x64.dmg` |
| Linux | `.AppImage` (portable), `.deb`, or `.rpm` |

Releases are **not code-signed**, so on first launch:

- **macOS** — Gatekeeper blocks it. Right-click the app → *Open*, or run
  `xattr -dr com.apple.quarantine "/Applications/Transom.app"`.
- **Windows** — SmartScreen shows "Windows protected your PC". Click
  *More info* → *Run anyway*.

---

## What it extracts

**Per device**
- IPv4 address, MAC, and vendor from the bundled IEEE OUI registry (53k prefixes)
- Randomized/private MACs detected and flagged rather than mislabelled
- Hostnames from mDNS, NetBIOS, reverse DNS and UPnP
- Open TCP ports with service names
- HTTP status / `Server` / page title / full response headers
- TLS certificate subject, issuer, SANs, expiry, self-signed detection
- Complete mDNS TXT records — ESPHome chip and firmware, Home Assistant version,
  printer capabilities, Apple device identifiers
- UPnP device description: manufacturer, model, serial number
- Device-type inference, **with the evidence that produced it**

**Network**
- Interfaces, addresses, MTU, link state, routing table
- DNS configuration and per-resolver response timing
- Gateway and WAN latency with min/avg/max/jitter/loss
- Public IP, traceroute with per-hop loss
- Wi-Fi link stats, channel-congestion survey and a channel recommendation

---

## Networks

Scans are grouped by **network**, and each network keeps its own history, diff
baseline and controller settings. Without this, scanning at two sites writes into
one history and the diff becomes nonsense — every device at the first site reads
as "disappeared" and every device at the second as "new".

**The subnet cannot identify a network.** Two different buildings routinely both
use `192.168.0.0/24`. Networks are told apart primarily by the **gateway's MAC
address**: a specific piece of hardware, observable without configuration, and
unchanged when the router's IP or SSID changes. SSID, subnet and resolvers act as
weaker corroborating signals when no gateway address can be read.

On launch the app fingerprints the network you are on and:

| Situation | What happens |
| --- | --- |
| Matches the selected network | Nothing — carry on |
| Matches a different saved one | Offers to switch |
| Nothing matches | Offers to create one, pre-named from the SSID |
| Only the subnet matches, ambiguously | **Asks** — it will not guess, because guessing is how two histories get merged |

Switch from the picker at the top of the sidebar, or manage them under
**Networks**. Existing installations migrate automatically: previous scans become
the first network, fingerprinted from the newest snapshot rather than from
wherever you happen to be when you upgrade.

A network showing **Weak fingerprint** has only its subnet to go on — usually
because the gateway's address had not been resolved when it was created. Select
it while connected and use *Re-detect* to pick that up.

### Locations

One place often has several networks — a main LAN, a guest SSID, a lab VLAN —
and as a flat list they read as unrelated. A **location** is a named place
networks belong to. The switcher groups by it and names it without being opened;
assign one when a new network is detected, or later under **Networks**.

A location is a label and nothing more. It has no effect on how networks are
identified, scanned or kept apart, and **deleting one keeps every network in it
along with every scan they hold** — the networks simply move to *No location*.
Locations are held by id rather than by name, so renaming one updates every
network at once and two spellings cannot split a group. An installation that
never creates one looks exactly as it did before.

### Other networks seen from here

Subnets this machine can demonstrably reach are offered under **Networks**, each
with the evidence for it — a route the host holds, a subnet the controller
declares, an address that answered, a router that answered a traceroute. Nothing
is inferred from neighbouring addresses, and a prefix that was assumed rather
than stated says so. A subnet this machine is *not* attached to can also be added
by address range.

---

## UniFi controller (optional)

Connecting a UniFi controller adds what a scan fundamentally cannot observe from
outside a device:

| | The scan knows | The controller knows |
| --- | --- | --- |
| Identity | vendor from MAC, open ports, mDNS/SSDP | operator-assigned alias, DHCP fingerprint |
| Where it is plugged in | nothing | **which switch port or access point** |
| History | point samples | continuous association |
| Intent | nothing | configured VLANs and networks |

The point is not the union of the two. It is the small set of findings that exist
**only where they disagree**:

- **Devices unknown to the controller** — reachable on the network, but no lease
  was ever issued. A static IP, something behind an unmanaged switch, or a
  spoofed address. Neither tool can produce this category alone.
- **Devices the scan never reached** — known to the controller but silent to us.
  This is where our *own* coverage is blind, which is the difference between
  "there are 29 devices" and "there are 29 devices I can see".
- **Randomized MACs identified** — phones using a private MAC have no resolvable
  vendor, and the app correctly refuses to guess. The controller has their
  hostname and fingerprint, so the join recovers the identity.
- **Ports hiding other devices** — several hardware addresses learned on one
  switch port means an unmanaged switch, VM host or bridge is behind it, and an
  entire segment the controller cannot see into.

### Infrastructure faults

The controller also reports readings about its own hardware that its dashboard
does not surface as problems:

- **Port faults** — a link that is up and negotiated at gigabit but running
  **half duplex**, one accumulating error or drop counters, or one blocked by
  spanning tree because a loop was detected. This is separate from a *degraded
  link*, which is about negotiated speed.
- **Saturated radios** — airtime at or above 80%, split into your own traffic
  and a neighbour's interference where both figures are reported. The remedy
  differs completely: your own traffic means move clients or add capacity, a
  neighbour's means change channel.
- **Devices under load** — managed hardware pinned at high sustained CPU. "The
  whole network freezes under load" is very often a saturated gateway rather
  than anything on the LAN.
- **Wireless backhaul** — access points meshed with no cable, whose uplink caps
  every client behind them. Reported as context, deliberately not counted as a
  fault.

**Check diagnostic fields** (under the controller settings) asks the controller
which of these it actually reports, and lists any diagnostic that cannot run
with the exact fields missing. Controller releases differ in what they expose,
and silence from a diagnostic that never had its evidence is not a clean bill of
health.

### Connecting one

1. In the UniFi console, create a **local** user with the **Viewer** role.
   The integration only ever reads; a read-only credential cannot change your
   network even if it leaks.
2. In the app, open **Setup & Status → UniFi controller**, enter the address,
   site, username and password, and press **Test connection**.

The password is stored in your operating system's keychain, never in scan
snapshots (which are exportable). The controller's TLS certificate is pinned on
first connection: if it ever changes, the app refuses to connect **before**
sending credentials and explains why. Only private addresses are accepted.

Works with UniFi OS (UDM, UDR, Cloud Key Gen2+) and legacy software controllers;
the flavour is detected rather than configured. An account with two-factor
authentication cannot be used — create a separate local Viewer account without
it.

Debugging without the GUI:

```bash
UNIFI_PASSWORD=… cargo run -p netdiag-core --bin netdiag-cli -- unifi <controller-ip> viewer
```

---

## Troubleshooting assistant (optional, in development)

Describe a symptom the way you would to a colleague — *"when I upload a large
file to the NAS the whole network freezes"* — and the assistant investigates and
proposes a fix, citing the readings it rests on.

**The model never measures anything.** It decides what to check and explains
what the readings mean; every number comes from this app's own probes. A finding
or remedy that cites no evidence, or cites evidence that was never gathered, is
**rejected rather than shown**.

- **Read-only.** Its twelve tools query scan results and run live probes —
  ping, traceroute, port scan, Wi-Fi survey. None of them can change your
  network, and the live ones refuse targets outside private address ranges.
- **Reproduce it while I watch.** For faults that only appear under load, it
  takes a controller reading, waits while you reproduce the problem, then
  reports **only the counters that moved** — port errors, byte counters, radio
  airtime, CPU, Wi-Fi retries — against noise floors, so ordinary traffic does
  not read as a finding.
- **It asks, and it waits.** When it needs something only you know it stops and
  asks. A case is stored per network, so the answer can come minutes later or
  after restarting the app.
- **Bounded.** Steps, output tokens and wall-clock time are all capped, and each
  limit ends the case with a reason rather than silently. There is a Stop
  button.

### Privacy

Identifiers are replaced **before anything is sent** and restored in the reply:
MACs, hostnames, Wi-Fi names and your public IP become stable placeholders
(`device-3`, `host-7`, `wifi-1`). Vendors, models, port numbers, speeds, signal
levels and error counts pass through — they carry the diagnosis and identify
nobody. Private addresses pass through deliberately, since the topology is the
subject. This is on by default and can be turned off.

### Connecting a model

**Setup & Status → Troubleshooting assistant.** Off until you connect one.

| | Where it runs | Notes |
| --- | --- | --- |
| **Anthropic** | Cloud | Best reasoning. Key stored in the OS keychain, never in a settings file, and never returned to the interface. |
| **Ollama** | This machine | Nothing leaves the machine, at the cost of a weaker diagnosis — local models handle multi-step tool use less reliably. |

Marked **in development**: the investigation loop and its safeguards are
complete and tested, but the feature is new and the quality of a diagnosis
depends heavily on the model behind it.

---

## Updates

On launch the app checks GitHub for a newer release and shows a notice if one
exists, with a button that opens the release page. It is deliberately quiet:

- The check runs in Rust, not the webview, so no CSP exception is needed.
- It **fails silently** when offline — this app is frequently launched precisely
  because the internet is broken.
- Results are cached for six hours, so relaunching cannot exhaust GitHub's
  unauthenticated rate limit.
- "Skip this version" suppresses that release only; a later one still notifies.
  Checks can be turned off entirely.

---

## Setup & Status

The app includes a **Setup & Status** page that reports what it can actually do
on your machine, tiered by how much it matters:

| Tier | Meaning |
| --- | --- |
| **Required** | The app cannot scan without this. Scanning is disabled and a blocking banner explains why. |
| **Important** | Scanning still works, but a major feature is lost. |
| **Optional** | Only narrows the level of detail. |

Anything not working states **what stops working** and **how to fix it**, with
the exact command for your OS. A badge on the sidebar and a status-bar link
surface problems without opening the page.

These are **functional probes, not `which` checks** — which matters, because a
binary being present says very little:

- On Linux, `ping` can exist but be unable to open an ICMP socket without
  `cap_net_raw`.
- On macOS 14+, the Wi-Fi tools exist but return nothing until Location Services
  permission is granted.
- Inside a container, the ARP table can be readable but permanently empty.

Each check therefore *performs* the operation and reports what really happened.

---

## How it works without privileges

Raw-socket ARP scanning and SYN scanning both need root/Administrator. This app
uses neither:

| Need | Approach |
| --- | --- |
| Host liveness | The system `ping` binary, which is capability-endowed on every supported OS |
| **MAC addresses** | Pinging populates the kernel's ARP/neighbour cache as a side effect; the OS's own tool then reads it back unprivileged |
| Open ports | Full TCP connect scan on ordinary sockets |
| Service identity | mDNS, SSDP and NetBIOS implemented directly on UDP sockets |

A host that ignores ICMP is still found: the TCP phase scans every address in
range, a refused connection proves the host is up, and the neighbour cache is
re-read afterwards to recover MACs the first pass missed.

> **On Windows**, that last signal is weaker. Windows Firewall commonly *drops*
> connections to closed ports rather than rejecting them, which makes a closed
> port indistinguishable from a filtered one. A device that ignores ping *and*
> has none of the scanned ports open may therefore go undetected on Windows,
> where it would be found on Linux or macOS. The Setup & Status page states this
> explicitly when running on Windows.

### Portable by default

mDNS, SSDP, NetBIOS, DNS and the port scanner are implemented in Rust directly on
sockets, so they behave identically everywhere and need **no Avahi, Bonjour,
nmap, or Samba tools installed**. Only four things genuinely differ per OS and
live behind a small platform layer:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| ARP table | `ip neigh` | `arp -an` | IP Helper API (`arp -a` fallback) |
| Routes | `ip route` | `route` / `netstat` | `route print` |
| Resolvers | `resolvectl` | `scutil --dns` | `Get-DnsClientServerAddress` |
| Wi-Fi | `nmcli` | `system_profiler` | `netsh wlan` |

Platform differences that bite are covered by tests — for example BSD `ping`
takes `-W` in **milliseconds** where Linux takes **seconds**, and Windows `ping`
exits 0 even when every reply is "Destination host unreachable".

On Windows the neighbour cache is read through the **IP Helper API** rather than
by parsing `arp -a`, whose output is localised — so it returned nothing on
non-English installations — and which prints only part of the cache. The
hardware address is what yields the vendor, and the vendor is frequently the
only clue what a device is, so this materially changes what gets identified
there. Virtual adapters are matched by their Windows *friendly* names, so
Hyper-V, WSL, VirtualBox, VMware, VPN and tunnel interfaces are no longer taken
for the real network and swept as if they were.

### Off-subnet discovery

mDNS and SSDP run **before** the sweep, because they reveal addresses outside the
local subnet. Any private range they announce is added to the scan
automatically — on the development network this surfaced a Home Assistant
instance on a subnet the host had no route-table knowledge of.

Docker bridges and link-down interfaces are excluded, and any range wider than a
`/22` is refused so a typo cannot queue a 65k-host sweep.

---

## Repeatability

- **Run scan** — live per-phase progress, cancellable.
- **Repeat every 5/15/60 min** — a background supervisor runs scans unattended.
- **History** — every run stored as JSON in the OS app-data directory (last 200).
- **Changes** — appeared / disappeared / IP changed / ports opened / ports closed.
- **Export** — JSON (everything) or CSV (device list) via a native save dialog.

Devices are paired across runs by MAC first, then by IP. Both passes matter:
matching on MAC makes a DHCP lease change read as "same device, new address", and
the IP fallback stops a device whose MAC resolved in only one of the two runs
from being reported as both "gone" and "new". Port deltas are suppressed when the
two runs used different port profiles, since unprobed is not the same as closed.
The first scan is marked as a baseline, so nothing is flagged "new" when there is
nothing to be new against.

---

## Architecture

```
crates/netdiag-core/     Engine. No GUI dependency at all.
  src/scan/              mdns, ssdp, netbios, ports, sweep, banners,
                         connectivity, correlate, dns, http, hostinfo, wifi
  src/platform/          linux.rs · macos.rs · windows.rs
  src/unifi/             Controller client, model, correlation and findings
  src/assist/            Assistant: providers, tool loop, redaction, cases
  src/networks.rs        Network identity, locations, per-network storage
  src/adjacent.rs        Reachable subnets and the evidence for each
  src/doctor.rs          Capability probes
  src/store.rs           Snapshot persistence, pairing and diffing
  src/bin/netdiag-cli.rs Headless entry point
src-tauri/               Desktop shell: window, commands, events, scheduler
app/ components/ lib/    Next.js static export (no server, no API routes)
```

The engine is a **standalone crate with no Tauri dependency**, so it builds and
its 332 tests run on a machine with no desktop toolchain. That is what lets CI
validate the per-OS code paths on all three platforms cheaply, independently of
whether the GUI builds. The assistant reuses the crate's existing TLS stack
rather than adding an HTTP dependency; the only platform dependency is
`windows-sys`, for the IP Helper API.

Scan phase order is load-bearing: `announce` precedes `sweep` so off-subnet
ranges are known before targets are fixed, and `connectivity` follows `sweep` so
latency is measured on an idle link rather than one saturated by our own probes.

---

## Development

Prerequisites: **Node ≥ 20**, **Rust ≥ 1.77**, and the Tauri system
dependencies for your OS
([Tauri prerequisites](https://tauri.app/start/prerequisites/)).

On Ubuntu/Debian:

```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev \
                 libxdo-dev libssl-dev libayatana-appindicator3-dev patchelf
```

```bash
npm install
npm run dev        # run the desktop app with hot reload
npm run build      # produce installers for the current platform

npm run test:core  # engine tests (no GUI toolchain needed)
npm run doctor     # capability report from the terminal
npm run typecheck
npm run lint
```

The headless CLI is useful for debugging the engine without the GUI:

```bash
cargo run -p netdiag-core --bin netdiag-cli -- doctor
cargo run -p netdiag-core --bin netdiag-cli -- scan standard
```

### Refreshing the vendor database

`crates/netdiag-core/data/oui.json` is committed so the app identifies hardware
with no internet access — which matters, since a network diagnostic tool is often
run precisely when the internet is broken. Regenerate it with the IEEE registries
when it ages.

---

## Releasing

CI (`.github/workflows/ci.yml`) runs on every push: `cargo fmt --check`, clippy
with warnings denied, engine tests and the capability doctor on **Linux, macOS
and Windows**, plus frontend typecheck/lint/build and a desktop build on all
three. Run `cargo fmt --all` before pushing — the formatting gate is the one
that most often fails a branch that is otherwise green.

To publish, push a tag:

```bash
git tag v1.0.0 && git push origin v1.0.0
```

`.github/workflows/release.yml` builds Windows, macOS (Apple silicon **and**
Intel) and Linux, uploads every installer to a draft GitHub release, and
publishes it once all platforms succeed. Linux is built on `ubuntu-22.04`
deliberately: glibc is forward- but not backward-compatible, so building on a
newer image would break older distributions.

### Code signing

Builds are unsigned by default and work fine. To sign, add these repository
secrets — the workflow already references them and skips signing when unset:

| Platform | Secrets |
| --- | --- |
| macOS | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` |
| Windows | `TAURI_WINDOWS_CERTIFICATE_THUMBPRINT` |

---

## Safety

- Every external command runs through a wrapper using **argv arrays with no
  shell**, so an IP or CIDR from the UI can never be interpreted as shell syntax.
  All calls are time-boxed.
- Only **private** address ranges are scanned; the deep-scan action refuses
  public addresses.
- The webview has a strict CSP and **no network permissions** — all I/O happens
  in Rust.
- Missing tools never fail a scan: the probe is marked unavailable, a warning is
  recorded, and everything else still runs.
- Nothing leaves the machine unless you connect one: the controller integration
  reads a private address with a credential you supply, and the assistant is off
  until a model is configured. Both are read-only, and the assistant redacts
  identifiers before sending by default.
- Secrets — the controller password and the model API key — live in the
  operating system's keychain, never in a settings file and never in scan
  snapshots, which are exportable.

This tool scans your own network. Do not point it at networks you are not
responsible for.

---

## Not included

No router authentication, so there is no DHCP lease table, no per-client
bandwidth, and no client list from the router itself. The correlation layer takes
identity from independent sources, so router lease data can be merged in later as
one more source.

Path-MTU discovery is also absent: `tracepath` provides it on Linux, but macOS
and Windows have no comparable tool, and a Linux-only field would be misleading.

---

## Contributing

Contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). The engine
crate builds and tests without any GUI toolchain, so you can work on discovery
logic with just Rust installed.

Please also read the [Code of Conduct](CODE_OF_CONDUCT.md).

## Security

Report vulnerabilities privately — see [SECURITY.md](SECURITY.md). Note that
scan snapshots contain MAC addresses, hostnames and open ports for your network;
review them before sharing in a bug report.

## Legal

This tool performs unauthenticated scanning of the network it is attached to.
Use it only on networks you own or are authorised to test. Scanning networks
without permission may be unlawful in your jurisdiction.

## Credits

Hardware vendor names come from the [IEEE MA-L/MA-M/MA-S registries][ieee].
Built with [Tauri](https://tauri.app), [Next.js](https://nextjs.org) and Rust.

[ieee]: https://standards-oui.ieee.org/

## License

[MIT](LICENSE) © 2026 MarkoVcode and contributors
