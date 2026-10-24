#!/usr/bin/env bash
# Host-side plumbing for the CP4 VM rehearsal (docs/vm-development.md has the
# manual walkthrough this automates; just recipes below delegate here).
#
# Every setting is an environment variable; guest-side actions run through
# the QEMU guest agent (so the guest needs `qemu-guest-agent` installed
# once); file transfer rides the VM's NAT bridge over a short-lived HTTP
# server that stops when the transfer ends. Variables, commands, and the
# workflow order: `scripts/vm.sh help`. `scripts/vm.sh status` checks every
# precondition and reports ok/missing instead of failing.
set -euo pipefail

VM_NAME="${VM_NAME:-PopOs24.04}"
VM_USER="${VM_USER:-jpt-virt}"
BRIDGE_IP="${BRIDGE_IP:-192.168.122.1}"
PORT="${PORT:-8000}"
# Resolved per-run by resolve_guest() unless explicitly set above.
VM_HOME="${VM_HOME:-}"
VM_BIN="${VM_BIN:-}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="$REPO/target/release/airlock"
URL="http://$BRIDGE_IP:$PORT/airlock"
SERVER_PID=""

die() { echo "vm: error: $*" >&2; exit 1; }
note() { echo "vm: $*"; }

# --- host-side guards -------------------------------------------------------

require() {  # require <command> <install hint>
    command -v "$1" >/dev/null 2>&1 || die "'$1' not found on this host — $2"
}

check_host() {
    require virsh "sudo apt install libvirt-clients (or the full virt-manager stack)"
    require python3 "sudo apt install python3"
    require curl "sudo apt install curl"
    require ip "sudo apt install iproute2"
    require ss "sudo apt install iproute2"
    virsh -c qemu:///system version >/dev/null 2>&1 ||
        die $'cannot reach qemu:///system\n  → permission denied usually means: sudo usermod -aG libvirt "$USER", then log out and back in'
}

domain_state() { virsh -q domstate "$VM_NAME" 2>/dev/null; }

domain_must_exist() {
    [[ -n "$(domain_state)" ]] ||
        die "no libvirt domain '$VM_NAME' — check 'virsh list --all', or set the name: VM_NAME=<name> just vm-push"
}

domain_must_run() {
    domain_must_exist
    local state; state="$(domain_state || true)"
    [[ "$state" == "running" ]] ||
        die "VM '$VM_NAME' is '$state', not running — start it in virt-manager first (the guest agent and the transfer both need a live VM)"
}

check_bridge() {
    ip -4 addr show 2>/dev/null | grep -q "inet $BRIDGE_IP/" ||
        die "this host has no $BRIDGE_IP address — is the VM NAT network up? (sudo virsh net-start default; or override BRIDGE_IP=)"
}

# --- guest-agent plumbing ---------------------------------------------------

agent_call() { virsh -q qemu-agent-command "$VM_NAME" "$1"; }

# Runs a shell command in the guest as root, forwarding stdout, stderr, and
# the exit code.
guest_exec() {
    local cmd="$1" req reply pid exited
    req="$(CMD="$cmd" python3 -c '
import json, os
print(json.dumps({"execute": "guest-exec",
                  "arguments": {"path": "/bin/sh", "arg": ["-c", os.environ["CMD"]],
                                "capture-output": True}}))')"
    reply="$(agent_call "$req" 2>/dev/null)" ||
        die "guest agent unreachable in '$VM_NAME' — is the VM running, and is qemu-guest-agent installed in it? (sudo apt install qemu-guest-agent, inside the VM)"
    pid="$(printf '%s' "$reply" | python3 -c 'import json,sys; print(json.load(sys.stdin)["return"]["pid"])')"
    while :; do
        reply="$(agent_call "{\"execute\":\"guest-exec-status\",\"arguments\":{\"pid\":$pid}}")" \
            || die "lost the guest agent mid-command (is the VM still running?)"
        exited="$(printf '%s' "$reply" | python3 -c 'import json,sys; print(json.load(sys.stdin)["return"]["exited"])')"
        [[ "$exited" == "True" ]] && break
        sleep 0.2
    done
    printf '%s' "$reply" | python3 -c '
import base64, json, sys
r = json.load(sys.stdin)["return"]
sys.stdout.write(base64.b64decode(r.get("out-data") or "").decode("utf-8", "replace"))
sys.stderr.write(base64.b64decode(r.get("err-data") or "").decode("utf-8", "replace"))
sys.exit(r.get("exitcode") or 0)'
}

guest_exec_as_user() { guest_exec "sudo -u '$VM_USER' -H $*"; }

agent_alive() { agent_call '{"execute":"guest-ping"}' >/dev/null 2>&1; }

