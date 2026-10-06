# Architecture

```text
WizardClient.log -> discovery/follower/parser -> local session state
  -> WizRust101-DB zone resolver -> localhost Axum HTTP + SSE
  -> transparent HTML/CSS/JS Browser Source in OBS

Native eframe settings window <- system tray/menu bar
  -> local versioned config

Optional paired peer: local presence -> Noise PSK encrypted WebSocket
  -> remote paired peer presence -> smaller party card
```

## Local game and overlay path

The ingestion boundaries follow WizRust101-RPC: Steam discovery, byte-offset incremental following with rotation/truncation handling, conservative event parsing, and exact zone resolution against the generated WizRust101-DB catalog. The DB data is vendored from the sibling's audited output; this app does not duplicate the DB extraction/mapping logic. Unknown zones stay unknown. Session duration starts at a recognized in-world zone and ends on a recognized character-selection marker or other supported stop signal.

Axum and Tokio serve the page and one-way state stream. SSE is appropriate for OBS because the browser only consumes state, EventSource reconnects natively, and browser-to-app control is not needed. `/overlay`, `/style.css`, `/state`, and `/events` are served on `127.0.0.1:17841`; `/settings` and `/api/config` do not exist. Configuration is never exposed over HTTP.

The HTML/CSS/JS uses OBS Browser Source transparency, so there is no native OBS plugin. OBS loads the local URL; the UI itself remains a native eframe window. No remote font, image, script, or account service is requested.

## Native application

The app starts with its eframe settings viewport hidden. Windows and macOS use `tray-icon`; Linux uses the StatusNotifierItem protocol through `ksni`. Tray activation shows/focuses settings. Closing the window hides it; Quit is explicit. Settings are stored as versioned JSON in the per-user config directory. eframe provides a small native cross-platform settings window without a web settings surface.

## Optional peer collaboration

Peer presence has a separate listener on TCP port `17842` and `/peer` WebSocket route. It binds to all interfaces only after the user explicitly enables the peer server. UPnP mapping is a second explicit opt-in, is off by default, and is removed on disable/shutdown where possible. It is never attempted just because the app starts.

The peer channel uses Noise `Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s`: a per-peer 256-bit PSK is shared in an invitation, authenticated as part of the Noise handshake, then encrypts and authenticates every peer payload. Pairing is manually initiated by invite import; the server accepts only configured peer IDs and their secret. No public unauthenticated presence API exists. The route is restricted to paired presence messages, with size/rate/concurrency limits; it cannot serve overlay assets or config. Local OBS delivery remains loopback-only SSE on a separate port.

The invite creator is the listener side and the importer is the connecting side. Presence flows both ways over that connection. The transport does not use TLS; Noise provides peer authentication and payload confidentiality/integrity, while the peer ID/path and network metadata remain observable. Treat invitations and `config.json` as credentials. A reachable host/address and router/firewall forwarding may still be needed. UPnP may fail or be unavailable (including CGNAT environments); the app does not discover or publish a public address.

Each instance constructs its own overlay snapshot from its own active profile and game state, then appends connected remote presences. Thus a user's own wizard stays primary regardless of which instance hosts the listener.
