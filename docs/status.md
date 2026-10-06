# Development status

## M1-M3 vertical slice

Implemented: RPC-derived Steam discovery/follow/parser behavior; WizRust101-DB-backed location resolution; local session state; configurable profiles/schools; loopback Axum overlay HTTP/SSE; transparent primary plaque, zone arrival, and party event transitions; isolated multi-instance CLI with per-instance config/HTTP/peer ports and display names; controllable mock location mode that skips game-log discovery; hidden-start native settings via cross-platform tray; host/join/leave Party UX; versioned expiring copy/paste invites; Noise PSK authenticated/encrypted peer channel; host-authoritative live roster relay; default-off UPnP and advanced manual address/port settings; specs, research, and test coverage.

The peer layer supports one host and up to eight guests. Each invite identifies one guest and can be used by one active connection at a time. Every participant sees their own configured character as the primary card; remote party members are smaller cards.

## Remaining acceptance

Needs real two-or-more-instance LAN/internet testing, router/UPnP/CGNAT testing, OBS Browser Source visual acceptance, and tray/native UI acceptance on Windows, macOS, and Linux desktops. Direct internet hosting needs UPnP or manual router/address setup; no cloud relay is supplied. The Windows MSVC target is the only cross-target available on the development machine. Installers/release workflows are not part of this milestone.

## First UI acceptance follow-up

Updated the native profile wording/actions, added persisted Light/Dark appearance, simplified tray labels, and made Quit shut down HTTP/SSE, log following, Party tasks, UPnP mapping, and tray resources. The plaque default moved above the HUD based on the supplied screenshot. Session duration is no longer displayed. Overlay icons now use the actual RPC catalog and world PNGs, with absent labels omitted. Check the RPC fallback appearance manually: there is no local `wizard101.png` in RPC, so its `wizard101` fallback key currently serves this project's bundled WizRust101 app icon.

Tray event handling was revised so native callbacks enqueue Open/Quit and send only the thread-safe repaint wake signal. The eframe update loop owns viewport changes and Quit. Added a queued-action regression test with 500 Open actions before Quit. School colors now come from one shared JSON palette for native settings and the overlay; manual tray hide/show stress testing is still needed.

The bundled DB JSON keeps its source revision/hash documented in `research.md`; package work should establish a pinned submodule or reproducible refresh process. macOS CrossOver and Linux Flatpak Steam paths remain unvalidated on this host.
