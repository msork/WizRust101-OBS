# Research and technical decisions

Research recorded 2026-10-06. Sibling sources inspected locally before implementation: WizRust101-RPC ingestion modules/specification, tests and packaged DB integration; WizRust101-DB generator, output schema, audit documents and tests.

## Proven sibling behavior reused

- RPC discovers Steam app `799960` through Steam metadata and `steamlocate`, then checks `Bin/WizardClient.log`; its Windows/Linux roots and library enumeration are adapted directly. The macOS CrossOver bottle discovery module and preference lookup are also reused, though this host cannot live-validate macOS paths.
- RPC's byte-offset tailer buffers incomplete UTF-8/lines and handles file replacement/truncation. Its implementation is reused, including platform-specific file identity handling.
- The parser only emits exact non-empty `zone = ... ,` records and `CHARACTER LIST`; it has no health parser or health event type.
- RPC consumes DB's generated mapping at `out/zones.json`, whose path-to-object values contain `world` and `zone`. This project vendors that output under `vendor/zones.json`, not a competing extraction/resolution engine. The source tree inspected for this snapshot reports DB revision `56c9e4489f0b8f296f5dc016eb7d3579adbf1977`; package work should convert this snapshot to a Git submodule pinned at that revision.
- DB documents exact canonical paths and explicit `Unknown`; this project passes the DB labels through exactly.

## Packaging conventions reviewed

RPC currently packages Windows Setup and portable executables, Linux Flatpak and AppImage builds, and macOS app and pkg archives, with platform-specific artifact verification scripts. This first OBS milestone establishes the native Rust/Browser Source application and discovery only; release workflows should follow those sibling conventions after per-OS OBS and game discovery acceptance.

## HTTP and OBS choice

Axum 0.8's official docs describe HTTP routing and SSE response support; its maintained example binds an SSE app to `127.0.0.1` and serves static assets. Tokio supplies the async listener and broadcast channel. SSE is suitable because overlay clients only receive state. `EventSource` reconnects automatically; on connection this server sends the current snapshot followed by broadcasts. A 15-second keepalive keeps idle streams open.

OBS's official Browser Source page lists Windows, macOS, and Linux (Linux through official packages), supports a URL and width/height, and documents transparent default CSS. This confirms an ordinary localhost Browser Source can satisfy the overlay's requirements without a plugin. Cross-platform actual rendering still needs acceptance against installed OBS/CEF versions.

Sources: [OBS Browser Source](https://obsproject.com/kb/browser-source), [Axum SSE docs](https://docs.rs/axum/latest/axum/response/sse/), [Axum official SSE example](https://github.com/tokio-rs/axum/blob/main/examples/sse/src/main.rs), [`steamlocate` 2.1.1 docs](https://docs.rs/steamlocate/2.1.1/steamlocate/).

## M2 native settings and peer transport

The settings surface is native eframe 0.33.3: its crate documents native Windows, macOS, and Linux support. The eframe app API exposes close-request handling, which allows the application to cancel the close and hide its viewport while retaining a separate Quit action. Native system menus use `tray-icon` on Windows/macOS and Linux StatusNotifierItem via `ksni`; both provide activation/menu hooks. These are OS desktop-shell APIs, not OBS APIs. Linux still depends on a desktop environment with a compatible status notifier host.

Peer presence uses WebSocket as a framed bidirectional transport, with Noise PSK encryption/authentication layered on the messages. This keeps the peer protocol separate from the local SSE read-only stream, avoids depending on TLS certificates for direct peer connections, and gives each invitation a per-peer secret. Noise's PSK mode and the `snow` API are documented by the crate. The listener exposes only the paired `/peer` route and is configured separately from the localhost HTTP server. Maximum configured peers/concurrent sessions is eight; messages are bounded and rate limited.

UPnP uses `igd-next` only when both peer server and the separate port-forward checkbox are enabled. The mapping is TCP 17842 with a one-hour lease, renewed periodically and removed when possible at shutdown/disable. Router discovery and CGNAT behavior vary; manual address/firewall configuration may be required. UPnP does not announce an address to another participant.

Sources: [`eframe` 0.33.3](https://docs.rs/crate/eframe/0.33.3), [`eframe::App` close handling](https://docs.rs/eframe/0.33.3/eframe/trait.App.html), [`tray-icon`](https://docs.rs/tray-icon/latest/tray_icon/), [`ksni::Tray`](https://docs.rs/ksni/latest/ksni/trait.Tray.html), [`snow::Builder`](https://docs.rs/snow/0.10.0/snow/struct.Builder.html), [`igd-next::Gateway`](https://docs.rs/igd-next/0.17.1/igd_next/struct.Gateway.html).

## M3 Pokélink party UX and live model

Pokélink's public theme documentation describes browser themes receiving an initial full player/session list and later `client:party:updated` events. The companion web-source repository documents real-time Socket.IO events and Vue-based roster rendering. The M3 adaptation keeps those useful UX properties—host/join, share/copy invite, a live roster, initial state plus incremental change events—without copying Pokélink code, importing its assets, or adding its account/cloud requirements. Since OBS needs only the existing local SSE view, this app relays party roster snapshots over its authenticated peer channel instead of opening a browser-to-party socket.

The selected topology is one host and up to eight guests (star topology). Every invite has its own random Noise PSK and member ID, so invitations are single-participant and can be revoked by leaving the party. Versioned invites use a compact `WIZPARTY1.` copy/paste code, validate the exact `/peer` address, reject unsupported/legacy shapes, and expire after 24 hours. The client accepts the host's encrypted Welcome then replaces its remote roster from host snapshots, filtering its assigned own member ID. This makes each OBS view perspective-local: each app's own profile stays primary, and the host has no special visual role on clients.

The peer protocol adds directional `Join`, `Welcome`, and full `PartySnapshot` frames. Guests send only their own name/school and log-derived active/world/zone/session fields. The host forwards host presence plus all currently connected guest presences to each client on roster changes and every second. A client connection is considered stale after eight seconds without a roster snapshot; hosts expire idle guest sockets on the same cadence. Application frame size, profile fields, party size, rate, concurrency, duplicate invite use, URL shape, and invite expiry are checked before accepting state.

Addressing is intentionally honest about direct-connect tradeoffs. The default host address is the machine's local IPv4 route address, which works for peers on the same LAN. With UPnP enabled, `igd-next` can report the router's external IP and the host requests a TCP mapping; the crate documents these as separate external-address and mapping operations. Cross-internet reachability remains router/CGNAT-dependent; manual IPv4/hostname + port is available. No rendezvous or cloud relay is introduced.

Sources: [Pokélink theme event documentation](https://github.com/Cysha/pokelink-web/blob/master/themes/template/readme.md), [Pokélink web-source repository](https://github.com/pokelinkapp/pokelink-web-sources/blob/master/README.md), [`igd-next` gateway external IP and mapping API](https://docs.rs/igd-next/0.17.1/igd_next/struct.Gateway.html).

The bundled DB snapshot has SHA-256 `d5fa647d0e1d956d0571141b4ded64e1d22090b0625965fd79440d95099a5eb3`.

## Limits and follow-up

RPC records current client-specific zone syntax and conservative game/session evidence. Zone observation means the process/log was observed in a zone, not that Wizard101 is guaranteed foreground-visible. `CHARACTER LIST` offers a recognized menu marker; lack of a marker does not prove process/session activity. No current quest source is verified. Profile identity is user supplied. Steam paths for Linux Flatpak and macOS CrossOver are not live-validated by this project. The current bundled DB JSON came from the sibling working tree and should be replaced with a recorded exact Git submodule revision before release.
