# Test fonts

Deterministic text-backend fixtures for the `typography` module's unit
tests and the preview example's `--fixture-font` mode. Never installed:
`airlock install` places `res/icons` and the desktop entry only — nothing
here reaches a user's system, and the crate package carries the directory
solely so packaged tests can reference it.

| File | Provenance |
|------|------------|
| `OpenSans-Regular.ttf` | Debian `fonts-open-sans` 1.11-2 (upstream <https://www.opensans.com/>); digitized data copyright © 2010–2011 Google Corporation. Same family as COSMIC's own default interface font, so the fixture db shapes the face the default follow names. |
| `LICENSE-Apache-2.0.txt` | The font's license (Apache-2.0, DFSG-free, GPLv3-compatible). |

Tests load this directory into a fresh font database — never the system
scan — so a test result never depends on the fonts a host happens to have.
