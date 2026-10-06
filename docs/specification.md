# Product specification

## Purpose and data boundary

WizRust101-OBS is a local cross-platform streaming overlay designed to feel as if Wizard101 added streaming support. It uses original parchment, brass, restrained magical ornament, readable serif type, and school colors while leaving game UI visible.

Automatic data is limited to evidence in `WizardClient.log`: canonical zone changes, world/zone labels from WizRust101-DB, best-effort in-world state, and local session duration. Character name and school always come from the user's active saved profile. They are never reported as log-detected. Current quests/objectives are unsupported because no reliable selected-quest evidence has been verified. Health, mana, deck/spells, combat, and other game-visible information are omitted. The app does not inspect process memory, inject, intercept packets, OCR, or modify Wizard101.

## Desktop settings and OBS

The app starts in the tray/menu bar with settings hidden. Tray activation shows settings, closing hides them, and Quit exits the full application cleanly. The tray menu contains only **Open Settings** and **Quit WizRust101-OBS**. The native settings UI includes persistent Light and Dark themes, profile management, school selection, overlay toggles, position, scale, opacity, transition duration, and Copy Overlay URL. Valid edits save locally as they are made; **Save Profile** is in My Wizard above **Remove Profile**.

OBS Browser Source URL: `http://127.0.0.1:17841/overlay`; recommended canvas 1920x1080. The page is transparent and percentage-positioned. Its default plaque sits in the upper-left gameplay area below Wizard101's corner controls. `/overlay`, `/style.css`, `/state`, `/events`, and the read-only `/worlds/{asset}` icon route are local HTTP routes. `/settings` and `/api/config` do not exist. Session duration is not rendered on the overlay.

## Party experience

The Party tab offers **Host Party**, **Join Party**, **Leave Party**, **Copy Invite**, a connected wizard list with presence/location status, optional automatic router setup, and advanced manual address/port fallback. Hosting is disabled until the user selects it. UPnP is off by default. Host generates a copyable invite code; guest pastes it and joins. Each code supports one guest at a time and expires after 24 hours; hosts create another invite for each additional guest (up to eight connected guests). Leave Party disconnects and removes saved party credentials.

Each running app contributes its own manually configured wizard identity and automatic world/zone/session presence. The host relays live roster snapshots when members join, leave, or change presence. Each app always renders its own wizard as the large primary card. World images use the exact `world-assets.json` mapping and PNG artwork from WizRust101-RPC. The primary card shows the actual mapped icon, world name, and zone when resolved. Party cards show name, school, world icon, and zone only. The world name is never repeated for party members. When a world is unresolved, the RPC catalog's `wizard101` fallback is used without a world label. When a zone is unresolved, its line is omitted. The RPC source has no local PNG for its `wizard101` key, so this app uses the already-bundled WizRust101 app icon as that fallback. Fixed icon dimensions keep the layout stable while images change. Party join, leave, and location changes produce restrained temporary notices. The grid is bounded for eight guests and scales within 16:9 OBS canvases.

## Security, network, and privacy

Invites use a versioned `WIZPARTY1.` code, per-member random secret, host address/port, and expiry. Peer WebSocket traffic is authenticated and encrypted with Noise PSK. Only configured, unexpired peer IDs can establish a connection. The peer listener is separate from loopback-only OBS HTTP/SSE and offers no OBS/config route. No port or router mapping is exposed automatically. UPnP is a distinct opt-in and does not work around all CGNAT or router restrictions. Local-network invites work when participants can reach one another on that LAN; internet reachability may need UPnP or manual router/address setup.

Presence and local config remain on-device except for the deliberately shared party presence. Do not share invite codes outside the intended party or publish config files. The peer destination address and network metadata are visible to network observers; Noise encrypts the peer application frames. No cloud relay, analytics, streaming account, or Twitch/YouTube/Kick API is required.

Windows, Linux, and macOS are supported targets. Linux tray operation requires a desktop StatusNotifierItem host. Native UI, tray, network, and OBS/CEF acceptance on each operating system remains a release task.

## Isolated developer instances

For local Party testing, each process accepts `--data-dir`, `--http-port`, `--peer-port`, and `--instance-name`. The independent data directory stores only that instance's `config.json`, including its own profiles and invite credentials. Mock mode (`--demo`, optionally initialized with `--demo-world` and `--demo-zone`) does not run game-log discovery. Native mock controls set location/session state through the normal shared-state pipeline. Peer networking and authentication remain fully real. `--advertise-host 127.0.0.1` makes same-machine invites connect over loopback. See the README for copy/paste Windows PowerShell commands for two or three instances.
