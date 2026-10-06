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

Axum and Tokio serve the overlay at `127.0.0.1:17841`. Routes are `/overlay`, `/style.css`, `/school-palette.json`, `/state`, `/events`, and an allowlisted `/worlds/{asset}` handler. SSE suits OBS because the browser only consumes state and EventSource reconnects natively. The old `/settings` page and config endpoints are removed. Config never travels over HTTP. Application shutdown signals graceful HTTP closure, stops log following and peer tasks, removes an active UPnP mapping, then drops the tray.

The native settings app has one eframe root viewport for its lifetime. Tray callbacks enqueue `Open`/`Quit` actions and request a repaint. `Open` sends `Minimized(false)` followed by `Visible(true)`; the next UI frame sends `Focus`. eframe 0.33.3 documents that `Focus` has no effect while a viewport is minimized or invisible, which explains why the former same-frame `Visible(true)` + `Focus` sequence could leave a minimized/hidden Windows window inert. Close-to-tray still cancels native close and hides the root viewport, so tray activation restores the same window rather than creating another one. Native Windows behavior still requires manual acceptance.

The profile selector writes its selected ID to `active_profile` and persists the config immediately when the config is valid. `SharedState::publish_current` refreshes overlay SSE and host roster subscribers; peer clients read the new local identity on their next presence update.

The transparent HTML/CSS/JS uses OBS Browser Source, so no native OBS plugin is needed. No remote images, fonts, scripts, account service, or streaming-platform API is used.

## Desktop settings

The eframe viewport starts hidden. Windows/macOS use `tray-icon`; Linux uses StatusNotifierItem through `ksni`. Tray callbacks enqueue commands on an unbounded local channel and request a repaint to wake a hidden viewport. They never issue viewport commands, take application locks, or wait for UI work. The eframe update loop alone handles open/focus and quit commands, so either action remains independently serviceable after repeated hide/show cycles. Close hides; Quit shuts down the application and its services. The tray menu labels are exactly **Open Settings** and **Quit WizRust101-OBS** and do not expose ports or instance internals. Light/Dark appearance persists in the versioned local JSON config, which remains backward compatible. Valid edits save locally as they are made; the profile save control is in My Wizard.

School primary/secondary colors live in `static/school-palette.json`. The native settings UI reads that palette through the shared Rust module; the overlay reads the same JSON from the loopback `/school-palette.json` route. This keeps the school colors consistent while leaving text on theme-aware parchment/ink colors for contrast.

Developer CLI options allow multiple independent OS processes: `--data-dir` selects an isolated config file, `--http-port` selects that instance's loopback overlay listener, `--peer-port` selects its Party listener, and `--instance-name` labels its window/tray. The optional `--advertise-host` is useful for same-machine Party tests. `--demo` bypasses all Steam/Wizard101 discovery and exposes editable world/zone controls in native settings; those changes use `SharedState::set_demo_state`, the same local state/SSE/peer-presence path as parsed log events. Each process still runs the real peer server, invite validation, Noise PSK handshake, encryption, host relay, and client reconnection loop. No Party behavior is simulated.

Use unique data directories and ports for each process. Tray identifiers are instance-specific on Linux and tray labels/window titles include the supplied instance name on all platforms. OBS listeners bind only to loopback. For a single-PC test, explicitly set the host invite address to `127.0.0.1` and assign distinct HTTP and peer ports.

## Party topology and perspective

The host-authoritative Party topology supports four total wizards: the host and up to three guests. The host can create single-participant invites; each guest connects directly to the host. Invites may remain available when the party is full, but the peer listener reserves only three guest slots and rejects additional connections with an HTTP 409 `Party is full` response before WebSocket upgrade. An admission lock coordinates active-member IDs and guest-slot permits. Each accepted guest sends only its own configured name/school and log-derived world/zone/session state. The host keeps a unique-member roster and relays a full `PartySnapshot` after join, leave, or presence change and once per second. Clients replace their remote roster from host snapshots, omit their assigned own member ID, and validate at most four total presences / three remote cards.

