# Developing and testing in the COSMIC VM

Airlock takes over Alt+Tab on the user's real desktop, so honest end-to-end
testing means a real COSMIC session — ideally not the one you are typing in.
The rehearsal VM runs a stock Pop!_OS 24.04 (COSMIC) guest under KVM /
virt-manager on this host, with snapshots to make every install/uninstall
pass repeatable from a clean point.

The research behind every setting here (why virt-manager, why virgl, why not
Venus/VFIO/Boxes) lives in
[docs/research/cosmic-vm-environment.md](research/cosmic-vm-environment.md).

## What the VM proves — and what it can't

- ✅ Proves: install takes over Alt+Tab on a stock system, the switcher
  gesture works against a real compositor, crash paths survive (daemon
  kill, missing binary), uninstall restores byte-exactly.
- ❌ Can't prove: gesture *feel* (SPICE keyboard is not real keys), the
  Vulkan render path (a virgl guest runs wgpu on its GL backend), and
  multi-monitor layouts. Those need bare metal.

## Quick reference

`just vm-help` prints every variable, the override forms, and the run
order; `just vm-status` is the live preflight doctor. This page is the full
guide.

## One-time host setup

```bash
sudo apt install virt-manager          # pulls qemu-system-x86, libvirt-daemon-system, libvirt-clients
sudo usermod -aG libvirt "$USER"       # then LOG OUT / IN — groups only apply to new sessions
sudo virsh net-autostart default && sudo virsh net-start default
```

Verify: `virsh -c qemu:///system version` prints versions without a
password prompt. Only the `libvirt` group is needed — its daemon handles
`/dev/kvm` itself; you do not need the `kvm` group.

## One-time VM creation

In virt-manager → New VM:

| Setting | Value | Why |
|---|---|---|
| Install media | **Pop!_OS 24.04 LTS — Intel/AMD** ISO | every 24.04 ISO ships COSMIC; NVIDIA = drivers for hardware the VM does not have; ARM = wrong CPU. Checksum it against system76.com/download-pop |
| "Operating system" dropdown | **Ubuntu 24.04 LTS** | the dropdown is the libosinfo catalog, which lags; Pop is Ubuntu-based and this only sets defaults |
| RAM / CPUs | as-built: 4096 MiB / 4 | the [research record](research/cosmic-vm-environment.md) recommends 8 GB / 4 vCPU for comfort — the guest is a Rust build target too |
| Disk | 40 GB qcow2 (as-built) | thin — only occupies what is used; 60 GB if you plan to build in the guest |
| Firmware | **UEFI (OVMF)**, Secure Boot off | Pop's installer is UEFI-only; the download page says Secure Boot off |
| Video | model **Virtio** + **"3D acceleration" ticked** | the virgl checkbox — without it COSMIC renders in software and test results are contaminated |
| Disk bus / NIC | VirtIO (defaults) | |

The 3D checkbox is **not** in the main wizard — pick "Customize
configuration before install" and set it there.

After install: run Pop's updates once and reboot.

## One-time guest setup

Inside the VM:

```bash
sudo apt install qemu-guest-agent
```

This unlocks `just vm-push` / `vm-install` / `vm-uninstall`, which drive
the guest through libvirt's agent channel — no ssh, no keys.

## The per-build loop

From the repo on the host:

```bash
just vm-status                       # preflight: everything ok?
just vm-push                         # build → transfer over NAT bridge → sha256-verify → stage ~/.local/bin
just vm-snapshot pre-airlock         # clean point before the first install
just vm-install                      # Alt+Tab now runs airlock inside the VM
# ... rehearse (checklist below) ...
just vm-uninstall                    # stock switcher restored
just vm-restore pre-airlock          # optional: rewind to the snapshot
```

After the **first** staging only, log out/in inside the VM — `.profile`
stamps the session PATH the compositor uses to find `airlock alt-tab`.
Verify with `which airlock` in the guest. Later pushes to the same path
need no re-login.

### Manual bootstrap (before the guest agent exists)

