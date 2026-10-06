# WizRust101-OBS

A local Rust application that turns verified Wizard101 log observations into a quiet OBS Browser Source. It works independently of Twitch, YouTube, Kick, and streaming accounts.

## Start and preview

Run `cargo run` to start the game-log watcher, local overlay server, and tray/menu-bar app. Settings are hidden initially; activate the WizRust101-OBS tray icon to open them. Close hides the window. Choose **Quit WizRust101-OBS** to exit. Linux needs a desktop StatusNotifierItem/AppIndicator host.

Run `cargo run -- --demo` to preview sample characters, worlds/zones, transitions, and session start/stop without launching Wizard101. Create a saved profile and select it as your primary wizard under **My Wizard**.

## OBS Browser Source

In **Overlay** settings, click **Copy overlay URL**. In OBS:

1. Add a **Browser** source.
2. Leave **Local file** unchecked.
3. Paste `http://127.0.0.1:17841/overlay` as the URL.
4. Set width to `1920` and height to `1080` (or your 16:9 canvas size).
5. Keep WizRust101-OBS running while the source is in use.

The transparent page displays the manually selected wizard and automatically observed location. Zone arrivals and party events use short restrained notices. Placement, scale, opacity, transition duration, and both overlay elements can be changed in settings.

## Host or join a Party

1. Set your active wizard profile first.
2. In **Party**, choose **Host Party**. This explicitly starts your peer listener and creates an invite.
3. Click **Copy Invite** and privately share the code. Each invite is for one guest and expires after 24 hours; create another invite for each additional participant.
4. Guests paste the code and click **Join Party**. Use **Leave Party** to disconnect and remove party credentials.

The party roster updates when a wizard joins/leaves or changes observed location. Each participant's own wizard remains the large primary element on their OBS scene; other party members are smaller cards. The peer channel is encrypted and authenticated. The local OBS page/config are not exposed to peers.

The first invite uses a detected local IPv4 address for same-LAN play. For internet play, the optional **Automatic router setup** can request UPnP forwarding and use the router's reported public address. It is off by default. Some routers or CGNAT networks cannot provide direct reachability. **Advanced address and port** allows manual settings for a router rule. No cloud relay is used.

## Automatic data and deliberate omissions

Automatic information is limited to canonical zone changes parsed from `WizardClient.log`, world/zone names resolved through WizRust101-DB, best-effort observed in-world state, and a local session duration. Character name and school are configured by the user; the log does not reliably expose selected identity.

Health, mana, deck/spells, combat, and other game-visible HUD information are omitted. Current quest/objective tracking is unsupported because reliable selected-quest evidence has not been verified. The app does not inject, read memory, intercept packets, OCR, or modify Wizard101.

## Development

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

See [specification](docs/specification.md), [architecture](docs/architecture.md), [research and decisions](docs/research.md), and [development status](docs/status.md). MIT licensed.
