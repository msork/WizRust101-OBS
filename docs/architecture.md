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

Axum and Tokio serve the overlay at `127.0.0.1:17841`. The only routes are `/overlay`, `/style.css`, `/state`, and `/events`. SSE suits OBS because the browser only consumes state and EventSource reconnects natively. The old `/settings` page and config endpoints are removed. Config never travels over HTTP.

The transparent HTML/CSS/JS uses OBS Browser Source, so no native OBS plugin is needed. No remote images, fonts, scripts, account service, or streaming-platform API is used.

## Desktop settings

The eframe viewport starts hidden. Windows/macOS use `tray-icon`; Linux uses StatusNotifierItem through `ksni`. Tray activation shows settings; close hides; Quit exits. The settings window provides local profile/overlay controls and the friendly Party flow. Versioned JSON remains in the per-user config directory.

## Party topology and perspective

The first host-authoritative v1 topology supports one host and up to eight guests. The host issues one single-participant invite at a time; each guest connects directly to the host. A guest sends only its own configured name/school and log-derived world/zone/session state. The host keeps the current guest roster and relays a full `PartySnapshot` after join, leave, or presence change and once per second. Clients replace their remote roster from the host snapshot and omit their own assigned member ID.

Each overlay derives its primary wizard from that app's own active profile and game state. The host has no special presentation role on guests' OBS views: every connected wizard appears only as a smaller party card in other participants' views. The host's own overlay keeps the host as primary.

The primary card presents DB-resolved world name and zone beside an original inline vector sigil selected from the resolved world label. Party cards omit world-name text and show only that sigil plus the zone under name/school identity. Unknown and unmapped labels select a built-in fallback. Icon selection is recomputed during every local SSE enrichment and party snapshot update, so location and icon changes appear without reconnecting. Party change notices refer to the remote wizard's zone only.

World art selection is centralized in `presentation::world_icon`; the bundled SVG symbols are original geometric designs keyed by those stable IDs. A coverage test walks every world in the bundled DB catalog so a newly resolved world cannot silently fall through to the unknown mark. Local and party sigils use fixed square viewports and shared stroke/crop rules. Cards reserve stable location heights and ellipsize long labels, avoiding size jumps when zone/world text changes. The layout uses bounded viewport-relative sizing: at 1920x1080 it lays out two columns of compact party cards beside the local plaque; at 2560x1440 the available party width permits three columns. The party grid is capped at the supported eight guests.

The app uses event-driven full roster snapshots rather than per-member UI commands. This follows the Pokélink web-source pattern of an initial party roster plus party update events, with a simpler local protocol and state model.

## Invite, authentication, and network exposure

Party hosting is off until the user selects **Host Party**. That action explicitly enables a peer-only listener on the configured port (default TCP 17842). Hosting creates a copyable `WIZPARTY1.` invite containing an address, single-member ID, random 256-bit secret, schema version, and issue/expiry timestamps. Invites expire after 24 hours. A guest pastes the code and chooses **Join Party**. Only one active socket can use each invite; create another invite for each additional guest. **Leave Party** clears paired credentials and stops hosting or joining.

Peer WebSockets use Noise `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`; authenticated transport encrypts and integrity-protects all application frames. The native peer route accepts only a configured, unexpired invite ID and bounded `Join` presence payload. The host sends a `Welcome` identity and then `PartySnapshot` updates. Message size, presence field lengths, school values, rate, invite expiry, active peers, and total party size are bounded. There is no route on the peer listener for OBS assets, state HTTP, or config.

The overlay server always remains bound to loopback on its own port. The peer listener binds all interfaces only after Host Party. Automatic router setup (UPnP) is an independent explicit option and remains off by default. It maps the selected TCP peer port for a one-hour lease, renews periodically, and removes the mapping when possible. UPnP may be unavailable or blocked by CGNAT. The default invite address is the local IPv4 route address, suitable for the same LAN. With UPnP enabled, the router's reported external address is used. Advanced settings permit a manually reachable IPv4 address/hostname and port for custom router rules. No cloud relay or public IP discovery service is used.

Invites and the local config contain credentials; share invites only with intended participants and keep config private. Noise hides presence payload contents on the wire; destination addresses and connection metadata remain visible. Server binding, UPnP mapping, invite creation, and joining require explicit user action.
