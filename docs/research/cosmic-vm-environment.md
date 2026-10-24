# VM environment for testing Pop!_OS COSMIC as a guest — research findings

**Date:** October 24, 2026 · **Scope:** running Pop!_OS 24.04 LTS / COSMIC Epoch 1 (cosmic-epoch `epoch-1.11.0`, tagged 2026-10-20, latest as of research date) as a KVM guest on this COSMIC host (kernel `7.2.6-76070206-generic`), for airlock's CP4 gate · **Method:** primary sources only — system76.com download/support/blog pages, pop-os GitHub source and issue trackers, qemu.org documentation, docs.mesa3d.org, docs.kernel.org (VFIO), Oracle VirtualBox manual, Broadcom/VMware tech docs, GNOME GitLab and virt-manager/libvirt trackers; plus direct inspection of this host's packages. Community reports are explicitly marked and never load-bearing.

---

## 1. TL;DR

**Use virt-manager + KVM with a virtio-gpu (VirtIO) video device and "3D acceleration" (virgl) enabled, UEFI (OVMF) firmware, and the standard Pop!_OS 24.04 LTS Intel/AMD ISO — no GPU passthrough, no Venus/Vulkan paravirt, and not GNOME Boxes.** The load-bearing discovery: **COSMIC does not require Vulkan in the guest.** cosmic-comp's renderer is OpenGL ES (Smithay `renderer_glow`, with a pixman software fallback), and libcosmic/wgpu apps automatically fall back to OpenGL when Vulkan fails to initialize — stated verbatim by the maintainers in [cosmic-epoch#3831](https://github.com/pop-os/cosmic-epoch/issues/3831) (Drakulix, 2026-08-24: *"cosmic-comp currently only has an OpenGL ES backend and cosmic apps also have a software rendering fallback"*; mmstick: *"wgpu automatically falls back to OpenGL when Vulkan fails to initialize"*). A virgl guest therefore gives cosmic-comp real (host-side) GL and gives airlock a real GL path, which is sufficient for **functional** CP4 testing: config takeover, Alt+Tab across workspaces, mid-gesture daemon kill, restore. Verified in the field by [cosmic-epoch#3844](https://github.com/pop-os/cosmic-epoch/issues/3844) (Oct 2026): COSMIC 1.6.0 running in a virtio-gpu/virgl VM with zero Vulkan libraries loaded in cosmic-comp or any iced/wgpu component. Venus (paravirt Vulkan) is unnecessary for COSMIC and is not reachable from virt-manager anyway ([virt-manager#362](https://github.com/virt-manager/virt-manager/issues/362), blocked on [libvirt#638](https://gitlab.com/libvirt/libvirt/-/issues/638)); VFIO passthrough buys fidelity this gate does not need; VirtualBox/VMware offer no guest Vulkan and weaker GL. What a VM **cannot** validate: hardware-Vulkan renderer behavior, timing/latency feel, and multi-monitor layouts (see §10).

---

## 2. Guest OS: Pop!_OS 24.04 LTS with COSMIC is stable, not alpha

