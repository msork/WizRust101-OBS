# Development status

## M1-M3 vertical slice

Implemented: RPC-derived Steam discovery/follow/parser behavior; WizRust101-DB-backed location resolution; local session state; configurable profiles/schools; loopback Axum overlay HTTP/SSE; transparent primary plaque, zone arrival, and party event transitions; isolated multi-instance CLI with per-instance config/HTTP/peer ports and display names; controllable mock location mode that skips game-log discovery; hidden-start native settings via cross-platform tray; host/join/leave Party UX; versioned expiring copy/paste invites; Noise PSK authenticated/encrypted peer channel; host-authoritative live roster relay; default-off UPnP and advanced manual address/port settings; specs, research, and test coverage.

The peer layer supports one host and up to eight guests. Each invite identifies one guest and can be used by one active connection at a time. Every participant sees their own configured character as the primary card; remote party members are smaller cards.

## Remaining acceptance

Needs real two-or-more-instance LAN/internet testing, router/UPnP/CGNAT testing, OBS Browser Source visual acceptance, and tray/native UI acceptance on Windows, macOS, and Linux desktops. Direct internet hosting needs UPnP or manual router/address setup; no cloud relay is supplied. The Windows MSVC target is the only cross-target available on the development machine. Installers/release workflows are not part of this milestone.

The bundled DB JSON keeps its source revision/hash documented in `research.md`; package work should establish a pinned submodule or reproducible refresh process. macOS CrossOver and Linux Flatpak Steam paths remain unvalidated on this host.