```bash
# host
python3 -m http.server 8000 --bind 192.168.122.1 --directory target/release
# guest
curl -O http://192.168.122.1:8000/airlock && sha256sum airlock   # vs host: sha256sum target/release/airlock
mkdir -p ~/.local/bin && mv airlock ~/.local/bin/ && chmod +x ~/.local/bin/airlock
```

## Variables

Every setting is an environment variable. Override any of them per run:

```
VM_USER=alice just vm-push          environment prefix
just vm-user=alice vm-push          assignment, before the recipe
just --set vm-user alice vm-push    same, long form
```

| Variable | Meaning | Default |
|---|---|---|
| `VM_NAME` | libvirt domain | `PopOs24.04` |
| `VM_USER` | desktop user inside the guest | `jpt` |
| `VM_HOME` | guest home directory | looked up in the guest's `/etc/passwd` |
| `VM_BIN` | guest binary path | `$VM_HOME/.local/bin/airlock` |
| `BRIDGE_IP` | host IP on the VM NAT bridge | `192.168.122.1` |
| `PORT` | transfer port | `8000` |

## The rehearsal checklist

Run through in order; note anything odd. `just vm-restore pre-airlock`
rewinds to re-run from scratch.

- [ ] `just vm-install` — output says both keys now run airlock, and how to undo
- [ ] file check: `head -3 ~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions`
      → two `// airlock-v1` marker lines + body
- [ ] quick-tap Alt+Tab: switches to the MRU window, no overlay lingering
- [ ] hold Alt, tap Tab: overlay appears, cycles current-workspace windows only
- [ ] release Alt: commits the selection, overlay gone
- [ ] windows parked on *other* workspaces never appear in the list (the core feature)
- [ ] Shift+Alt+Tab cycles backwards
- [ ] Escape (or any focus/workspace change) dismisses cleanly
- [ ] mid-gesture kill: `pkill -9 airlock` while the overlay is up → overlay
      dies → the next Alt+Tab auto-spawns a fresh daemon
- [ ] missing-binary escape hatch: `rm ~/.local/bin/airlock`, press Alt+Tab
      (nothing happens — expected), then hand-restore: delete the two
      `// airlock-v1` marker lines and put the stock switcher lines back
      (`cosmic-launcher alt-tab` / `cosmic-launcher shift-alt-tab`), or
      restore the snapshot
- [ ] `just vm-uninstall` (binary back in place first) → stock switcher
      behaves exactly as before, markers gone
- [ ] byte-diff: restore the `pre-airlock` snapshot and compare
      `system_actions` against the post-uninstall copy — identical = pass

## Troubleshooting

`just vm-status` runs every precondition and reports ok/MISSING. The
common MISSING lines:

| `vm-status` says | Fix |
|---|---|
| `MISSING virsh` | `sudo apt install libvirt-clients` (or the full virt-manager stack) |
| `MISSING qemu:///system` | `sudo usermod -aG libvirt "$USER"`, then log out and back in |
| `MISSING domain '…'` | `virsh list --all` to find the name; `VM_NAME=<name> just vm-status` |
| `MISSING VM not running` | start it in virt-manager |
| `MISSING guest IP` | the VM's NIC is not on the `default` NAT network — attach it |
| `MISSING guest agent` | inside the VM: `sudo apt install qemu-guest-agent` |
| `MISSING guest user '…'` | `VM_USER=<name> just vm-push` |
| `MISSING bridge IP …` | `sudo virsh net-start default` (or override `BRIDGE_IP=`) |
| `MISSING release binary` | `just release` (`vm-push` builds it too) |

Runtime failures: a **checksum mismatch** means the transfer was corrupted
— retry. **Port busy** with something that is not this transfer: set
`PORT=<other>`. **Guest has neither curl nor wget**: `sudo apt install
curl` inside the VM. **Snapshot revert failed**: check it exists
(`virsh snapshot-list <VM_NAME>`).

Guest IP is DHCP and changes across reboots (`virsh domifaddr <VM_NAME>`
or `just vm-ip`); the host bridge address is fixed at 192.168.122.1.

## Teardown

Done with the VM forever: delete it in virt-manager and tick "delete
associated storage". Everything VM-related lives under
`/var/lib/libvirt/images/` — nothing on the host outside that plus the
four apt packages from host setup.
