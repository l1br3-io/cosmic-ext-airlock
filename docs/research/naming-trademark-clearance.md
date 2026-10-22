# Airlock — naming and trademark clearance

**Date:** October 22, 2026 · **Method:** brand-naming session followed by
availability checks against GitHub (repository search), crates.io (API),
Flathub (search), and the System76 COSMIC Trademark Policy. This record is
closed; corrections go in a new record, not edits here.

## Decision

| Slot | Value |
|------|-------|
| Display name | **Airlock** |
| Package / repo / crate | `cosmic-ext-airlock` |
| App ID | `io.github.<username>.Airlock` (outside the official `com.system76.` prefix) |
| Description wording | "… **for the COSMIC™ desktop**" |
| Tagline | "Nothing crosses workspaces." |

## Why Airlock

A hybrid, space-native metaphor for isolation: an airlock is the spacecraft
chamber that keeps environments sealed from each other — nothing crosses
workspaces. Short, spellable, logo-ready (circular hatch).

Finalists considered (from a 16-name evaluation): Airlock, Umbra, Asterism,
Karman, Occultation. Airlock won on clarity + memory + space-native function
tie. Runner-ups: Asterism (zero-collision pick if Airlock ever conflicts),
Umbra (moodiest, but a satellite-imaging company wears it).

## Availability verdicts (checked 2026-10-22)

- **GitHub**: repository search for "airlock" + COSMIC → **0 results**.
  `cosmic-ext-airlock` unclaimed.
- **crates.io** ([API](https://crates.io/api/v1/crates/airlock)):
  - `airlock` — **taken** by an unrelated `no_std async communication`
    library (airlock-rs), dormant at 0.0.8 since 2023-07. No category conflict.
  - `cosmic-airlock` — **free** (404), but not policy-compliant (below).
  - `cosmic-ext-airlock` — **free** (404). Chosen.
- **Flathub**: no app matches "airlock".
- **Other products named Airlock**: Ergon AIRLOCK (enterprise WAF),
  "AirLock OS" by Cosmic Construct (manufacturing software). Different
  categories; the word is generic space hardware vocabulary.

## COSMIC trademark policy (the load-bearing finding)

The official [cosmic-app-template README](https://github.com/pop-os/cosmic-app-template)
requires checking the
[COSMIC Trademark Policy](https://github.com/pop-os/cosmic-epoch/blob/master/TRADEMARK.md).
Key rules:

- The `cosmic-` package namespace and `com.system76.` App IDs are reserved
  for **official** System76 software only.
- Third-party projects must not imply endorsement by System76.
- Precedent: [cosmic-ext-kickoff-launcher](https://github.com/justinkelly/cosmic-ext-kickoff-launcher/blob/main/LICENSES/System76-COSMIC-TRADEMARK.md)
  asked System76 and was told to use the `cosmic-ext-` package prefix, an App
  ID outside `com.system76.`, and the wording "for the COSMIC™ desktop".

Consequences for this project: use `cosmic-ext-airlock` (never `cosmic-airlock`),
App ID outside `com.system76.`, description says "for the COSMIC™ desktop",
and the project states it is not affiliated with or endorsed by System76.
Optional: email trademark@system76.com for written confirmation, as Kickoff did.

## Sources

1. COSMIC Trademark Policy — https://github.com/pop-os/cosmic-epoch/blob/master/TRADEMARK.md
2. cosmic-app-template README (points at the policy) — https://github.com/pop-os/cosmic-app-template
3. Kickoff trademark clearance — https://github.com/justinkelly/cosmic-ext-kickoff-launcher/blob/main/LICENSES/System76-COSMIC-TRADEMARK.md
4. crates.io `airlock` — https://crates.io/api/v1/crates/airlock
5. GitHub repo search `airlock cosmic` — https://api.github.com/search/repositories?q=airlock+cosmic
