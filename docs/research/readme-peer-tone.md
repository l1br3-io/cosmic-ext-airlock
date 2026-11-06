# README length and tone vs. peer apps — research findings

**Date:** November 6, 2026 · **Author:** research agent for jpt · **Status:** Complete — norm established from 11 primary-source READMEs; verdict for Airlock's README in §4

**Scope:** structure and tone survey of README files in Airlock's peer group — first-party COSMIC apps, third-party `cosmic-ext-*` apps/applets, single-purpose Wayland utilities, and X11 window switchers. Answers three questions: what does a peer README normally contain, how long is it, and in what voice. Companion to the repo's README work; no code claims.

**Method:** primary sources only — each peer's own README fetched from its repository's default branch (raw.githubusercontent.com; fuzzel via Codeberg after its GitHub repo 404'd, see §2.8). All fetched 2026-11-06. Word counts are `wc -w` on the raw Markdown (includes headings/code/links — same yardstick applied to Airlock's own README.md on the working tree). Heading lists, badge/screenshot/TOC/install flags, and bullet-vs-prose line counts were extracted mechanically from the fetched bytes; tone characterizations quote the fetched text verbatim. No secondary sources (no "how to write a README" articles) were used. Peer selection: the three first-party COSMIC apps named by the owner, the four most-starred distinct third-party `cosmic-ext-*` apps/applets across maintainers from GitHub search (`cosmic-ext in:name`, sorted by stars), and one pick each from the launcher/hyprland/emersion/switcher families the owner listed.

---

## 1. TL;DR — the peer norm, in numbers