# Validates VM_USER against the guest's own account database and resolves
# VM_HOME/VM_BIN from it (instead of assuming /home/<user>).
resolve_guest() {
    domain_must_run
    agent_alive ||
        die "guest agent not responding in '$VM_NAME' — install it inside the VM once: sudo apt install qemu-guest-agent"
    local passwd home
    passwd="$(guest_exec "getent passwd '$VM_USER'")" ||
        die "guest user '$VM_USER' does not exist in the VM — set the right one: VM_USER=<name> just vm-push"
    home="$(printf '%s' "$passwd" | cut -d: -f6)"
    [[ -n "$home" ]] || die "could not read the home directory of guest user '$VM_USER'"
    VM_HOME="${VM_HOME:-$home}"
    VM_BIN="${VM_BIN:-$VM_HOME/.local/bin/airlock}"
}

guest_binary_must_exist() {
    guest_exec "test -x '$VM_BIN'" ||
        die "no airlock binary staged at $VM_BIN in the guest — run 'just vm-push' first"
}

# --- transfer ---------------------------------------------------------------

vm_ip() {
    virsh -q domifaddr "$VM_NAME" 2>/dev/null | awk '$3 == "ipv4" {print $4}' | cut -d/ -f1 | head -n1
}

serve_up() {
    curl -sf -o /dev/null --max-time 2 "$URL" && return 0   # reuse existing
    ss -ltn 2>/dev/null | grep -q ":$PORT " &&
        die "port $PORT is busy with something that is not this transfer — set PORT=<other>"
    [[ -x "$BINARY" ]] || die "no release binary at $BINARY — build first (just release)"
    python3 -m http.server "$PORT" --bind "$BRIDGE_IP" --directory "$(dirname "$BINARY")" \
        >/dev/null 2>&1 &
    SERVER_PID=$!
    local _
    for _ in $(seq 1 25); do
        curl -sf -o /dev/null --max-time 1 "$URL" && return 0
        sleep 0.2
    done
    die "transfer server failed to come up on $BRIDGE_IP:$PORT"
}

serve_down() {
    [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
}

# --- commands ---------------------------------------------------------------

cmd_ip() {
    check_host
    domain_must_run
    local ip; ip="$(vm_ip || true)"
    [[ -n "$ip" ]] || die "no guest IP found on the NAT network — is the VM's NIC attached to the 'default' network?"
    echo "$ip"
}

cmd_push() {
    check_host
    check_bridge
    resolve_guest
    note "building release binary"
    (cd "$REPO" && cargo build --release --quiet)
    local host_sha guest_sha downloader
    host_sha="$(sha256sum "$BINARY" | cut -d' ' -f1)"
    serve_up
    trap serve_down EXIT
    local gip; gip="$(vm_ip || true)"
    note "transferring to $VM_NAME at ${gip:-unknown IP} → $VM_BIN"
    downloader="$(guest_exec "command -v curl >/dev/null && echo curl || (command -v wget >/dev/null && echo wget)")" \
        || die "the guest has neither curl nor wget — install one inside the VM: sudo apt install curl"
    [[ -n "$downloader" ]] ||
        die "the guest has neither curl nor wget — install one inside the VM: sudo apt install curl"
    case "$downloader" in
        curl) guest_exec "curl -sf -o /root/airlock '$URL'" ;;
        wget) guest_exec "wget -q -O /root/airlock '$URL'" ;;
    esac || die "guest could not download $URL"
    guest_sha="$(guest_exec "sha256sum /root/airlock" | awk '{print $1}')" \
        || die "could not checksum the downloaded file inside the guest"
    [[ "$guest_sha" == "$host_sha" ]] ||
        die "checksum mismatch — host $host_sha, guest $guest_sha; the transfer was corrupted, retry"
    guest_exec "install -D -o '$VM_USER' -g '$VM_USER' -m 755 /root/airlock '$VM_BIN' && rm /root/airlock"
    guest_exec "test -x '$VM_BIN'" || die "staging verification failed — $VM_BIN is not executable in the guest"
    serve_down
    trap - EXIT
    note "staged $VM_BIN (sha256 $host_sha)"
    note "first staging only: log out/in inside the VM so the session PATH picks up ~/.local/bin"
    note "next: just vm-snapshot pre-airlock && just vm-install"
}

cmd_install() {
    check_host
    resolve_guest
    guest_binary_must_exist
    guest_exec_as_user "'$VM_BIN' install"
}

cmd_uninstall() {
    check_host
    resolve_guest
    guest_binary_must_exist
    guest_exec_as_user "'$VM_BIN' install --uninstall"
}

cmd_snapshot() {
    check_host
    domain_must_run
    local name="${1:-checkpoint}"
    [[ -n "$name" ]] || die "snapshot name must not be empty"
    virsh snapshot-create-as "$VM_NAME" "$name" >/dev/null ||
        die "snapshot '$name' failed (live snapshots need a running VM and a qcow2 disk)"
    note "snapshot '$name' taken"
}

