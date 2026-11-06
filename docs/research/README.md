# Research index

Status lives here; the records themselves are never edited after the fact.
Corrections or follow-ups get a new record and a new row.

| Date | Record | Status |
|------|--------|--------|
| 2026-10-22 | [Alt+Tab workspace isolation](alt-tab-workspace-isolation.md) | Complete — conclusions adopted by the Airlock spec |
| 2026-10-22 | [Naming and trademark clearance](naming-trademark-clearance.md) | Complete — Airlock / `cosmic-ext-airlock` locked in |
| 2026-10-24 | [VM environment for COSMIC testing](cosmic-vm-environment.md) | Complete — virt-manager + KVM + virtio-gpu/virgl recipe for the CP4 rehearsal gate |
| 2026-10-31 | [Switcher behavior adaptation](switcher-behavior-adaptation.md) | Complete — verdicts landed: Enter/arrows (`gesture-keys`) and same-app/quick-toggle (`same-app`) built; §4's wheel-defer verdict superseded by `pointer-select` (its sole blocker, zero pointer plumbing, is gone); type-to-filter rejected |
| 2026-11-01 | [Switcher UX and icons](switcher-ux-and-icons.md) | Complete — landed in `overlay-visuals` (module complete 2026-11-02, CP-OV passed): pop-launcher icon pipeline with SVG-first (local evidence: Pop/Cosmic themes are SVG-only), 32 px row icons, ≤600 px card, fg@20% selection fill, no fade-out (kept zero mapped-time rule); thumbnails confirmed out of reach for external Wayland clients |
| 2026-11-03 | [What's next after v0.1.0](whats-next-post-v0.1.0.md) | Complete — roadmap verdicts: finish the release first (hardware pass on HEAD, re-cut the stale v0.1.0 tag + GitHub Release, `cargo publish` — crates.io still 404s against the README's install instructions), then audit follow-ups (`unsafe_code` pin, bounded SVG/PNG/IPC surfaces) and CI; upstream #1777 still open (dock filtered 2026-04, switcher not); Flathub, thumbnails, type-to-filter, pointer input ruled out with citations; the pointer verdict superseded by `pointer-select` (shipped 2026-11-04) |
| 2026-11-06 | [README peer tone survey](readme-peer-tone.md) | Complete — 11 primary-source peer READMEs fetched (3 first-party COSMIC, 4 third-party cosmic-ext, fuzzel/hyprpaper/slurp/alttab): median 183 words, bullet-and-code-first, one-sentence descriptions; 0/11 ship Troubleshooting or Uninstall sections. Airlock's README measured 1103 words (6x median, 1.8x the longest peer), 99 prose vs 12 bullet lines — confirmed longest and most literary in its peer group |
