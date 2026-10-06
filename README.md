# WizRust101-OBS

A local Rust application that turns verified Wizard101 log observations into a quiet OBS Browser Source. It works independently of Twitch, YouTube, Kick, and streaming accounts.

## Start and preview

Run `cargo run` to start the game-log watcher, local overlay server, and tray/menu-bar app. Settings are hidden initially; activate the WizRust101-OBS tray icon to open them. Close hides the window. Choose **Quit WizRust101-OBS** to exit. Linux needs a desktop StatusNotifierItem/AppIndicator host.

Run `cargo run -- --demo` to use controllable mock game state without launching Wizard101 or discovering game logs. The native Overlay tab lets you edit the mock world/zone and apply it or end the mock session. Choose a saved profile under **My Wizard**; profile identity stays configured independently of the mock location.

## Isolated local Party test instances (Windows PowerShell)

Each process needs its own data directory and ports. Mock instances skip Wizard101/Steam log discovery entirely, but still use the real authenticated Party protocol. Start these in separate PowerShell windows from the repository root:

```powershell
cargo run -- --data-dir "$PWD\.dev-party\A" --http-port 17841 --peer-port 17842 --instance-name A --advertise-host 127.0.0.1
```

Instance A uses real Wizard101 log state. To mock A instead, add `--demo --demo-world "Wizard City" --demo-zone "The Commons"`.

```powershell
cargo run -- --data-dir "$PWD\.dev-party\B" --http-port 17843 --peer-port 17844 --instance-name B --demo --demo-world "Krokotopia" --demo-zone "The Oasis"
```

Optional third instance:

```powershell
cargo run -- --data-dir "$PWD\.dev-party\C" --http-port 17845 --peer-port 17846 --instance-name C --demo --demo-world "Celestia" --demo-zone "Survey Camp"
```

Open the matching tray settings window for each instance, create a different profile in each, and select it as active. On A, choose **Host Party** and copy an invite. Paste it into B and choose **Join Party**. For C, have A create another invite and paste that one into C. For this same-machine test, A advertises `127.0.0.1`; UPnP stays off.

Open each overlay in a browser or OBS Browser Source:

- A: `http://127.0.0.1:17841/overlay`
- B: `http://127.0.0.1:17843/overlay`
- C: `http://127.0.0.1:17845/overlay`

Each view must keep its own wizard prominent and render connected peers as smaller cards. On B/C, edit **Mock Wizard Location** and click **Apply location**; the other connected views should update without reconnecting. **End mock session** tests offline presence. **Leave Party** tests leave/rejoin. To test reconnect, quit and relaunch B with the same command and data directory; its saved invite should reconnect automatically. `cargo run -- --help` lists the available instance options.

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