Client credentials contain a backward-compatible `auto_reconnect` flag that defaults to false. Importing an invite queues one explicit attempt; the supervisor consumes that request only when it starts the attempt. A failed first attempt, including the server's full response, ends without a retry and leaves the flag false, so it is not restored at startup. A peer becomes reconnectable only when the client receives its matching `Welcome` frame over the Noise-encrypted session; that flag is saved locally at that point. The client loop then retries unexpected disconnects, and startup restores only credentials with this established-session flag. The UI merges that flag into its draft before saving so a stale settings view cannot erase it. Removing the peer credential on Leave Party disables the supervisor and prevents reconnect.

Each overlay derives its primary wizard from that app's own active profile and game state. The host has no special presentation role on guests' OBS views: every connected wizard appears only as a smaller party card in other participants' views. The host's own overlay keeps the host as primary.

The primary card uses actual world PNGs and the catalog from `WizRust101-RPC/data/world-assets.json`. Party cards omit world-name text and show only the mapped icon plus a resolved zone under name/school identity. A missing zone removes its text line. An unresolved world omits the world label and uses the RPC catalog's `wizard101` fallback key. RPC defines this key for its Discord activity but ships no matching PNG, so the overlay uses the existing bundled WizRust101 app icon as the fallback image. No replacement world art is drawn. Icon lookup is recomputed during every local SSE enrichment and party snapshot update, so location and icon changes appear without reconnecting. Party change notices refer to the remote wizard's zone only. The overlay does not display session duration.

World art lookup uses the RPC catalog and world PNGs. A test walks every world in the bundled DB catalog and confirms it resolves either to an RPC asset or its declared fallback. Local and party cards use fixed square image viewports. Known worlds retain their text when a zone is absent; absent world/zone values render no placeholder label. The default plaque is placed below the game HUD in the upper-left gameplay area, at x=2% and y=35% (the y value is its lower anchor). Party cards keep fixed icon dimensions and omit zone text when unavailable.

The app uses event-driven full roster snapshots rather than per-member UI commands. This follows the Pokélink web-source pattern of an initial party roster plus party update events, with a simpler local protocol and state model.

## Invite, authentication, and network exposure

Party hosting is off until the user selects **Host Party**. That action explicitly enables a peer-only listener on the configured port (default TCP 17842). Hosting creates a copyable `WIZPARTY1.` invite containing an address, single-member ID, random 256-bit secret, schema version, and issue/expiry timestamps. Invites expire after 24 hours. A guest pastes the code and chooses **Join Party**. Only one active socket can use each invite; create another invite for each additional guest. **Leave Party** clears paired credentials and stops hosting or joining.

Peer WebSockets use Noise `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`; authenticated transport encrypts and integrity-protects all application frames. The native peer route accepts only a configured, unexpired invite ID and bounded `Join` presence payload. The host sends a `Welcome` identity and then `PartySnapshot` updates. Message size, presence field lengths, school values, rate, invite expiry, active peers, and total party size are bounded. A full-party rejection includes a response header so a new guest sees **Party is full** and can explicitly choose **Join Party** again; it does not trigger background attempts. Automatic retry is enabled only after the client has received its authenticated `Welcome`. There is no route on the peer listener for OBS assets, state HTTP, or config.

The overlay server always remains bound to loopback on its own port. The peer listener binds all interfaces only after Host Party. Automatic router setup (UPnP) is an independent explicit option and remains off by default. It maps the selected TCP peer port for a one-hour lease, renews periodically, and removes the mapping when possible. UPnP may be unavailable or blocked by CGNAT. The default invite address is the local IPv4 route address, suitable for the same LAN. With UPnP enabled, the router's reported external address is used. Advanced settings permit a manually reachable IPv4 address/hostname and port for custom router rules. No cloud relay or public IP discovery service is used.

Invites and the local config contain credentials; share invites only with intended participants and keep config private. Noise hides presence payload contents on the wire; destination addresses and connection metadata remain visible. Server binding, UPnP mapping, invite creation, and joining require explicit user action.
