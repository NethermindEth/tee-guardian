"""
VM monitoring service for Guardian nodes.

State Machine:
  Dev VM (no health_url):   STOPPED → BOOTING → SSH_ONLINE
  Guardian VM (health_url): STOPPED → BOOTING → BOOTSTRAPPING → LEARNER → CANDIDATE → FOLLOWER/LEADER
                                                     ↓
                                                  OFFLINE (was operational, now unreachable)

Check Pattern: Fast check first (SSH handshake or HTTP), fall back to pgrep on TDX host if failed.
"""

import atexit
import logging
import os
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from enum import Enum

import paramiko
import requests

from guardian_cli.utils import load_config, remote_execute

logger = logging.getLogger("guardian.vm_monitor")


class VMStatus(Enum):
    UNKNOWN = "unknown"
    STOPPED = "stopped"
    BOOTING = "booting"
    SSH_ONLINE = "ssh_online"
    BOOTSTRAPPING = "bootstrapping"
    LEARNER = "learner"
    CANDIDATE = "candidate"
    FOLLOWER = "follower"
    LEADER = "leader"
    OFFLINE = "offline"
    INVALID = "invalid"


_OPERATIONAL = {VMStatus.BOOTSTRAPPING, VMStatus.LEARNER, VMStatus.CANDIDATE, VMStatus.FOLLOWER, VMStatus.LEADER}

_HEALTH_STATUS_MAP = {
    "BOOTSTRAPPING": VMStatus.BOOTSTRAPPING,
    "LEARNER": VMStatus.LEARNER,
    "CANDIDATE": VMStatus.CANDIDATE,
    "FOLLOWER": VMStatus.FOLLOWER,
    "LEADER": VMStatus.LEADER,
    "INVALID": VMStatus.INVALID,
}


@dataclass
class VMState:
    vm_id: str
    ip: str
    health_url: str | None = None
    ssh_port: int = 22
    status: VMStatus = VMStatus.UNKNOWN
    error_message: str | None = None
    _status_changed: threading.Event = field(default_factory=threading.Event)
    _lock: threading.Lock = field(default_factory=threading.Lock)
    node_state: str | None = None
    term: int | None = None
    verified_peers: int | None = None


def _ip_to_mac(ip: str) -> str:
    """192.168.122.X → 52:54:00:12:34:{X:02x}"""
    return f"52:54:00:12:34:{int(ip.split('.')[-1]):02x}"


class VMMonitor:
    def __init__(self, vms: list[VMState], check_interval: float = 2.0, health_timeout: int = 3):
        self.vms = vms
        self.check_interval = check_interval
        self.health_timeout = health_timeout
        self._running = False
        self._thread: threading.Thread | None = None
        self._config: dict | None = None

    def start(self):
        if self._running:
            return
        self._running = True
        self._thread = threading.Thread(target=self._monitor_loop, daemon=True)
        self._thread.start()
        atexit.register(self.stop)

    def stop(self):
        self._running = False
        if self._thread:
            self._thread.join(timeout=5)

    def wait_for_status(self, vm: VMState, target: VMStatus | tuple[VMStatus, ...], timeout: int = 120):
        start = time.time()
        targets = (target,) if isinstance(target, VMStatus) else target
        while time.time() - start < timeout:
            with vm._lock:
                if vm.status in targets:
                    logger.info(f"{vm.vm_id} reached {vm.status.value} ({int(time.time() - start)}s)")
                    return
            vm._status_changed.wait(timeout=min(timeout - (time.time() - start), 2.0))
            vm._status_changed.clear()
        raise TimeoutError(f"{vm.vm_id} timeout after {timeout}s (status: {vm.status.value})")

    def wait_for_update(self, vm: VMState, timeout: int = 10):
        start = time.time()
        while time.time() - start < timeout:
            with vm._lock:
                if vm.status != VMStatus.UNKNOWN:
                    return
            vm._status_changed.wait(timeout=min(timeout - (time.time() - start), 1.0))
            vm._status_changed.clear()
        raise TimeoutError(f"{vm.vm_id} status update timeout after {timeout}s")

    def _monitor_loop(self):
        def check_vm(vm: VMState):
            with vm._lock:
                self._check_guardian_vm(vm) if vm.health_url else self._check_dev_vm(vm)

        while self._running:
            with ThreadPoolExecutor(max_workers=len(self.vms)) as executor:
                list(executor.map(check_vm, self.vms))
            time.sleep(self.check_interval)

    def _check_dev_vm(self, vm: VMState):
        if self._ssh_is_online(vm.ip, vm.ssh_port):
            self._set_status(vm, VMStatus.SSH_ONLINE)
        elif self._qemu_is_running(vm.ip):
            self._set_status(vm, VMStatus.BOOTING)
        else:
            self._set_status(vm, VMStatus.STOPPED)

    def _check_guardian_vm(self, vm: VMState):
        try:
            resp = requests.get(vm.health_url, timeout=self.health_timeout)
            if resp.status_code == 200:
                data = resp.json()
                trusted_peers = data.get("trusted_peers", [])
                vm.node_state, vm.term, vm.verified_peers = data.get("status"), data.get("term"), len(trusted_peers)
                self._set_status(vm, _HEALTH_STATUS_MAP.get(vm.node_state or "", VMStatus.UNKNOWN))
                return
        except Exception:
            pass
        # Health failed - clear stale health data
        vm.node_state, vm.term, vm.verified_peers = None, None, None
        # Check if QEMU is still running
        if self._qemu_is_running(vm.ip):
            self._set_status(
                vm, VMStatus.OFFLINE if vm.status in _OPERATIONAL else VMStatus.BOOTING, "health check failed"
            )
        else:
            self._set_status(vm, VMStatus.STOPPED)

    def _ssh_is_online(self, ip: str, port: int) -> bool:
        """Check if SSH is responding via paramiko handshake. Silent - no exceptions leak."""
        transport = None
        devnull = open(os.devnull, "w")
        old_stderr = sys.stderr
        try:
            sys.stderr = devnull
            transport = paramiko.Transport((ip, port))
            transport.connect()
            return True
        except Exception:
            return False
        finally:
            sys.stderr = old_stderr
            devnull.close()
            if transport:
                try:
                    transport.close()
                except Exception:
                    pass

    def _qemu_is_running(self, ip: str) -> bool:
        """Check if QEMU process exists on TDX host via pgrep."""
        if self._config is None:
            self._config = load_config()
        try:
            # Use [q]emu trick to prevent pgrep from matching itself
            result = remote_execute(
                self._config,
                f"pgrep -f '[q]emu.*mac={_ip_to_mac(ip)}' >/dev/null && echo yes || echo no",
                streaming=False,
                timeout=10,
                raise_on_nonzero=False,
            )
            return result.stdout.strip() == "yes"
        except Exception:
            return False

    def _set_status(self, vm: VMState, status: VMStatus, reason: str = ""):
        if vm.status != status:
            logger.debug(f"{vm.vm_id}: {vm.status.value} → {status.value}" + (f" ({reason})" if reason else ""))
            vm.status, vm.error_message = status, reason or None
            vm._status_changed.set()
