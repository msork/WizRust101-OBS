# WizRust101-OBS

A local Rust app that turns verified Wizard101 log observations into a quiet, transparent OBS Browser Source. It is independent of Twitch, YouTube, Kick, and streaming-platform accounts.

## Start

Run `cargo run` to start the game-log watcher, local overlay server, and tray/menu-bar app. Its settings window starts hidden. Open it from the WizRust101-OBS tray icon/menu; closing the window hides it. Choose **Quit WizRust101-OBS** to exit. On Linux, use a desktop session with StatusNotifierItem/AppIndicator support.

Run `cargo run -- --demo` to preview sample profiles, locations, zone transitions, and session start/stop without launching Wizard101. Configure a saved profile and select it as the primary wizard in **My Wizard**.

## Add the overlay to OBS

In the native **Overlay** settings tab, click **Copy overlay URL**. In OBS:

1. Add a **Browser** source.
2. Leave **Local file** unchecked.
3. Paste `http://127.0.0.1:17841/overlay` as the URL.
4. Set width to `1920` and height to `1080` (or use your 16:9 canvas dimensions).
5. Keep WizRust101-OBS running while OBS uses the source.

The source document is transparent. The persistent plaque shows your manually selected wizard plus the automatically observed world and zone. Zone changes trigger a restrained temporary reveal. The panel placement, scale, opacity, transition duration, and both elements can be adjusted in settings.

## Optional party presence

Both participants need WizRust101-OBS. The listener must explicitly enable **Enable my peer server** and save settings. **Request UPnP port forwarding** is a separate opt-in and defaults off. Create an invitation, share its JSON privately, and import it on the connecting instance. Invitations and `config.json` contain pairing secrets; treat them as credentials. Each overlay keeps its own wizard prominent; connected peers appear as smaller party cards.

Peer presence uses a separate authenticated/encrypted Noise PSK WebSocket on TCP port 17842. The peer listener does not serve the OBS page or configuration. A reachable LAN/public host and firewall/router configuration may be required; UPnP is not available on every network and does not work around all CGNAT setups. No port is opened or forwarded automatically.

## Data and deliberate omissions

Automatic information is limited to canonical zone changes parsed from `WizardClient.log`, world/zone names resolved through the WizRust101-DB mapping, best-effort active session state, and a local session duration. Name and school are user configured. The log does not reliably identify the selected character.

Health, mana, deck/spells, combat, and other game-visible HUD values are intentionally omitted. Current quest/objective tracking is unsupported because no reliable selected-quest evidence has been verified in the log. The app does not inject, read memory, intercept packets, OCR, or modify Wizard101.

## Development

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [specification](docs/specification.md), [architecture](docs/architecture.md), [research and decisions](docs/research.md), and [development status](docs/status.md). MIT licensed.
