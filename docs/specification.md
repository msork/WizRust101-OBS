# Product specification

## Purpose

WizRust101-OBS is a local, cross-platform OBS overlay that should feel like Wizard101 itself added streaming support. Its UI uses original parchment, antique brass, restrained magical detailing, readable serif type, and school colors. It keeps gameplay visible and avoids repeating the game's own HUD.

## Data contract

Automatic data is limited to evidence in `WizardClient.log`: canonical zone changes, mapped world and player-facing zone from WizRust101-DB, a best-effort observed in-world session state, and locally elapsed session duration. `CHARACTER LIST` or a missing/replaced log can indicate that an observed session ended; if the client exits without a marker, stop detection is not guaranteed. Unknown locations remain unknown.

Character name and school come from the user's selected saved profile. They are never claimed to be detected from the log. School is one of Fire, Ice, Storm, Myth, Life, Death, or Balance. Profile metadata is forward-compatible for future manually entered fields.

Current quest/objective tracking is unsupported because no reliable selected-quest evidence from the log has been verified. Health, mana, deck/spells, combat UI, and other game-visible HUD values are intentionally omitted. The app does not read process memory, inject, intercept packets, OCR, or modify Wizard101.

## Desktop settings and overlay

The app runs primarily in the system tray/menu bar. Settings are hidden at startup. Clicking/activating the tray icon opens the native settings window; closing that window hides it. Quit is separate. Users can select/manage profiles, set the active wizard, independently toggle the persistent location plaque and zone reveal, configure position/scale/opacity/reveal duration, manage optional peer invitations, and copy the OBS URL.

OBS Browser Source URL: `http://127.0.0.1:17841/overlay`; recommended canvas: 1920x1080. Position is percentage-based and scales with other canvas sizes. The page has a transparent background. The compact primary wizard/location plaque remains the streamer's own configured character. Connected participants appear only as smaller, clearly labeled party cards.

The local HTTP surface is `/overlay`, `/style.css`, `/state`, and `/events`. `/settings` and `/api/config` are removed. SSE delivers state to OBS; it does not accept commands.

## Optional peer presence

Multiplayer/collab presence is opt-in. Enabling the peer server explicitly opens the peer-only listener on port 17842. UPnP port forwarding is a separate option, disabled by default; no listener or port mapping is enabled automatically. Pairing uses a manually shared invite and a per-peer 256-bit Noise PSK. Connected instances exchange only configured wizard name/school and automatic world/zone/session presence. Each client renders its own wizard as primary and the remote wizard as a smaller party card.

Peer sharing is independent of Twitch, YouTube, Kick, or any streaming-platform API. The peer listener does not expose OBS, settings, or configuration routes.

## Privacy and support

No cloud service, analytics, or streaming account is required. Logs/config remain on-device except for the deliberately shared presence sent to a paired peer. Invitations and config contain secrets and must be treated as credentials. The app targets Windows, Linux, and macOS; real-machine acceptance on every target remains necessary. Steam discovery is the supported source for v1.