- COSMIC Epoch 1 shipped **stable** with **Pop!_OS 24.04 LTS on 2025-12-11**; confirmed by System76's blog: *"COSMIC Epoch 1 was released December 11th, 2025"*, with subsequent point releases (1.0.1 tagged 2025-12-30, 1.0.2 tagged 2026-01-13) rolling out on 24.04 LTS as they pass QA ([System76 blog, *The Latest Updates to COSMIC Epoch 1*](https://system76.com/blog/post/cosmic-epoch-1-updates), 2026-01-16). The umbrella repo continues tagging: `epoch-1.11.0` is the latest release (2026-10-20, [cosmic-epoch releases](https://github.com/pop-os/cosmic-epoch/releases)); per its [README](https://github.com/pop-os/cosmic-epoch), Pop!_OS consumes submodule updates through its own build system, so an installed 24.04 VM updated via `pop-upgrade`/Store tracks something between the last two epoch tags. **There is no alpha/beta channel to pick anymore — the default download is the stable COSMIC edition.**
- The [download page](https://system76.com/download-pop/) lists exactly four 24.04 ISOs, all COSMIC: **Intel/AMD** ("for NVIDIA 10 Series and older" too), **NVIDIA 16-series+**, and two **ARM64** builds (irrelevant on this x86-64 KVM host). Stated minimums: 64-bit CPU, **4 GB RAM, 16 GB storage, Secure Boot disabled**. The ISO ships Linux kernel 7.0.9, Mesa 25.1.5, NVIDIA 585 — Mesa 25.1.5 exceeds the Venus guest minimum (Mesa 24.2.0+, §4) and comfortably covers virgl.
- Pop!_OS 18.04+ boots via **systemd-boot** and the installer targets UEFI ([System76, *Repair the Bootloader*](https://system76.com/support/bootloader/)); the VM must use UEFI firmware (OVMF), not SeaBIOS. Secure Boot must be off ([download page](https://system76.com/download-pop/)); virt-manager's OVMF default leaves it off unless explicitly enabled.
- Alternative guest: the **Fedora COSMIC Spin** is an official option ([system76.com/cosmic](https://system76.com/cosmic)) and is what issue #3844 actually ran. Not needed here — CP4 targets the Pop!_OS config layout.

## 3. What COSMIC actually needs from a GPU (verified in source and issues)

- **cosmic-comp (the compositor) uses Smithay's GL/GLES renderer, not Vulkan.** [Cargo.toml](https://github.com/pop-os/cosmic-comp/blob/master/Cargo.toml) pins `smithay 0.7.0` with features `backend_egl`, `backend_vulkan`, **`renderer_glow`** (GLES via glow), `renderer_multi`, and **`renderer_pixman`** (CPU software renderer); there is no direct vulkano/ash dependency. Maintainer confirmation in [#3831](https://github.com/pop-os/cosmic-epoch/issues/3831) (2026-08): Drakulix — *"cosmic-comp currently only has an OpenGL ES backend and cosmic apps also have a software rendering fallback."*
- **libcosmic/iced (wgpu) apps fall back to GL when Vulkan is absent** — mmstick in the same issue: *"wgpu automatically falls back to OpenGL when Vulkan fails to initialize"*; and in [#3844](https://github.com/pop-os/cosmic-epoch/issues/3844): *"Add export WGPU_BACKEND=gl to your VM's environment variables."* Consequence: in a virgl-only guest, `vulkaninfo` reports just llvmpipe and some Vulkan-sensing apps report an emulated GPU ([cosmic-epoch#2215](https://github.com/pop-os/cosmic-epoch/issues/2215), open), but the session itself runs.
- **Live proof in a VM:** [#3844](https://github.com/pop-os/cosmic-epoch/issues/3844) (Oct 2026) ran Fedora COSMIC Spin / COSMIC 1.6.0 in a virtio-gpu VM with virgl on (`+virgl` in dmesg, no `resource_blob`/`context_init`, i.e. no Venus features): COSMIC worked; `lsof` showed **no Vulkan libraries loaded** in cosmic-comp or in cosmic-panel/launcher/settings/osd/notifications; `glxinfo` reported `virgl`. The complaint was only *fluidity* vs KWin, not correctness. Non-fatal log noise in VMs: *"Unable to become drm master, assuming unprivileged mode"* and *"Failed to initialize drm lease global"* — the reporter notes these also appear on bare metal ([#3782](https://github.com/pop-os/cosmic-epoch/issues/3782)).
- **History (alpha era, now fixed):** [cosmic-comp#136](https://github.com/pop-os/cosmic-comp/issues/136) (2023) — black screen with virtio because cosmic-comp's KMS format list lacked XRGB8888 (the only format the virtio primary plane supports); fixed by adding the format plus Smithay fix [smithay#1070](https://github.com/Smithay/smithay/pull/1070). Accelerated virtio was confirmed working in-thread; unaccelerated virtio still failed sessions as of Aug 2024, and mmstick advised (May 2024) using the **QXL** adapter as the workaround — noting **GNOME Boxes cannot configure that, so virt-manager is required**. (Issue left open; the epoch-1.x state is that virgl works, per #3844.)

**Verdict:** guest Vulkan (Venus/lavapipe) is *not* a prerequisite. What COSMIC needs is a working EGL/GLES context — provided by virgl (host GPU), or degraded to llvmpipe/pixman in plain 2D virtio/QXL (works, slow: the alpha-era recommendation of QXL was about correctness then, not speed).

## 4. QEMU/KVM display options: 2D vs virgl vs venus

Per the official [QEMU VirtIO GPU documentation](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html) (master/11.1.50):

| Mode | What it gives the guest | Host requirements | Guest requirements |
|---|---|---|---|
| Default 2D (`-device virtio-gpu` / virtio-vga) | Nothing 3D — *"The default 2D backend only performs 2D operations"*; the guest must fall back to *"Mesa's LLVMpipe/Lavapipe, or SwiftShader"* | QEMU | any |
| **virgl** (`virtio-gpu-gl` / "3D acceleration") | Guest **OpenGL** translated to host GL via virglrenderer (Gallium3D IR); guest GL capped at 4.3 by default | libvirglrenderer **0.8.2+**; *"Any Linux version compatible with QEMU if not using host blobs feature, Linux 6.13+ otherwise"* | Linux with virtio-gpu; the doc's table also lists a "Mesa 16.0.0+" figure for the GL column (as printed) |
| **venus** (`virtio-gpu-gl,hostmem=…,blob=true,venus=true`) | Guest **Vulkan** — *"Translation of Vulkan API calls is supported since release of virglrenderer v1.0.0 using venus protocol"*; requires host blobs | **Linux 6.13+**, virglrenderer **1.0.0+**, host GPU access | **Linux 5.16+, Mesa 24.2.0+** |

- [Mesa's Venus driver docs](https://docs.mesa3d.org/drivers/venus.html) add host-driver specifics: host needs **Vulkan 1.1 + `VK_KHR_external_memory_fd`**; tested hosts include ANV/RADV 21.1+, NVIDIA proprietary 570.86+, **Lavapipe 22.1+**; some combos (Intel xe, NVIDIA-on-Intel) additionally need kernel 6.16+/QEMU 11.0+ with `honor-guest-pat=on`. Guest kernel 5.16+ with `RESOURCE_BLOB`, `CONTEXT_INIT`, etc.
- **Venus is unnecessary for COSMIC** (§3) and **not reachable from virt-manager**: [virt-manager#362](https://github.com/virt-manager/virt-manager/issues/362) ("RFE: Add support for Venus / Vulkan VirtIO-GPU driver") is open and labeled *blocked* on [libvirt#638](https://gitlab.com/libvirt/libvirt/-/issues/638); maintainer crobinso notes Venus went upstream with QEMU 9.2.0 (Nov 2024 comment), but even the `qemu:commandline` escape hatch "still needs patches" per the last user report (Oct 2025).
- **This host, checked directly (2026-10-24):** Pop!_OS 24.04 LTS, kernel 7.2.6 (≥ 6.13 — fine), `/dev/kvm` present and owned by `kvm`. QEMU/virt-manager are *not yet installed*; the distro candidates are `qemu-system-x86 1:8.2.2+ds-0ubuntu1.18`, `virt-manager 1:4.1.0-3ubuntu0.1`, and `libvirglrenderer1 1.0.0-1ubuntu2`. So: **virgl is fully covered by stock packages** (virglrenderer 1.0.0 ≥ 0.8.2); **venus is effectively unavailable** through the distro stack (QEMU 8.2 < 9.2 upstream-venus threshold, plus no libvirt XML support). That is acceptable — venus is not needed.

## 5. Frontend choice: virt-manager (not Boxes, not hand-rolled QEMU)

- **virt-manager — recommended.** It exposes exactly the knobs needed: VirtIO video + "3D acceleration" checkbox (virgl → `virtio-vga-gl`), UEFI firmware selection, virtio disk/net, snapshots for CP4 repeatability. It is the tool System76's own maintainers point COSMIC-in-VM users to (mmstick, May 2024, [cosmic-comp#136](https://github.com/pop-os/cosmic-comp/issues/136): GNOME Boxes can't configure what's needed, *"so virt-manager is needed"*). SPICE+GL requires the display to be local with listen type "none" (standard virt-manager behavior when OpenGL is enabled) — community-documented step-by-step: [Himmelwright, *VirtIO 3D acceleration*](https://ryan.himmelwright.net/post/virtio-3d-vms), [AskUbuntu 1348975](https://askubuntu.com/questions/1348975/how-to-use-opengl-3d-acceleration-in-virt-manager-with-ubuntu) [community].
- **GNOME Boxes — not for this.** System76's general virtualization guide covers Boxes ([system76.com/support/virtualization](https://system76.com/support/virtualization/)) and its niceties (spice-vdagent clipboard/resize, spice-webdavd folder share, snapshots, 8 GB/60 GB sizing guidance) — but Boxes' 3D acceleration has a troubled record: the virgl option itself was the subject of [GNOME/gnome-boxes#288](https://gitlab.gnome.org/GNOME/gnome-boxes/-/issues/288) and [#696 "3D acceleration doesn't work"](https://gitlab.gnome.org/GNOME/gnome-boxes/-/issues/696); Ubuntu at one point disabled it for guest VMs due to kernel hangs ([Phoronix](https://www.phoronix.com/forums/forum/software/distributions/1320743-ubuntu-22-04-lts-disables-3d-acceleration-for-guest-vms-with-gnome-boxes-virt-manager/page3) [community/press]); the Flatpak build lacked the option until v49 ([flathub#77](https://github.com/flathub/org.gnome.Boxes/issues/77)). Boxes cannot set QXL fallbacks or VRAM, and no Venus. Use it only for a quick throwaway look.
- **Plain QEMU** — full control (including `venus=true` if you ever build QEMU ≥ 9.2), no snapshot UI; overkill here.

## 6. Other hypervisors

- **VirtualBox:** guest 3D is *experimental OpenGL / Direct3D 8–9 only*. Manual §4.5: *"The Oracle VM VirtualBox Guest Additions contain experimental hardware 3D support for Windows, Linux, and Oracle Solaris guests"* for apps *"using OpenGL or Direct3D 8/9"*, with an explicit security warning for untrusted guests ([VirtualBox manual, ch. 4](https://www.virtualbox.org/manual/ch04.html)). **Vulkan is not offered at all** (absent from the chapter). GL-level acceleration might technically start cosmic-comp, but nothing about this is aligned with a Wayland-Vulkan-era DE, and the COSMIC community reports around it are negative ([cosmic-epoch#2657](https://github.com/pop-os/cosmic-epoch/issues/2657) — VirtualBox fullscreen bug). **Reject.**
- **VMware Workstation:** guest graphics top out at **OpenGL 4.3 compatibility profile** for Linux guests via the SVGA3D driver ([Broadcom tech docs, Workstation Pro 25H2 — *Prepare the Host System to Use 3D Accelerated Graphics*](https://techdocs.broadcom.com/us/en/vmware-cis/desktop-hypervisors/workstation-pro/25H2/using-vmware-workstation-pro/configuring-and-managing-virtual-machines/configure-display-settings-for-a-virtual-machine/prepare-the-host-system-to-use-accelerated-3d-graphics.html); [Mesa svga3d docs](https://docs.mesa3d.org/drivers/svga3d.html)). The "Vulkan renderer" on that page is a *host-side* renderer for D3D11/GL guests — **no guest Vulkan**. Community reviews of COSMIC in VMware-family VMs report popup/window rendering struggles ([r/pop_os 24.04 review thread](https://www.reddit.com/r/pop_os/comments/1po4g42/pop_os_2404_full_review_the_cosmic_desktop_is_here) [community]). Possible-but-inferior; also proprietary host tooling on a COSMIC host. **Reject for CP4.**
- **Parallels (evidence only, Apple-Silicon host):** COSMIC *does* run there over virtio-gpu/virgl — that is exactly issue #3844 — with reduced fluidity and Fedora-specific tiling quirks ([#3306](https://github.com/pop-os/cosmic-epoch/issues/3306)). Validates the virtio-gpu approach cross-platform; not available to us.

## 7. GPU passthrough (VFIO) — the high-fidelity option, not needed for CP4

The kernel's VFIO framework is *"an IOMMU/device agnostic framework for exposing direct device access to userspace, in a secure, IOMMU protected environment"*, enabling VM device assignment with *"significantly reduced latency, higher bandwidth"* and bare-metal drivers; requires VT-d/AMD-Vi DMA remapping, IOMMU groups, and binding the device to `vfio-pci` ([docs.kernel.org, VFIO](https://docs.kernel.org/driver-api/vfio.html)). A passed-through GPU would give the guest real Vulkan and hardware timing — the only way to exercise the hardware-Vulkan codepath of libcosmic/wgpu inside a VM. Cost: a second GPU (or fragile single-GPU reset tricks), host session reconfiguration. **Not justified for a functional config-takeover gate; revisit only if CP5+ needs Vulkan-specific renderer behavior.**

## 8. Known-good recipes (evidence summary)

| Source | Setup | Outcome |
|---|---|---|
| [cosmic-epoch#3844](https://github.com/pop-os/cosmic-epoch/issues/3844) (Oct 2026, primary) | Fedora COSMIC 1.6.0, virtio-gpu **with virgl**, no Venus features | Works; all-COSMIC UI functional on GL; fluidity below KWin; no Vulkan libs loaded |
| [cosmic-comp#136](https://github.com/pop-os/cosmic-comp/issues/136) (2023–2024, primary) | libvirt VMs, virtio ± accel, QXL | XRGB8888 plane-format bug fixed (smithay#1070); **accelerated virtio confirmed working**; unaccelerated virtio failed sessions (Aug 2024); QXL = fallback; use virt-manager not Boxes (mmstick) |
| [r/pop_os "COSMIC + VMs"](https://www.reddit.com/r/pop_os/comments/1e2fsc2/cosmic_vms) (2024) [community] | System76 devs in-thread | Software rendering for VMs was in progress; recommendation: enable GPU acceleration |
| [9to5linux COSMIC 1.9 comments](https://9to5linux.com/cosmic-1-9-desktop-environment-released-with-cosmic-viewer-and-cosmic-osk) (Sep 2026) [community] | VM (VirtualBox 7.2 mentioned nearby) | "COSMIC in a VM is surprisingly responsive, almost like no VM" |
| [Himmelwright](https://ryan.himmelwright.net/post/virtio-3d-vms) / [AskUbuntu](https://askubuntu.com/questions/1348975/how-to-use-opengl-3d-acceleration-in-virt-manager-with-ubuntu) [community] | virt-manager: VirtIO video + 3D accel, SPICE GL local | Canonical how-to for the exact checkbox set |

No System76-staff forum recipe for "COSMIC in a VM" surfaced beyond the GitHub maintainer statements above; the official support article ([virtualization](https://system76.com/support/virtualization/)) is Boxes-centric and predates COSMIC-as-guest guidance — treat the mmstick/Drakulix issue statements as the authoritative word.

## 9. Caveats specific to airlock's CP4 gate

1. **Host Alt+Tab theft.** The host also runs COSMIC and owns `Alt+Tab` (see the workspace-isolation record). Without keyboard grab, the host switcher fires when you Alt+Tab "into" the guest. virt-viewer/virt-manager's *Grab keyboard* policy (the `grab-keyboard` setting; [Red Hat BZ#1350390](https://bugzilla.redhat.com/show_bug.cgi?id=1350390)) captures host shortcuts while the console is focused — enable it (or use QEMU's documented `grab-on-hover` for the GTK UI, [QEMU invocation docs](https://qemu-project.gitlab.io/qemu/system/invocation.html)). The guest's own Alt+Tab handling (which is what airlock takes over) is unaffected either way.
2. **Renderer divergence.** On real COSMIC hardware, libcosmic/wgpu apps (including airlock and cosmic-launcher) typically run the **Vulkan** backend; in a virgl VM they run the **GL** backend (wgpu auto-fallback; #3831/#3844). CP4 in a VM therefore does *not* exercise airlock's wgpu-Vulkan codepath — only the GL one. The compositor side is identical (GLES either way).
3. **DRM-master log noise.** *"Unable to become drm master, assuming unprivileged mode"* / *"Failed to initialize drm lease global"* appear in cosmic-comp logs in VMs and on some bare-metal setups (#3844, #3782) — non-fatal; do not fail CP4 on them.
4. **Input plumbing.** virtio-keyboard delivers host key events as-is; SPICE adds spice-vdagent for clipboard/resize ([System76 virtualization article](https://system76.com/support/virtualization/)). Key-repeat behavior through SPICE for the hold-Alt-cycle-Tab gesture was **not verified from primary sources** — Wayland key repeat is client-side, so guest-side repeat should follow the guest's repeat-rate settings, but if gesture timing feels off, compare against the live-USB path (below) before blaming airlock.
5. **Resources.** Give the guest ≥ 8 GB RAM / 4 vCPUs (System76's own VM sizing guidance: 8 GB/60 GB; [virtualization article](https://system76.com/support/virtualization/)) — airlock is a Rust build target as well as a test subject. Bump virtio video memory well above the 16 MiB default for a 1080p+ desktop [practice, not primary-sourced].
6. **Try-on-hardware escape hatch.** System76 officially suggests live-booting without installing: the installer's **"Try Demo Mode"** *"to continue testing Pop!_OS in the live environment"* ([*Installing Pop!_OS*](https://system76.com/support/install-pop/); live-disk creation at [*pop-live-disk*](https://system76.com/support/pop-live-disk/)). A live USB on real hardware is the cheap fallback whenever VM results are ambiguous (keyboard feel, grab behavior, GPU timing).

## 10. Recommended recipe (concrete)

**Hypervisor/stack:** KVM + libvirt + **virt-manager** (`sudo apt install virt-manager qemu-system-x86 libvirt-daemon-system` — host has KVM and kernel 7.2.6 ready; virglrenderer 1.0.0 satisfies virgl).

**VM definition (virt-manager):**
- Firmware: **UEFI** (OVMF x86-64), Secure Boot off; machine type q35.
- CPUs: host-passthrough or host-model, 4 vCPU; RAM 8 GB; disk virtio, ≥ 60 GB.
- Display: **SPICE**, OpenGL enabled, listen type "none" (local).
- Video: model **VirtIO**, **3D acceleration ✓ (virgl)**, video RAM ≥ 64 MiB.
- NIC/disk/input: virtio everywhere; install `spice-vdagent` in the guest.
- Guest ISO: **Pop!_OS 24.04 LTS (Intel/AMD build)** from [system76.com/download-pop](https://system76.com/download-pop/) — stable COSMIC; after install, update to pick up the rolling epoch point releases.
- In-guest env for determinism: leave wgpu auto-fallback as-is (it will pick GL); set `WGPU_BACKEND=gl` explicitly if any app tries llvmpipe-Vulkan first (mmstick's suggestion in #3844).
- Workflow for CP4: libvirt **snapshots** before each install/uninstall/kill-daemon pass; enable **Grab keyboard** in the viewer during Alt+Tab exercises.

**What this pass can validate:** real cosmic-comp session semantics — layer-surface rendering and keyboard grabs on the guest compositor, workspace membership/switching, Alt+Tab takeover install/uninstall via `system_actions` config, daemon kill mid-gesture and clean restore, packaging/systemd units on genuine Pop 24.04.

**What it cannot validate:** the wgpu **Vulkan** renderer path (VM runs GL), performance/RSS/latency characteristics representative of hardware (virgl adds overhead and #3844 shows measurable fluidity loss), multi-monitor/workspace-per-output topologies, and real-hardware keyboard/repeat feel — for those, use the live-USB Try Demo Mode path or a VFIO passthrough rig.

---

## 11. Sources

**System76 (primary):**

1. Download Pop!_OS — editions, ISOs, minimums (4 GB RAM / 16 GB storage / Secure Boot off; kernel 7.0.9, Mesa 25.1.5) — https://system76.com/download-pop/
2. Blog: The Latest Updates to COSMIC Epoch 1 (Epoch 1 released 2025-12-11; 1.0.1/1.0.2) — https://system76.com/blog/post/cosmic-epoch-1-updates
3. COSMIC page (Pop!_OS + Fedora COSMIC Spin) — https://system76.com/cosmic
4. Support: Virtualization (GNOME Boxes, spice-vdagent/webdavd, snapshots, sizing) — https://system76.com/support/virtualization/
5. Support: Installing Pop!_OS ("Try Demo Mode"; Secure Boot) — https://system76.com/support/install-pop/
6. Support: Live disk creation — https://system76.com/support/pop-live-disk/
7. Support: Repair the Bootloader (systemd-boot on Pop 18.04+) — https://system76.com/support/bootloader/

**pop-os GitHub (primary):**

8. cosmic-epoch releases (`epoch-1.11.0`, 2026-10-20, latest) — https://github.com/pop-os/cosmic-epoch/releases
9. cosmic-epoch README (component list; distro installs; epoch tag scheme) — https://github.com/pop-os/cosmic-epoch
10. cosmic-epoch#3844 — COSMIC in virtio-gpu/virgl VM: GL-only, no Vulkan libs, works; WGPU_BACKEND=gl; DRM-master noise — https://github.com/pop-os/cosmic-epoch/issues/3844
11. cosmic-epoch#3831 — GLES fallback request; Drakulix "cosmic-comp currently only has an OpenGL ES backend…"; mmstick "wgpu automatically falls back to OpenGL…" — https://github.com/pop-os/cosmic-epoch/issues/3831
12. cosmic-epoch#2215 — Vulkan apps silently on llvmpipe — https://github.com/pop-os/cosmic-epoch/issues/2215
13. cosmic-comp#136 — alpha-era virtio blank screen; XRGB8888 fix; smithay#1070; accelerated virtio works; QXL fallback; "virt-manager is needed" (mmstick) — https://github.com/pop-os/cosmic-comp/issues/136
14. cosmic-comp Cargo.toml (smithay 0.7.0: renderer_glow/renderer_pixman/backend_vulkan) — https://github.com/pop-os/cosmic-comp/blob/master/Cargo.toml
15. cosmic-epoch#3306, #2657 — Parallels tiling quirk; VirtualBox fullscreen bug — https://github.com/pop-os/cosmic-epoch/issues/3306 · https://github.com/pop-os/cosmic-epoch/issues/2657

**QEMU / Mesa / kernel (primary):**

16. QEMU VirtIO GPU docs (2D default = llvmpipe/lavapipe; virgl requirements; venus via `hostmem/blob/venus`; host Linux 6.13+ for blobs; guest Linux 5.16+/Mesa 24.2+ for Vulkan) — https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html
17. QEMU invocation docs (`grab-on-hover`) — https://qemu-project.gitlab.io/qemu/system/invocation.html
18. Mesa Venus driver docs (host Vulkan 1.1 + VK_KHR_external_memory_fd; guest 5.16+; tested hosts incl. Lavapipe 22.1+) — https://docs.mesa3d.org/drivers/venus.html
19. Mesa VMware/SVGA3D docs (guest GL driver) — https://docs.mesa3d.org/drivers/svga3d.html
20. Kernel VFIO docs (device passthrough framework, IOMMU requirements) — https://docs.kernel.org/driver-api/vfio.html

**Hypervisor docs & trackers (primary):**

21. virt-manager#362 — Venus RFE, open/blocked on libvirt — https://github.com/virt-manager/virt-manager/issues/362
22. libvirt#638 — libvirt Venus support tracker — https://gitlab.com/libvirt/libvirt/-/issues/638
23. GNOME Boxes#288 / #696 — 3D acceleration option / broken accel — https://gitlab.gnome.org/GNOME/gnome-boxes/-/issues/288 · https://gitlab.gnome.org/GNOME/gnome-boxes/-/issues/696
24. flathub org.gnome.Boxes#77 — 3D option missing until Boxes 49 — https://github.com/flathub/org.gnome.Boxes/issues/77
25. Red Hat BZ#1856717 — Boxes 3D accel requires virglrenderer — https://bugzilla.redhat.com/show_bug.cgi?id=1856717
26. Red Hat BZ#1350390 — virt-viewer `grab-keyboard` behavior — https://bugzilla.redhat.com/show_bug.cgi?id=1350390
27. VirtualBox manual ch. 4 (§4.5 experimental OpenGL/D3D8-9; no Vulkan) — https://www.virtualbox.org/manual/ch04.html
28. VMware Workstation Pro 25H2 tech docs — host GL 2.0/3.3/4.5 for D3D guests; guest "OpenGL 4.3 (and earlier)"; host-side Vulkan renderer — https://techdocs.broadcom.com/us/en/vmware-cis/desktop-hypervisors/workstation-pro/25H2/using-vmware-workstation-pro/configuring-and-managing-virtual-machines/configure-display-settings-for-a-virtual-machine/prepare-the-host-system-to-use-accelerated-3d-graphics.html

**Community (marked, non-load-bearing):**

29. r/pop_os — "COSMIC + VMs" (staff: enable GPU accel) — https://www.reddit.com/r/pop_os/comments/1e2fsc2/cosmic_vms
30. r/pop_os — 24.04 review thread (VM GPU struggles) — https://www.reddit.com/r/pop_os/comments/1po4g42/pop_os_2404_full_review_the_cosmic_desktop_is_here
31. 9to5linux — COSMIC 1.9 release + VM responsiveness comment — https://9to5linux.com/cosmic-1-9-desktop-environment-released-with-cosmic-viewer-and-cosmic-osk
32. Himmelwright — VirtIO 3D acceleration in virt-manager — https://ryan.himmelwright.net/post/virtio-3d-vms
33. AskUbuntu 1348975 — enabling OpenGL/3D accel in virt-manager — https://askubuntu.com/questions/1348975/how-to-use-opengl-3d-acceleration-in-virt-manager-with-ubuntu
34. Phoronix forums — Ubuntu disabling Boxes/virt-manager 3D accel — https://www.phoronix.com/forums/forum/software/distributions/1320743-ubuntu-22-04-lts-disables-3d-acceleration-for-guest-vms-with-gnome-boxes-virt-manager/page3

**Host inspection (this machine, 2026-10-24):** kernel 7.2.6-76070206-generic; `/dev/kvm` present; Pop!_OS 24.04 LTS; apt candidates qemu-system-x86 1:8.2.2+ds-0ubuntu1.18, virt-manager 1:4.1.0-3ubuntu0.1, libvirglrenderer1 1.0.0-1ubuntu2.
