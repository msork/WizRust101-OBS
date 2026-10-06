# Development status

## M1 and M2 vertical slice

Implemented: RPC-derived Steam discovery/follow/parser behavior; WizRust101-DB-backed world/zone resolution; local session state; configurable profiles and school validation; loopback Axum overlay HTTP/SSE; transparent persistent character/location plaque and animated zone arrival; demo mode; native eframe settings UI hidden at startup; Windows/macOS tray and Linux StatusNotifierItem; hide-on-close and explicit Quit; optional Noise-PSK peer presence with pairing and opt-in UPnP forwarding; documentation and tests.

The local server exposes only `/overlay`, `/style.css`, `/state`, and `/events`; there is no settings page or config endpoint. Peer and overlay listeners use distinct ports and route sets.

## Remaining acceptance

Needs runtime visual/interaction acceptance with OBS Browser Source (including CEF rendering), tray shell acceptance on Windows/macOS and a Linux desktop with StatusNotifierItem support, live game/log discovery acceptance on each OS, and network acceptance for two real paired clients behind representative routers. UPnP behavior is inherently router-dependent. Release installers/workflows are not part of this milestone.

The bundled DB JSON came from the sibling working tree and retains its recorded source revision/hash in `research.md`; package work should establish a pinned submodule or reproducible refresh process. macOS CrossOver and Linux Flatpak Steam paths have not been live-validated on this host.