cmd_restore() {
    check_host
    domain_must_exist
    local name="${1:-}"
    [[ -n "$name" ]] || die "usage: vm.sh restore <name> — see 'vm.sh snapshots'"
    virsh snapshot-revert "$VM_NAME" "$name" --running ||
        die "revert to '$name' failed — does it exist? (scripts/vm.sh snapshots)"
    note "reverted to '$name' (VM running)"
}

cmd_snapshots() {
    check_host
    domain_must_exist
    virsh snapshot-list "$VM_NAME" --name 2>/dev/null | sed '/^$/d'
}

# Doctor: runs every precondition and reports ok/missing instead of failing.
cmd_status() {
    local state ip
    echo "config:"
    echo "  domain $VM_NAME · user $VM_USER · bridge $BRIDGE_IP:$PORT"
    echo "  binary ${VM_BIN:-<resolved from the guest at push/install>}"
    echo "host:"
    if command -v virsh >/dev/null 2>&1; then
        echo "  ok        virsh on PATH"
        if virsh -c qemu:///system version >/dev/null 2>&1; then
            echo "  ok        qemu:///system reachable"
            state="$(domain_state || true)"
            if [[ -n "$state" ]]; then
                echo "  ok        domain '$VM_NAME' exists ($state)"
                if [[ "$state" == "running" ]]; then
                    ip="$(vm_ip || true)"
                    if [[ -n "$ip" ]]; then
                        echo "  ok        guest IP $ip"
                    else
                        echo "  MISSING   guest IP — no NIC on the 'default' network?"
                    fi
                    if agent_alive; then
                        echo "  ok        guest agent responding"
                        if guest_exec "getent passwd '$VM_USER'" >/dev/null 2>&1; then
                            echo "  ok        guest user '$VM_USER' exists"
                        else
                            echo "  MISSING   guest user '$VM_USER' — VM_USER=<name> just vm-push"
                        fi
                    else
                        echo "  MISSING   guest agent — sudo apt install qemu-guest-agent (inside the VM)"
                    fi
                else
                    echo "  MISSING   VM not running — start it in virt-manager"
                fi
            else
                echo "  MISSING   domain '$VM_NAME' — VM_NAME=<name> just vm-push"
            fi
        else
            echo "  MISSING   qemu:///system — sudo usermod -aG libvirt \"\$USER\", then log out/in"
        fi
    else
        echo "  MISSING   virsh — sudo apt install libvirt-clients"
    fi
    if ip -4 addr show 2>/dev/null | grep -q "inet $BRIDGE_IP/"; then
        echo "  ok        bridge IP $BRIDGE_IP present"
    else
        echo "  MISSING   bridge IP $BRIDGE_IP — sudo virsh net-start default (or BRIDGE_IP=)"
    fi
    if [[ -x "$BINARY" ]]; then
        echo "  ok        release binary built"
    else
        echo "  MISSING   release binary — just release"
    fi
    echo "snapshots:"
    if virsh -c qemu:///system snapshot-list "$VM_NAME" --name >/dev/null 2>&1; then
        local names; names="$(cmd_snapshots 2>/dev/null || true)"
        [[ -n "$names" ]] && sed 's/^/  /' <<<"$names" || echo "  (none)"
    else
        echo "  (unavailable — see host checks above)"
    fi
}

cmd_help() {
    cat <<'EOF'
vm — CP4 rehearsal plumbing (scripts/vm.sh; guide: docs/vm-development.md)

Variables (override per run — forms below):
  VM_NAME    libvirt domain                  default PopOs24.04
  VM_USER    desktop user inside the guest   default jpt-virt
  VM_HOME    guest home directory            default: looked up in the guest's /etc/passwd
  VM_BIN     guest binary path               default $VM_HOME/.local/bin/airlock
  BRIDGE_IP  host IP on the VM NAT bridge    default 192.168.122.1
  PORT       transfer port                   default 8000

Override forms (VM_USER shown; same for every variable):
  VM_USER=alice just vm-push          environment prefix
  just vm-user=alice vm-push          assignment, before the recipe
  just --set vm-user alice vm-push    same, long form

Workflow, in order:
  just vm-status                      preflight doctor — run this first
  just vm-push                        build → transfer → sha256-verify → stage in guest
  just vm-snapshot pre-airlock        clean point before the first install
  just vm-install                     take over Alt+Tab inside the VM (then rehearse)
  just vm-uninstall                   restore the stock switcher
  just vm-restore pre-airlock         back to the snapshot

Live values and preconditions: just vm-status
Full guide (setup, checklist, troubleshooting): docs/vm-development.md
EOF
}

case "${1:-help}" in
    ip)         cmd_ip ;;
    push)       cmd_push ;;
    install)    cmd_install ;;
    uninstall)  cmd_uninstall ;;
    snapshot)   cmd_snapshot "${2:-}" ;;
    restore)    cmd_restore "${2:-}" ;;
    snapshots)  cmd_snapshots ;;
    status)     cmd_status ;;
    help|*)     cmd_help ;;
esac
