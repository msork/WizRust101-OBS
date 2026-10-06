# Architecture

```text
WizardClient.log -> discovery/follower/parser -> local session state
  -> WizRust101-DB zone resolver -> localhost Axum HTTP + SSE
  -> transparent Browser Source in OBS

Native eframe settings <- tray/menu bar
  -> Party host / join / leave and local versioned config

Party host -> Noise PSK WebSocket -> joined clients send only own presence
  <- host relays authenticated party roster -> each client's local SSE view
```

## Local game and overlay

Ingestion boundaries follow WizRust101-RPC: Steam discovery, byte-offset incremental following with rotation/truncation handling, conservative event parsing, and exact zone resolution against generated WizRust101-DB data. DB output is vendored from its audited snapshot; this app does not duplicate DB extraction or mapping rules. Unknown zones stay unknown. Session duration starts at a recognized in-world zone and stops on recognized character-selection evidence.

Axum and Tokio serve the overlay at `127.0.0.1:17841`. Routes are `/overlay`, `/style.css`, `/state`, `/events`, and an allowlisted `/worlds/{asset}` handler. SSE suits OBS because the browser only consumes state and EventSource reconnects natively. The old `/settings` page and config endpoints are removed. Config never travels over HTTP. Application shutdown signals graceful HTTP closure, stops log following and peer tasks, removes an active UPnP mapping, then drops the tray.

The transparent HTML/CSS/JS uses OBS Browser Source, so no native OBS plugin is needed. No remote images, fonts, scripts, account service, or streaming-platform API is used.

## Desktop settings

The eframe viewport starts hidden. Windows/macOS use `tray-icon`; Linux uses StatusNotifierItem through `ksni`. Tray activation shows settings; close hides; Quit shuts down the application and its services. The tray menu labels are exactly **Open Settings** and **Quit WizRust101-OBS** and do not expose ports or instance internals. Light/Dark appearance persists in the versioned local JSON config, which remains backward compatible. Valid edits save locally as they are made; the profile save control is in My Wizard.

Developer CLI options allow multiple independent OS processes: `--data-dir` selects an isolated config file, `--http-port` selects that instance's loopback overlay listener, `--peer-port` selects its Party listener, and `--instance-name` labels its window/tray. The optional `--advertise-host` is useful for same-machine Party tests. `--demo` bypasses all Steam/Wizard101 discovery and exposes editable world/zone controls in native settings; those changes use `SharedState::set_demo_state`, the same local state/SSE/peer-presence path as parsed log events. Each process still runs the real peer server, invite validation, Noise PSK handshake, encryption, host relay, and client reconnection loop. No Party behavior is simulated.

Use unique data directories and ports for each process. Tray identifiers are instance-specific on Linux and tray labels/window titles include the supplied instance name on all platforms. OBS listeners bind only to loopback. For a single-PC test, explicitly set the host invite address to `127.0.0.1` and assign distinct HTTP and peer ports.

## Party topology and perspective

The first host-authoritative v1 topology supports one host and up to eight guests. The host issues one single-participant invite at a time; each guest connects directly to the host. A guest sends only its own configured name/school and log-derived world/zone/session state. The host keeps the current guest roster and relays a full `PartySnapshot` after join, leave, or presence change and once per second. Clients replace their remote roster from the host snapshot and omit their own assigned member ID.

Each overlay derives its primary wizard from that app's own active profile and game state. The host has no special presentation role on guests' OBS views: every connected wizard appears only as a smaller party card in other participants' views. The host's own overlay keeps the host as primary.

The primary card uses actual world PNGs and the catalog from `WizRust101-RPC/data/world-assets.json`. Party cards omit world-name text and show only the mapped icon plus a resolved zone under name/school identity. A missing zone removes its text line. An unresolved world omits the world label and uses the RPC catalog's `wizard101` fallback key. RPC defines this key for its Discord activity but ships no matching PNG, so the overlay uses the existing bundled WizRust101 app icon as the fallback image. No replacement world art is drawn. Icon lookup is recomputed during every local SSE enrichment and party snapshot update, so location and icon changes appear without reconnecting. Party change notices refer to the remote wizard's zone only. The overlay does not display session duration.

World art lookup uses the RPC catalog and world PNGs. A test walks every world in the bundled DB catalog and confirms it resolves either to an RPC asset or its declared fallback. Local and party cards use fixed square image viewports. Known worlds retain their text when a zone is absent; absent world/zone values render no placeholder label. The default plaque is placed below the game HUD in the upper-left gameplay area, at x=2% and y=35% (the y value is its lower anchor). Party cards keep fixed icon dimensions and omit zone text when unavailable.

The app uses event-driven full roster snapshots rather than per-member UI commands. This follows the Pokélink web-source pattern of an initial party roster plus party update events, with a simpler local protocol and state model.

## Invite, authentication, and network exposure

Party hosting is off until the user selects **Host Party**. That action explicitly enables a peer-only listener on the configured port (default TCP 17842). Hosting creates a copyable `WIZPARTY1.` invite containing an address, single-member ID, random 256-bit secret, schema version, and issue/expiry timestamps. Invites expire after 24 hours. A guest pastes the code and chooses **Join Party**. Only one active socket can use each invite; create another invite for each additional guest. **Leave Party** clears paired credentials and stops hosting or joining.

Peer WebSockets use Noise `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`; authenticated transport encrypts and integrity-protects all application frames. The native peer route accepts only a configured, unexpired invite ID and bounded `Join` presence payload. The host sends a `Welcome` identity and then `PartySnapshot` updates. Message size, presence field lengths, school values, rate, invite expiry, active peers, and total party size are bounded. There is no route on the peer listener for OBS assets, state HTTP, or config.

The overlay server always remains bound to loopback on its own port. The peer listener binds all interfaces only after Host Party. Automatic router setup (UPnP) is an independent explicit option and remains off by default. It maps the selected TCP peer port for a one-hour lease, renews periodically, and removes the mapping when possible. UPnP may be unavailable or blocked by CGNAT. The default invite address is the local IPv4 route address, suitable for the same LAN. With UPnP enabled, the router's reported external address is used. Advanced settings permit a manually reachable IPv4 address/hostname and port for custom router rules. No cloud relay or public IP discovery service is used.

Invites and the local config contain credentials; share invites only with intended participants and keep config private. Noise hides presence payload contents on the wire; destination addresses and connection metadata remain visible. Server binding, UPnP mapping, invite creation, and joining require explicit user action.
