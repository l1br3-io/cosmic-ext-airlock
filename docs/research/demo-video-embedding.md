# Demo video embedding: GitHub README and project website

**Question:** What is the industry-standard approach (as of late 2026) for attaching/embedding a 5:15 screen recording of a desktop GUI app — currently a 9.4 MB VP9/WebM — in (a) a GitHub README and (b) the project's public website?

**Date:** 2026-11-15

## Findings

### 1. GitHub's officially supported README mechanism: upload through the markdown editor; GitHub rewrites it to an anonymized URL that renders an inline player

Video upload has been generally available since May 2021: "Video upload is now supported everywhere you can author Markdown in GitHub", explicitly covering "issue, pull request, and discussion comments as well as on repository Markdown files such as READMEs" (GitHub changelog, [Video uploads now generally available](https://github.blog/changelog/2021-05-13-video-uploads-now-generally-available/), 2021-05-13).

The current docs page (fetched 2026-11-15) is [Attaching files](https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/attaching-files) (the old `attaching-files-to-issues-and-pull-requests` slug now 404s). Its facts:

- Supported video types: "Video (`.mp4`, `.mov`, `.webm`)", with a compatibility note recommending H.264 "for the greatest compatibility". Airlock's VP9/WebM file is uploadable as-is, but an H.264 MP4 maximizes playback compatibility per the doc.
- Size caps, quoted: "10MB for images and gifs"; "10MB for videos uploaded to a repository owned by a user or organization on a free GitHub plan"; "100MB for videos uploaded to a repository owned by a user or organization on a paid GitHub plan"; "25MB for all other files".
- Upload flows: "To attach a file in your browser, drag and drop it into the comment box", the paperclip button, copy-paste in many browsers, or the `gh` CLI for local images and videos.
- On upload, "it is uploaded immediately to GitHub and the text field is updated to show the anonymized URL for the file" (the `github.com/user-attachments/assets/…` form). In a README, that URL on its own line renders the inline player.

The 9.4 MB file fits under the 10 MB free-plan cap with ~0.6 MB headroom — it works today, but any re-encode that adds a few hundred KB breaks it; trim or lower the bitrate before uploading if it might grow.

### 2. Repo-relative video files and raw `<video>` HTML: not documented, not reliably supported

- The markdown-syntax docs cover relative paths for *images* only: "When you want to display an image that is in your repository, use relative links instead of absolute links" — there is no equivalent statement for video, and the page's documented HTML is limited to `<sub>`, `<sup>`, `<ins>`, `<br/>`, `<picture>`, and comments ([Basic writing and formatting syntax](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax), fetched 2026-11-15).
- The "rendered file types" docs list images, STL, CSV, PDF, prose diffs, GeoJSON, notebooks, Mermaid — video is not among them ([Working with non-code files](https://docs.github.com/en/repositories/working-with-files/using-files/working-with-non-code-files), fetched 2026-11-15).
- Community evidence (no GitHub staff in the thread) on [discussion #173635](https://github.com/orgs/community/discussions/173635): a repo-relative `<video src="./path.mp4">` "showed a player but would not play" ("all the buttons are greyed out", 2025-09-16); a user-attachments `src` worked ("It worked for me just now with `<video src="https://github.com/user-attachments/assets/…"/>`", 2026-08-19); and "Inlining videos used to work but not anymore, not even with the `<video>` tag workaround" (2026-08-05) — i.e., the HTML-tag path is not a stable contract.
- A widely-referenced inventory of GitHub's HTML allowlist notes `<video/>` tags "are also allowed now, as long as they are from a github source" (comment, 2023-11-26) and that the tags "aren't documented anywhere that I can find"; the gist is personal experimentation, last active 2026-08-27 ([seanh's gist](https://gist.github.com/seanh/13a93686bf4c2cb16e658b3cf96807f2)).

Verdict: the documented contract is the upload flow. Embedding via `<video src="user-attachments URL">` works in the wild today (see upscayl below) but is officially undocumented and has broken before; a plain uploaded URL is the supported, stable form.

### 3. What major projects actually do

Raw READMEs fetched 2026-11-15:

- **upscayl/upscayl** — the only real video embed found, and it matches the community-verified pattern: `<video src="https://github.com/upscayl/upscayl/assets/…/…" autoplay muted loop />` — GitHub-hosted (user-attachments), GIF-replacement attributes.
- **microsoft/PowerToys** — repo-relative `<picture>` PNGs (light/dark variants) only; zero video embeds.
- **appwrite/appwrite** — hero `<img>` hosted on user-attachments (image, not video); zero video embeds.
- **surrealdb/surrealdb** — repo-relative hero/illustration images only; zero video embeds.
- **janhq/jan** — user-attachments banner image plus vendor-hosted store badges; zero video embeds.

Pattern: static hero images dominate; when a demo video is embedded, it is GitHub-hosted via user-attachments and muted/looped. None of the five embeds a YouTube iframe in the README.

### 4. Hosting options compared

| Option | Limits | Renders inline in README? | Notes |
|---|---|---|---|
| Upload → user-attachments | 10 MB video (free plan) / 100 MB (paid) | Yes, documented | "anonymized URL"; recommended by the docs flow |
| Repo-committed file | none hard, repo bloat | Not documented; community reports relative-path players greyed out | `Working with non-code files` omits video entirely |
| Release asset | "Each file included in a release must be under 2 GiB"; "Up to 1000 release assets"; "no limit on the total size" | Not documented | Releases are documented as packaging "software, along with release notes and links to binary files" ([About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases), fetched 2026-11-15) — a >10 MB demo can ride here, but inline rendering is not a documented behavior |
| YouTube/Vimeo | no practical cap | Not in markdown; link out | web.dev: "YouTube embeds block the main thread for more than 1.7 seconds for the median website" — embed iframes are expensive; a thumbnail-link keeps the README cheap |
| Self-hosted on the project site | whatever the site budget allows | n/a (README links out) | Full control, no third-party; the standard choice when the project has its own site (AltTab, the closest peer switcher, serves its hero media from alt-tab.app's own `/public/…`) |

For a project with its own website, the standard split is: README shows a GitHub-hosted copy (user-attachments upload, inline player), and the website self-hosts the file in its assets directory under its own performance budget. The same master file can feed both after re-encode.

### 5. Web best practices for the site embed (web.dev, fetched 2026-11-15)

From [Lazy loading video](https://web.dev/articles/lazy-loading-video) (last updated 2026-07-02) and the Learn Performance [video-performance](https://web.dev/learn/performance/video-performance) module:

- Attributes for a demo-style clip (the GIF-replacement pattern): `autoplay muted loop`, plus `playsinline` — "`playsinline` is necessary for autoplaying to occur in iOS". web.dev's example is exactly `<video autoplay muted loop playsinline>`. The caveat: "Avoiding autoplaying videos is usually best practice as it leaves the control with the user" — muted looping is the accepted exception for short demo loops.
- Poster: "The `poster` attribute gives the `<video>` element a placeholder that will be displayed while the video loads"; "A `poster` image will be quicker to load than the video so where this is an LCP candidate, you should use a poster image", preloaded with `fetchpriority="high"`.
- LCP: "the first frame of a video file — once painted — will be considered as an LCP candidate", and "Videos that are LCP elements shouldn't use `loading=`lazy`'".
- Preload: default `metadata` "can result in more data being downloaded than desired"; `preload="none"` is "the best chance of avoiding loading the video" for user-initiated playback. `autoplay` makes the video "begin downloading immediately, even if they are located outside of the initial viewport".
- Lazy loading: "the `loading=`lazy`' attribute defers even downloading the poster image and metadata until the video is in or approaching the viewport"; with autoplay + lazy, "autoplay videos won't autoplay until in or near the viewport". Keep `controls` as the no-JS/no-lazy fallback.
- Size: "Animated GIFs can stretch into the range of several megabytes… Videos of similar visual quality tend to be far smaller"; strip silent audio streams (ffmpeg `-an`) for GIF-replacement clips.

Applied to this file: 9.4 MB over 5:15 is ~250 kbps — fine as a click-to-play demo, heavy as an autoplaying hero. If the demo sits on the landing page above the fold, prefer a short looping excerpt (poster + muted loop) and link the full 5:15 recording; keep the 9.4 MB file user-initiated (`preload="none"` + poster + `loading="lazy"` if below the fold) so it stays out of LCP.

### 6. VP9/WebM browser support: keep an H.264 MP4 fallback source

- caniuse WebM (fetched 2026-11-15): global support "96.65% + 0.73% = 97.38%"; Safari full support from 16.0 (12.1–15.6 partial), iOS Safari full from 17.4 (12.2–17.3 partial, earlier unsupported).
- MDN containers page (last modified 2026-11-08): "Compliant implementations of WebM are required to support the VP8 and VP9 video codecs and the Vorbis and Opus audio codecs"; its per-browser table marks VP8/VP9/AV1 "Yes" for Chrome, Edge, Firefox, Safari — no version numbers.
- MDN codec guide (last modified 2026-11-05) claims VP9 support in "All versions of Chrome, Edge, Firefox, Opera, and Safari" and notes VP9 is "Open and free of royalties and any other licensing requirements". This overstates Safari (caniuse's versioned table shows partial pre-16 Safari); trust caniuse's versioned data for fallback decisions.
- MDN's own recommended markup is dual-`<source>`: `<source src="…webm" type="video/webm" /><source src="…mp4" type="video/mp4" />` with a download link fallback.

Verdict: VP9/WebM is the right primary encoding for a self-hosted demo in 2026, shipped as `<source type="video/webm">` plus an H.264 MP4 `<source>` to cover the residual old-Safari slice (and it is also the format GitHub's docs recommend for attachment compatibility).

### Recommended approach (verdict)

1. **README:** drag-and-drop the WebM into the README in the web editor → GitHub rewrites it to a `user-attachments` URL that renders the inline player (documented flow; 9.4 MB fits the free-plan 10 MB cap, barely). Optionally wrap that URL in `<video … autoplay muted loop>` — community-verified and used by upscayl, but officially undocumented, so keep the bare-URL form as the baseline. If the file grows past 10 MB, the release-asset route (2 GiB cap) stores it but loses inline rendering.
2. **Website:** self-host in the site's assets directory as VP9/WebM + H.264 MP4 dual sources with a poster image: `<video poster muted loop playsinline preload="metadata">` if autoplaying near the fold, or `preload="none" controls loading="lazy"` if user-initiated/below the fold. Keep the 9.4 MB file out of the LCP path; for a hero, cut a short excerpt loop and link the full recording.

## Sources

- https://github.blog/changelog/2021-05-13-video-uploads-now-generally-available/ — 2021 launch of video uploads in markdown, incl. READMEs (accessed 2026-11-15)
- https://docs.github.com/en/get-started/writing-on-github/working-with-advanced-formatting/attaching-files — formats (.mp4/.mov/.webm), 10 MB free / 100 MB paid video caps, H.264 compatibility note, upload flow, anonymized URL (accessed 2026-11-15)
- https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax — relative paths documented for images only; limited HTML subset (accessed 2026-11-15)
- https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases — 2 GiB/file, 1000 assets, binary-distribution purpose (accessed 2026-11-15)
- https://docs.github.com/en/repositories/working-with-files/using-files/working-with-non-code-files — rendered file types omit video (accessed 2026-11-15)
- https://github.com/orgs/community/discussions/173635 — community behavior reports: relative-path `<video>` greyed out, user-attachments `src` works (2025-09…2026-08), no staff replies
- https://gist.github.com/seanh/13a93686bf4c2cb16e658b3cf96807f2 — informal HTML-allowlist inventory; `<video/>` allowed from github sources (comment 2023-11-26; last active 2026-08-27)
- https://raw.githubusercontent.com/upscayl/upscayl/main/README.md — `<video src="user-attachments…" autoplay muted loop>` in a major README (accessed 2026-11-15)
- https://raw.githubusercontent.com/microsoft/PowerToys/main/README.md, …/appwrite/appwrite, …/surrealdb/surrealdb, …/janhq/jan — static-image-only READMEs for contrast (accessed 2026-11-15)
- https://web.dev/articles/lazy-loading-video — poster/preload/`loading="lazy"`/LCP guidance, updated 2026-07-02 (accessed 2026-11-15)
- https://web.dev/learn/performance/video-performance — `<video autoplay muted loop playsinline>`, autoplay download eagerness, YouTube-embed main-thread cost (accessed 2026-11-15)
- https://caniuse.com/webm — 97.38% global, Safari 16.0+/iOS 17.4+ full (accessed 2026-11-15)
- https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Containers — WebM requires VP8+VP9; dual-source fallback snippet; modified 2026-11-08
- https://developer.mozilla.org/en-US/docs/Web/Media/Formats/Video_codecs — VP9 royalty-free; modified 2026-11-05
- https://alt-tab.app — closest peer's landing serves hero media from its own `/public/…` (accessed 2026-11-15)

## Unverified

- **GitHub's exact HTML sanitizer allowlist.** The gist is personal experimentation and its author says the tags "aren't documented anywhere"; discussion #173635 has no GitHub staff participation. `<video>` with a user-attachments `src` worked in August 2026 (sschmidTU) but had broken days earlier for another user (devnoname120, 2026-08-05) — treat raw `<video>` in GitHub markdown as undocumented and unstable.
- **Inline rendering of release-asset URLs in markdown.** Not documented either way; the docs only document user-attachments uploads rendering. The 2 GiB release route is verified for storage only.
- **Whether committed repo video files play in the repo file view.** The docs' rendered-file-types page omits video, and community comments conflict ("GitHub does not stream video directly in the repository view" vs. common experience of in-browser playback on file pages). Did not resolve from a primary source.
- **MDN vs caniuse on Safari.** MDN's codec page says VP9 works in "all versions" of Safari; caniuse's versioned table shows Safari 16.0+ full / iOS 17.4+ full with partial before. Both recorded; caniuse's versioned data is the operationally safer one.
- **AltTab's hero `<video>` element.** Observed on alt-tab.app as `<video autoplay muted loop playsinline>` with a `.still.svg` poster on 2026-02-10; today's landing serves the still SVG directly and the video markup was not retrievable this pass — the pattern is plausible but only partially verified.