| Metric | Peer norm (n=11) | Airlock README |
|---|---|---|
| Word count (`wc -w`) | min 75, **median 183**, mean 221, max 622 (fuzzel) | **1103** — 6x the median, 1.8x the longest peer |
| Markdown headings | 2–14, typical 4–8 | 11 (1 h1 + 10 h2) |
| Body composition | bullets + fenced code dominate; prose is a garnish (e.g. fuzzel: 45 bullet lines vs 37 prose; hyprpaper 8 vs 7) | **99 prose lines vs 12 bullet lines** — inverted |
| Mean words per prose sentence (code/headings/bullets stripped) | 13–24 across peers (medians 5–19) | 22.8 mean, 19 median, **max 68** |
| Badges at top | 4 of 11 (fuzzel: CI + repology + donate; alttab: CI; tweaks/tasks: Flathub) | none |
| Screenshot near top | 6 of 11 (all four third-party cosmic-ext; none of the three first-party pop-os apps) | yes |
| Table of contents / in-page anchors | 0 of 11 | several cross-reference anchors (no TOC block) |
| Install/build section with code block | 10 of 11 (all but cosmic-term) | yes |
| Per-distro install matrix | 1 of 11 (hyprpaper: Arch + openSUSE one-liners); fuzzel delegates to a repology badge | no (single `cargo install` path) |
| "Status" or project-state statement | 1 of 11 (caffeine: archival notice) | yes (Status line + Why section) |
| "Why this exists" section | 1 of 11 with a real paragraph (alttab); 1 with a shutdown note (caffeine) | yes |
| Troubleshooting section | **0 of 11** (nearest: cosmic-launcher's contributor-facing "Debugging & Profiling") | yes |
| Uninstall instructions | **0 of 11** | yes (dedicated section) |
| Voice | declarative one-line description + imperative build/install steps ("Clone this repository and run:") | description + narrative prose with em-dash asides throughout |

---

## 2. The evidence, project by project

Each entry: README URL (fetched 2026-11-06), word count, heading list, structure facts, tone facts with verbatim quotes.

### 2.1 pop-os/cosmic-launcher (first-party, closest kin — its alt-tab view is what Airlock replaces)

<https://raw.githubusercontent.com/pop-os/cosmic-launcher/master/README.md> — 284 words.

Headings (in order): `# Cosmic Launcher`, `# Building`, `## Build Commands`, `# Outside build chroot`, `# Inside build chroot`, `# Translators`, `# Debugging & Profiling`, `## Profiling async tasks with tokio-console` (the last two pairs are code-comment-style headings inside blocks, not real sections).

Structure: no badges, no screenshot anywhere, no TOC, no user-install section (install = `just` / `sudo just install` for packagers), no troubleshooting-for-users, no uninstall, no license section. 3 code blocks.

Tone: opens with a technical fragment, not a pitch — "Layer Shell frontend for https://github.com/pop-os/launcher." Build voice is imperative: "For a typical install from source, use `just` followed with `sudo just install`." The longest prose (Translators) is procedural, single long paragraph. No marketing language anywhere.

Unusual: half the file is contributor/packager mechanics (vendored builds, translations, tokio-console profiling) — a dev-facing README with essentially zero user documentation.

### 2.2 pop-os/cosmic-term

<https://raw.githubusercontent.com/pop-os/cosmic-term/master/README.md> — 87 words.

Headings: `# cosmic-term`, `## Color Schemes`. No badges, screenshot, TOC, install, troubleshooting, uninstall, or license section.

Tone: pure declarative description — "COSMIC terminal emulator, built using [alacritty_terminal] that is provided by the [alacritty] project." Short-to-medium sentences, three short paragraphs total.

Unusual: the shortest first-party README; no build instructions at all.

### 2.3 pop-os/cosmic-files

<https://raw.githubusercontent.com/pop-os/cosmic-files/master/README.md> — 127 words.

Headings: `# cosmic-files`, `## Build the project from source`, `## Community and Contributing`, `## License` (plus four comment lines inside the one code block that read like headings).

Structure: entire install section is one code block with comments; "Community and Contributing" is the shared pop-os boilerplate paragraph.

Tone: the whole product pitch is eight words — "File manager for the COSMIC desktop environment". Everything after it is clone/build commands and links. The most minimal voice in the set.

### 2.4 cosmic-utils/cosmic-ext-tweaks (third-party, most-starred cosmic-ext app)

<https://raw.githubusercontent.com/cosmic-utils/cosmic-ext-tweaks/main/README.md> — 107 words.

Headings (Markdown): `## Getting Started`, `## Dependencies`, `## Installation`, `## License`, above a centered HTML header (`<h1>Tweaks</h1>` + tagline).

Structure: Flathub badge; two theme-aware screenshots immediately under the header; install = 2-line code block; dependencies as 4 bullets. No TOC, status, troubleshooting, or uninstall.

Tone: carries the one genuinely marketing-ish line in the COSMIC set — the tagline "Beyond the limits of your desktop" — then drops to bare imperatives: "Clone this repository to your local machine and run:".

### 2.5 cosmic-utils/cosmic-ext-tasks

<https://raw.githubusercontent.com/cosmic-utils/cosmic-ext-tasks/main/README.md> — 75 words.

Headings: `# Installation`, `# Build`, `## Dependencies`, `# Copyright and licensing` (plus HTML `<h1>Tasks</h1>` header).

Structure: centered logo + screenshots + Flathub badge, one-sentence description ("A simple task management application for the COSMIC™ desktop"), install and build as bare code blocks, one dependency bullet, license. The shortest README in the survey.

Tone: no prose beyond the description sentence and the copyright line.

Unusual: the repo now lives under the cosmic-utils org but the README's clone URLs and license link still point at `edfloreshz/tasks` — stale self-links after an org move.

### 2.6 D-Brox/cosmic-ext-applet-system-monitor

<https://raw.githubusercontent.com/D-Brox/cosmic-ext-applet-system-monitor/master/README.md> — 287 words.

Headings: `# System Monitor Cosmic Applet`, `## Installing`, `## Building from source`, `# or` (a comment line), `## Roadmap`, `## Contributing`, `## Special Thanks`.

Structure: screenshot right under the one-line description; install offers prebuilt `.deb`/`.rpm`/tarball from releases plus a Flatpak repo; build steps as code blocks with a 3-bullet dependency list; a checkbox Roadmap (the only peer with one); credits bullets. No badges, TOC, status, troubleshooting, or uninstall.

Tone: one-line description — "A highly configurable system resource monitor for the COSMIC DE" — then imperative steps: "You can just grab the `.deb`, `.rpm` or tarball from the [releases] page, or install from the [COSMIC Flatpak Repository]." Bullets and code carry nearly everything; prose sentences are short.

### 2.7 tropicbliss/cosmic-ext-applet-caffeine

<https://raw.githubusercontent.com/tropicbliss/cosmic-ext-applet-caffeine/main/README.md> — 286 words.

Headings: `# Caffeine Applet for the COSMIC™ desktop`, `## Why am I archiving this repo?`, `## How It Works`, `## Install`, `## Enable the applet`, `## Additional Credits`.

Structure: screenshot; install code block plus a Debian-dependencies block; no badges, TOC, troubleshooting, or uninstall.

Tone: the only peer with a first-person narrative block — but it is a shutdown notice: "I no longer mainline COSMIC desktop and it has become increasingly obvious that a background daemon is required to support a multi-monitor setup, which is in my opinion going above and beyond what a simple tray applet should do." The rest is plain: "A lightweight system tray application that prevents your screen from going to sleep. Perfect for presentations, watching videos, or any situation where you need your display to stay active." Mild marketing, two short sentences.

Unusual: "Why am I archiving this repo?" is the only status/why heading in the peer set — an anti-precedent for Airlock's healthy-project "Why" section, but proof the section name itself is not unheard of.

### 2.8 dnkl/fuzzel (launcher)

GitHub repo gone (404 on both raw and API as of 2026-11-06 — the project moved to Codeberg). Canonical README fetched from <https://codeberg.org/dnkl/fuzzel/raw/branch/master/README.md> — 622 words, the longest peer.

Headings: `# Fuzzel`, `## Screenshot`, `## Features` (`### For app launching`, `### For dmenu mode`, `### For all modes`), `## Requirements` (`### Runtime`, `### Building`), `## Installation`, `## Testing`, `# No AI / LLM policy`, `## License`, `# Sponsoring/donations`.

Structure: CI badge + repology packaging-status badge + screenshot; 40+ feature bullets across three subsections; meson build steps as 7 short code blocks; dependency bullets with footnotes; a contribution policy section; donation links. No TOC, status, troubleshooting, or uninstall.

Tone: one-sentence self-definition — "Fuzzel is a Wayland-native application launcher and fuzzy finder, inspired by rofi and dmenu." Install steps are numbered imperatives: "To build, first, create a build directory, and switch to it". Even at 622 words, roughly two-thirds of the file is bullets and code, and the longest prose run is three sentences.

Unusual: the explicit "No AI / LLM policy" and sponsoring sections are unique in the set; per-distro packaging is delegated to the repology badge instead of a matrix.

### 2.9 hyprwm/hyprpaper

<https://raw.githubusercontent.com/hyprwm/hyprpaper/main/README.md> — 134 words.

Headings: `# hyprpaper`, `# Features`, `# Installation`, `## Manual:`, `### Dependencies`, `### Building`.

Structure: the only peer with per-distro install lines — "[Arch Linux]: `pacman -S hyprpaper`", "[OpenSuse Linux]: `zypper install hyprpaper`" — then CMake manual build. No badges, screenshot, TOC, status, troubleshooting, or uninstall.

Tone: single-sentence description — "Hyprpaper is a simple and fast wallpaper utility for Hyprland with the ability to dynamically change wallpapers through sockets." — then 4 feature bullets ("Per-output wallpapers", "IPC for fast wallpaper switches", …) and commands. Zero narrative.

### 2.10 emersion/slurp

<https://raw.githubusercontent.com/emersion/slurp/master/README.md> — 183 words.

Headings: `# slurp`, `## Building`, `## Example usage`, `## Contributing`, `## License`.

Structure: no badges or screenshot; build deps as 5 bullets; five example-invocation code blocks; license is the word "MIT". No TOC, status, troubleshooting, or uninstall.

Tone: the tersest voice — the description is a single imperative-shaped sentence of 12 words: "Select a region in a Wayland compositor and print it to the standard output." Everything else is shell commands with one-line leads ("Select a single point instead of a region:").

### 2.11 sagb/alttab (X11 window switcher, the switcher-adjacent peer)

<https://raw.githubusercontent.com/sagb/alttab/master/README.md> — 236 words.

Headings: `# Installation`, `# See also` (the title is an image + bold text, not a heading).

Structure: three screenshots before any text; CI badge; full usage synopsis as a code block ("(see man page for details)"); install code block plus links to INSTALL.md and per-WM setup docs; no TOC, status, troubleshooting, or uninstall.

Tone: the only peer with a real "why me" paragraph, and it is one compound sentence of telegraphic style: "Unlike task switchers integrated in most simple window managers (WM) or dmenu-like switchers, alttab features visual interface and convenient tactile behaviour: press modifier (Alt) - multiple switch with a key (Tab) - release modifier." Note the hyphen-chained clauses instead of prose connectives — dense but mechanical, no idiom.

Unusual: opens with an HTML comment stating the README's scope policy ("This file is solely for github front page. For distribution, use doc/README instead.") — the set's only example of deliberately splitting a README's audiences.

---

## 3. Synthesis — what the norm is

**Shape.** The modal peer README is: title, one-sentence plain description of what the thing is ("X is a Y for Z"), optional screenshot, optional short feature bullet list, install/build as code blocks under an Install/Build heading, license. Median 183 words; 75th percentile ≈ 286; the only 600+ word peer (fuzzel) earns it with a feature catalog in bullets, a build walkthrough in code blocks, and policy/sponsoring sections — not with paragraphs.

**Bullet-vs-prose.** Every peer is bullet-and-code first. Prose exists to (a) say what the project is, in one or two sentences, and (b) introduce a command. No peer has a section that is more than ~3 sentences of running text (caffeine's archival paragraph and cosmic-launcher's translators paragraph are the two longest runs, and both are special cases).

**Sentence style.** Short-to-medium declaratives and imperatives. Mean sentence length 13–24 words per peer, but the *median* is 5–19 and the max under ~50 (fuzzel 46, slurp 57 in a single list-heavy example; alttab 39). No peer uses the em-dash-aside + semicolon-chain construction that Airlock uses pervasively; the nearest stylistic cousin is alttab's "press modifier (Alt) - multiple switch with a key (Tab) - release modifier", which is a compressed mechanic, not narrative.

**Voice.** Impersonal. "Clone this repository… and run:" (tweaks); "To build, first, create a build directory, and switch to it" (fuzzel); "Install with:" (hyprpaper). First person appears once (caffeine's farewell). Marketing language is capped at one tagline or adjective set per README ("Beyond the limits of your desktop"; "highly configurable"; "simple and fast"; "Perfect for presentations…").

**Section-content norms.** Status statements: not a peer norm (1/11, and that one is an archive notice). A "why" section: not a norm, but alttab shows a one-sentence precedent. Troubleshooting: absent from all 11. Uninstall: absent from all 11 — peers that install system files (alttab: `sudo make install`) simply do not cover removal. Per-distro install matrices: essentially absent (1/11; fuzzel badges it instead). Screenshots: split — universal among third-party cosmic-ext apps, absent among first-party pop-os apps.

## 4. Where Airlock's README sits

Measured the same way (README.md on the working tree): 1103 words — 6x the peer median, 1.8x the longest peer. 11 headings vs the 4–8 typical. Composition inverted: 99 prose lines vs 12 bullet lines; no peer in the set is prose-majority. Mean sentence length 22.8 words (peer means 13–24) with a maximum of 68 — Airlock is at the top of the peer range on average sentence length, but the divergence is not chiefly sentence length; it is volume of running text, the em-dash/semicolon narrative register, and four sections no peer has (Uninstall, Troubleshooting, Performance table, Documentation index) plus Status + Why where only caffeine/alttab offer partial precedents.

Verbatim contrasts (most telling):

1. **The product description.** cosmic-files, an entire first-party file manager, is introduced in eight words: "File manager for the COSMIC desktop environment". Airlock's opener carries a clause and an idiomatic payoff: "Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and cycles only the windows on the **current workspace**, so nothing you parked elsewhere leaks into your focus."
2. **The behavior description.** Peers state mechanics as flat imperatives or bullets (slurp: "Select a single point instead of a region:" + one command). Airlock narrates the same kind of content with asides: "releasing **Alt** switches and dismisses, **Enter** confirms without letting go, **Escape** dismisses, and a quick tap — released before the overlay appears — still switches."
3. **The removal story.** No peer documents uninstall at all; alttab installs system-wide with `sudo make install` and stops there. Airlock devotes a section plus narrative guarantees: "the pre-install state returns byte-for-byte — a `custom` file install created is deleted again, and an asset file you edited by hand is kept and named, never silently deleted."

Verdict: the owner's suspicion is confirmed by primary sources on both axes. Airlock's README is the longest and the most literary in its peer group — ~6x median length, prose-dominant against a bullet-and-code norm, narrative voice with marketing-adjacent flourishes ("milliseconds-thin", "exits loudly by design") where peers cap flavor at one tagline. Two nuances the raw numbers shouldn't obscure: (a) Airlock's extra length is partly buying things peers simply omit — a Status line, a Why, Troubleshooting, Uninstall, a Performance table — so the excess is section count as much as prose style; (b) its sentence-level mechanics sit at the long end of the peer range rather than outside it. If trimmed toward the norm, the biggest levers are the Uninstall/Troubleshooting/Packaging/Development prose (candidates for `docs/` pages linked in one line each, a pattern fuzzel uses with its man pages) and compressing How-it-works narrative into the bullet/definition style every peer uses.

## 5. Caveats

- Word counts count raw Markdown including code blocks and link URLs; the yardstick is consistent across all twelve files, but a fair number of fuzzel's "words" are meson flags. Direction of every conclusion survives this noise (1103 vs 183 median is not a measurement artifact).
- The words-per-sentence figure strips fenced code, inline code, headings, tables, images, HTML tags, and bullet lines, then splits on sentence enders; abbreviations/decimals could shift single-file means by a word or two. Files with very few counted sentences (ext-tasks: 1, ext-sysmon: 2, ext-tweaks: 2) mean their means carry little weight — those READMEs are code-and-bullet bodies.
- `cosmic-ext` selection is stars-biased (most-starred distinct apps across maintainers); smaller cosmic-ext projects may trend even terser (see ext-tasks at 75 words), which would only sharpen the verdict, not soften it.
- fuzzel's README was fetched from Codeberg, its canonical home, after the GitHub repo 404'd on 2026-11-06; the content is the project's own default-branch README either way.
- Peer READMEs are moving targets; all facts above are pinned to the 2026-11-06 fetches. Airlock's own measurements are from the working-tree README.md at the time of writing (11 headings; the index row below pins the date).
