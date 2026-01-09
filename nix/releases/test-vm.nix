# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Copyright (C) 2026 Nethermind
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.


{ pkgs ? import (fetchTarball {
    # NixOS 25.11 release (commit: d355f89e) -- Updated: 2025-01-05
    url = "https://github.com/NixOS/nixpkgs/archive/d355f89e0014e51c9511298089d7ab55fd6f7056.tar.gz";
    sha256 = "01nxz6adq7lbpsidak1y2nrlvi8f7ds89zfn33bwfqfy4r2gkzhr";
  }) { system = "x86_64-linux"; }
}:

let
  inherit (pkgs) lib stdenv buildPackages fetchFromGitHub applyPatches fetchpatch;
  inherit (pkgs.linuxKernel) buildLinux;

  # Intel DCAP development package (headers + stub library for bindgen)
  intel-dcap = import ../packages/intel-dcap.nix {
    inherit lib stdenv fetchFromGitHub;
  };

  # Build with standard glibc toolchain - Nix provides reproducibility
  # The DCAP library is dynamically loaded at runtime on TDX hardware
  rustWorkspace = pkgs.rustPlatform.buildRustPackage {
    pname = "tee-kms-workspace";
    version = "1.0.0-preview";

    src = lib.cleanSourceWith {
      src = ../../.;
      filter = path: type:
        let
          baseName = baseNameOf path;
          relativePath = lib.removePrefix (toString ../../. + "/") (toString path);
        in
          # Include workspace files
          (baseName == "Cargo.toml") ||
          (baseName == "Cargo.lock") ||
          (baseName == "rust-toolchain.toml") ||
          # Include all crates directory contents
          (lib.hasPrefix "crates/" relativePath) ||
          # Include if it's the crates directory itself
          (baseName == "crates" && type == "directory");
    };

    cargoLock = {
      lockFile = ../../Cargo.lock;
      outputHashes = {
        # From guest-components repo (https://github.com/confidential-containers/guest-components v0.16.0)
        "attester-0.1.0" = "sha256-LpI/u6NJiBxMPB3hLNrgloSZzb4Q3DLmiY08LAM2n4g=";
        "crypto-0.1.0" = "sha256-LpI/u6NJiBxMPB3hLNrgloSZzb4Q3DLmiY08LAM2n4g=";
        # From trustee repo (https://github.com/confidential-containers/trustee v0.16.0)
        "eventlog-0.1.0" = "sha256-jiW4+t7LIA1csTViFOGHnCoEwWdVl9+CyNUmaYzHGCg=";
        "verifier-0.1.0" = "sha256-jiW4+t7LIA1csTViFOGHnCoEwWdVl9+CyNUmaYzHGCg=";
        # From SGXDataCenterAttestationPrimitives repo (https://github.com/intel/SGXDataCenterAttestationPrimitives DCAP_1.23)
        "intel-tee-quote-verification-rs-0.3.0" = "sha256-OmSm2ojfanJOekLqygxPDlVRovqF7dqiL8mQtOdbfFM=";
      };
    };

    nativeBuildInputs = with pkgs; [ pkg-config perl llvmPackages.libclang ];

    # OpenSSL for TLS, intel-dcap for DCAP headers and stub library
    buildInputs = with pkgs; [ openssl intel-dcap.dev ];

    cargoBuildFlags = [ "--workspace" "--bins" ];

    doCheck = false;  # Don't run tests during build (they require TDX hardware)

    enableParallelBuilding = true;

    # Bindgen needs libclang
    LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

    # Intel DCAP headers for bindgen (used by intel-tee-quote-verification-sys)
    # Also include glibc headers for standard library types (time.h, etc.)
    BINDGEN_EXTRA_CLANG_ARGS = "-I${intel-dcap.dev}/include -I${pkgs.glibc.dev}/include";

    meta = {
      description = "TEE KMS workspace binaries";
      platforms = [ "x86_64-linux" ];
    };
  };

  osRelease = pkgs.writeText "os-release" (lib.generators.toKeyValue {} {
    ID = "nixos";
    NAME = "TDX Guardian";
    VERSION = "1.0";
    PRETTY_NAME = "TDX Guardian 1.0";
    BUILD_ID = "tdx-guardian-1.0";
  });

  staticBusybox = pkgs.pkgsStatic.busybox.override { enableStatic = true; };

  staticOpenSSH = (pkgs.pkgsStatic.openssh.override {
    withPAM = false;
    withKerberos = false;
    withFIDO = false;
    withLdns = false;
  }).overrideAttrs (old: {
    configureFlags = old.configureFlags or [] ++ [
      "--with-privsep-user=nobody"
      "--with-privsep-path=/var/empty"
      "--disable-lastlog"
      "--disable-utmp"
      "--disable-utmpx"
      "--disable-wtmp"
      "--disable-wtmpx"
    ];
  });

  # Static Nebula binary for the initrd
  staticNebula = pkgs.pkgsStatic.nebula;

  # Nebula configuration for VMs (lighthouse addresses will be discovered via DNS/hardcoded)
  nebulaConfig = pkgs.writeText "nebula-config.yaml" (builtins.toJSON {
    pki = {
      ca = "/etc/nebula/ca.crt";
      cert = "/etc/nebula/host.crt";
      key = "/etc/nebula/host.key";
    };

    # Lighthouses - VMs discover these via static config
    # These IPs are the Nebula IPs of the host machines
    static_host_map = {
      "10.42.1.1" = [ "192.168.122.1:4242" ];  # TDX host (reachable via bridge)
      # Add more lighthouses as needed
    };

    lighthouse = {
      am_lighthouse = false;
      interval = 60;
      hosts = [ "10.42.1.1" ];
    };

    listen = {
      host = "0.0.0.0";
      port = 4242;
    };

    punchy = {
      punch = true;
      respond = true;
    };

    tun = {
      disabled = false;
      dev = "nebula1";
      drop_local_broadcast = false;
      drop_multicast = false;
      tx_queue = 500;
      mtu = 1300;
    };

    firewall = {
      outbound = [{ port = "any"; proto = "any"; host = "any"; }];
      inbound = [
        { port = "any"; proto = "any"; groups = [ "guardian-vms" ]; }
        { port = "any"; proto = "any"; groups = [ "tee-hosts" ]; }
        { port = "any"; proto = "any"; groups = [ "developers" ]; }
        { port = "any"; proto = "icmp"; host = "any"; }
      ];
    };

    logging = {
      level = "info";
      format = "json";
    };
  });

  # Default hl configuration for clean log viewing
  hlConfig = pkgs.writeText "hl-config.yaml" ''
    # Time format: YYYY-MM-DD HH:MM:SS (UTC, no timezone suffix)
    time-format: "%Y-%m-%d %H:%M:%S"
    time-zone: "UTC"

    theme: "universal"
    input-format: "logfmt"
    hide-empty-fields: true
    flatten: "always"
  '';

  # udhcpc script to configure network, fetch Nebula certs, and start Nebula
  # This runs as part of DHCP callback to minimize boot time
  udhcpcScript = pkgs.writeScript "udhcpc-default.script" ''
    #!/bin/sh
    CERT_SERVER="192.168.122.1:9999"
    NEBULA_DIR="/etc/nebula"

    case "$1" in
      bound|renew)
        # Configure network interface
        ip addr add "$ip/$mask" dev "$interface"
        ip link set "$interface" up
        [ -n "$router" ] && ip route add default via "$router" dev "$interface"

        # Configure DNS
        if [ -n "$dns" ]; then
          echo "# Generated by udhcpc" > /etc/resolv.conf
          for i in $dns; do
            echo "nameserver $i" >> /etc/resolv.conf
          done
        fi

        # Capture option 224 (node identity)
        mkdir -p /var/lib/dhcp
        echo "$opt224" > /var/lib/dhcp/option-224

        # === Nebula Certificate Fetch ===
        # Extract MAC address for cert server request
        MAC=$(ip link show "$interface" | grep ether | awk '{print $2}')
        
        mkdir -p "$NEBULA_DIR"
        
        # Fetch certs from host (retries for robustness)
        for attempt in 1 2 3; do
          RESPONSE=$(wget -q -O - "http://$CERT_SERVER/cert?mac=$MAC" 2>/dev/null) && break
          sleep 1
        done

        if [ -n "$RESPONSE" ]; then
          # Parse JSON and decode base64 (using busybox tools)
          # Response: {"cert": "base64...", "key": "base64...", "ca": "base64..."}
          echo "$RESPONSE" | sed 's/.*"cert":"\([^"]*\)".*/\1/' | base64 -d > "$NEBULA_DIR/host.crt"
          echo "$RESPONSE" | sed 's/.*"key":"\([^"]*\)".*/\1/' | base64 -d > "$NEBULA_DIR/host.key"
          echo "$RESPONSE" | sed 's/.*"ca":"\([^"]*\)".*/\1/' | base64 -d > "$NEBULA_DIR/ca.crt"
          chmod 600 "$NEBULA_DIR/host.key"
          
          # Signal that certs are ready
          touch "$NEBULA_DIR/.ready"
        fi
        ;;

      deconfig)
        ip addr flush dev "$interface"
        ;;
    esac
    exit 0
  '';

  initProgram = pkgs.runCommand "init" {
    nativeBuildInputs = [ pkgs.pkgsStatic.stdenv.cc ];
  } ''
    $CC -static -O2 -s -o $out ${pkgs.writeText "init.c" ''
      #include <sys/mount.h>
      #include <unistd.h>
      #include <stdlib.h>
      #include <stdio.h>
      #include <fcntl.h>
      #include <sys/stat.h>

      // Generate instance_id: 32 random bytes -> 64 hex chars
      static int generate_instance_id(void) {
          unsigned char buf[32];
          char hex[65];
          int fd = open("/dev/urandom", O_RDONLY);
          if (fd < 0 || read(fd, buf, 32) != 32) { close(fd); return -1; }
          close(fd);
          for (int i = 0; i < 32; i++) sprintf(hex + i*2, "%02x", buf[i]);
          hex[64] = 0;
          mkdir("/run/guardian", 0755);
          fd = open("/run/guardian/instance_id", O_WRONLY | O_CREAT | O_TRUNC, 0644);
          if (fd < 0 || write(fd, hex, 64) != 64) { close(fd); return -1; }
          close(fd);
          printf("Instance ID: %.16s...\n", hex);
          return 0;
      }

      int main() {
          printf("TDX Static Userspace\n");

          mount("proc", "/proc", "proc", 0, NULL);
          mount("sysfs", "/sys", "sysfs", 0, NULL);
          mount("dev", "/dev", "devtmpfs", 0, NULL);
          mount("tmpfs", "/tmp", "tmpfs", 0, NULL);
          mount("tmpfs", "/run", "tmpfs", 0, NULL);
          mount("tmpfs", "/var", "tmpfs", 0, NULL);
          mount("configfs", "/sys/kernel/config", "configfs", 0, NULL);

          // Generate instance_id before networking (needed for attestation)
          if (generate_instance_id() != 0) {
              printf("WARNING: Failed to generate instance_id\n");
          }

          printf("Network (DHCP)...\n");
          system("ip link set lo up");
          system("ip link set eth0 up");
          system("mkdir -p /dev/pts /var/lib/dhcp /var/lib/guardian /var/log /var/empty");
          mount("devpts", "/dev/pts", "devpts", 0, "mode=0620,gid=5");
          system("udhcpc -i eth0 -O 224 -q -s /etc/udhcpc/default.script");

          printf("SSH setup...\n");
          system("/usr/bin/ssh-keygen -t ed25519 -f /etc/ssh/ssh_host_ed25519_key -N \"\" >/dev/null 2>&1");
          system("/usr/bin/sshd -f /etc/ssh/sshd_config");

          // Start Nebula if certs were fetched during DHCP
          if (access("/etc/nebula/.ready", F_OK) == 0) {
              printf("Starting Nebula overlay network...\n");
              // Fork nebula into background
              if (fork() == 0) {
                  execl("/bin/nebula", "nebula", "-config", "/etc/nebula/config.yaml", NULL);
                  _exit(1);
              }
              // Brief pause to let nebula establish tunnel
              sleep(2);
              printf("Nebula started\n");
          } else {
              printf("WARNING: Nebula certs not found, skipping overlay network\n");
          }

          printf("Starting Guardian service (foreground with auto-restart)...\n");
          while(1) {
              int status = system("/bin/guardian");
              printf("Guardian exited with status %d, restarting in 5 seconds...\n", status);
              sleep(5);
          }
      }
    ''}
  '';

  initfs = pkgs.makeInitrd {
    name = "initfs";
    compressor = "gzip -9n";  # -n for deterministic compression (no timestamps)
    contents = [
      { object = initProgram; symlink = "/init"; }
      { object = "${staticBusybox}/bin/busybox"; symlink = "/bin/busybox"; }
      { object = staticOpenSSH; symlink = "/usr"; }
      { object = pkgs.cacert; symlink = "/etc/ssl"; }
      { object = udhcpcScript; symlink = "/etc/udhcpc/default.script"; }
      # Add all workspace binaries
      { object = "${rustWorkspace}/bin/tdx-info"; symlink = "/bin/tdx-info"; }
      { object = "${rustWorkspace}/bin/guardian"; symlink = "/bin/guardian"; }
      # Add hl log viewer
      { object = "${pkgs.hl-log-viewer}/bin/hl"; symlink = "/bin/hl"; }
      # Add Nebula for overlay networking
      { object = "${staticNebula}/bin/nebula"; symlink = "/bin/nebula"; }
      { object = nebulaConfig; symlink = "/etc/nebula/config.yaml"; }
    ];
    prepend = [
      (pkgs.runCommand "initrd-config" {
        nativeBuildInputs = [ pkgs.cpio pkgs.findutils ];
        SOURCE_DATE_EPOCH = "1";   # Ensure reproducible timestamps
        passAsFile = [ "passwdContents" "groupContents" "sshdConfigContents" ];
        passwdContents = ''
          root::0:0:root:/:/bin/sh
          nobody:x:65534:65534:nobody:/var/empty:/bin/false
        '';
        groupContents = ''
          root:x:0:
          nobody:x:65534:
        '';
        sshdConfigContents = lib.generators.toKeyValue {} {
          Port = 22;
          PermitRootLogin = "yes";
          PasswordAuthentication = "yes";
          PermitEmptyPasswords = "yes";
          PubkeyAuthentication = "no";
          HostKey = "/etc/ssh/ssh_host_ed25519_key";
          StrictModes = "no";
        };
      } ''
        export SOURCE_DATE_EPOCH=1
        mkdir -p staging/{bin,sbin,etc/ssh,etc/hl}

        cd staging/bin   # BusyBox symlinks
        for applet in $(${staticBusybox}/bin/busybox --list-full | grep -v '^sbin/'); do
          ln -sf busybox $(basename $applet)
        done
        cd -

        mkdir -p staging/sbin
        cd staging/sbin
        for applet in $(${staticBusybox}/bin/busybox --list-full | grep '^sbin/'); do
          ln -sf ../bin/busybox $(basename $applet)
        done
        cd -

        # System files from passAsFile
        cp "$passwdContentsPath" staging/etc/passwd
        cp "$groupContentsPath" staging/etc/group
        cp "$sshdConfigContentsPath" staging/etc/ssh/sshd_config

        # hl log viewer configuration
        cp ${hlConfig} staging/etc/hl/config.yaml

        # Package as uncompressed cpio with deterministic ordering
        (cd staging && find . -print0 | sort -z | cpio --null -o -H newc --reproducible) > $out
      '')
    ];
  };

  kernel = let
    version = "6.18";
    modDirVersion = "6.18.0-custom";

    src = pkgs.fetchurl {
      url = "mirror://kernel/linux/kernel/v6.x/linux-${version}.tar.xz";
      sha256 = "sha256-kQakYF2p4x/xdlnZWHgrgV+VkaswjQOw7iGq1sfc7Us=";
    };

    kernelConfig = (buildLinux rec {
      inherit version modDirVersion src;

      defconfig = "allnoconfig";
      enableCommonConfig = false;

      structuredExtraConfig = with lib.kernel; {
        # TDX Guest Requirements
        "64BIT" = yes;
        X86_64 = yes;
        CPU_SUP_INTEL = yes;
        X86_X2APIC = yes;

        HYPERVISOR_GUEST = yes;
        KVM_GUEST = yes;
        PARAVIRT = yes;

        ACPI = yes;
        EFI = yes;
        EFI_STUB = yes;

        INTEL_TDX_GUEST = yes;
        VIRT_DRIVERS = yes;
        TDX_GUEST_DRIVER = yes;  # This selects TSM_REPORTS and CONFIGFS_FS

        # Networking
        INET = yes;
        PACKET = yes;
        NETFILTER = yes;
        UNIX = yes;
        NET_CORE = yes;
        ETHERNET = yes;

        # Virtualization
        MMU = yes;
        PCI = yes;
        PCI_MSI = yes;
        NET = yes;
        NETDEVICES = yes;

        VIRTIO_MENU = yes;
        VIRTIO_PCI = yes;
        VIRTIO_BLK = yes;
        VIRTIO_NET = yes;
        VIRTIO_CONSOLE = yes;

        BLOCK = yes;
        BLK_DEV = yes;
        BLK_DEV_LOOP = yes;
        BLK_DEV_RAM = yes;
        BLK_DEV_INITRD = yes;

        # Filesystems
        DEVTMPFS = yes;
        TMPFS = yes;
        PROC_FS = yes;
        SYSFS = yes;
        EXT4_FS = yes;
        VFAT_FS = yes;

        INOTIFY_USER = yes;   # File notifications (for hl -F and other file watching tools)

        # Binary execution
        BINFMT_ELF = yes;
        BINFMT_SCRIPT = yes;

        # Console/TTY
        PRINTK = yes;
        TTY = yes;
        SERIAL_8250 = yes;
        SERIAL_8250_CONSOLE = yes;

        # Multicore & Performance Optimizations
        SMP = yes;
        NR_CPUS = lib.kernel.freeform "64";
        SCHED_MC = yes;
        SCHED_SMT = yes;
        SCHED_CORE = yes;

        QUEUED_SPINLOCKS = yes;
        QUEUED_RWLOCKS = yes;
        TREE_RCU = yes;

        IRQ_TIME_ACCOUNTING = yes;
        PREEMPT = yes;
        HZ_1000 = yes;
        NO_HZ_IDLE = yes;
        NUMA = yes;

        TRANSPARENT_HUGEPAGE = yes;
        COMPACTION = yes;

        # Deterministic build config
        BUILD_SALT = lib.kernel.freeform "";
        LOCALVERSION = lib.kernel.freeform "-custom";
        LOCALVERSION_AUTO = no;
        STRIP_ASM_SYMS = yes;

        # Namespace support (needed by dev tools)
        NAMESPACES = yes;
        UTS_NS = yes;
        PID_NS = yes;
        NET_NS = yes;
        USER_NS = yes;
        PERF_EVENTS = yes;
      };
    }).passthru.configfile;
  in
    (pkgs.callPackage "${pkgs.path}/pkgs/os-specific/linux/kernel/manual-config.nix" {}) {
      inherit version modDirVersion src;
      configfile = kernelConfig;

      # Override to disable modules
      config = {
        CONFIG_MODULES = "n";
        CONFIG_FW_LOADER = "y";
        CONFIG_RUST = "n";
      };
    };

  firmware = stdenv.mkDerivation {
    name = "edk2-tdx-debug";

    # Reproducible build environment
    SOURCE_DATE_EPOCH = "1";

    src = applyPatches {
      name = "edk2-tdx-src";
      src = fetchFromGitHub {
        owner = "tianocore";
        repo = "edk2";
        rev = "edk2-stable202505";
        fetchSubmodules = true;
        hash = "sha256-VuiEqVpG/k7pfy0cOC6XmY+8NBtU/OHdDB9Y52tyNe8=";
      };

      patches = [
        (fetchpatch {
          url = "https://src.fedoraproject.org/rpms/edk2/raw/08f2354cd280b4ce5a7888aa85cf520e042955c3/f/0021-Tweak-the-tools_def-to-support-cross-compiling.patch";
          hash = "sha256-E1/fiFNVx0aB1kOej2DJ2DlBIs9tAAcxoedym2Zhjxw=";
        })
        (fetchpatch {
          name = "fix-cross-compilation-antlr-dlg.patch";
          url = "https://github.com/tianocore/edk2/commit/a34ff4a8f69a7b8a52b9b299153a8fac702c7df1.patch";
          hash = "sha256-u+niqwjuLV5tNPykW4xhb7PW2XvUmXhx5uvftG1UIbU=";
        })
      ];

      postPatch = ''
        substituteInPlace BaseTools/Conf/tools_def.template --replace-fail \
          'DEFINE CLANGPDB_WARNING_OVERRIDES    = ' \
          'DEFINE CLANGPDB_WARNING_OVERRIDES    = -Wno-unneeded-internal-declaration '
      '';
    };

    nativeBuildInputs = with pkgs; [ python3 nasm perl bc acpica-tools ];
    buildInputs = [ pkgs.util-linuxMinimal pkgs.util-linuxMinimal.dev ];

    depsBuildBuild = [ buildPackages.stdenv.cc buildPackages.bash ];
    strictDeps = true;

    GCC5_X64_PREFIX = stdenv.cc.targetPrefix;
    PYTHON_COMMAND = "${buildPackages.python3}/bin/python3";

    env.NIX_CFLAGS_COMPILE = "-Wno-return-type" +
      lib.optionalString (stdenv.cc.isGNU) " -Wno-error=stringop-truncation";

    hardeningDisable = [ "format" "fortify" ];

    preConfigure = ''
      export WORKSPACE="$PWD"
      export PACKAGES_PATH="$WORKSPACE"
      export EDK_TOOLS_PATH="$WORKSPACE/BaseTools"

      export BUILD_CC="${buildPackages.stdenv.cc}/bin/gcc"
      export CC="${stdenv.cc}/bin/${stdenv.cc.targetPrefix}gcc"
      export CXX="${stdenv.cc}/bin/${stdenv.cc.targetPrefix}g++"
      export AR="${stdenv.cc.bintools.bintools}/bin/${stdenv.cc.targetPrefix}ar"
      export LD="${stdenv.cc.bintools.bintools}/bin/${stdenv.cc.targetPrefix}ld"
    '';

    buildPhase = ''
      make -C BaseTools -j$(nproc)

      for i in BaseTools/BinWrappers/PosixLike/*; do
        [ -f "$i" ] && chmod +x "$i" && patchShebangs --build "$i"
      done

      patchShebangs --build BaseTools/BinPipWrappers/PosixLike/* || true
      patchShebangs --build BaseTools/Source/Python/* || true

      source ./edksetup.sh BaseTools

      build -a X64 -b DEBUG -t GCC5 -p OvmfPkg/IntelTdx/IntelTdxX64.dsc -n $(nproc) \
        -D SECURE_BOOT_ENABLE=FALSE \
        -D BUILD_SHELL=TRUE
    '';

    installPhase = ''
      mkdir -p $out
      cp Build/IntelTdx/DEBUG_GCC5/FV/OVMF.fd $out/OVMF.fd
    '';

    enableParallelBuilding = true;
  };

  unifiedKernelImage = pkgs.runCommand "uki" {
    nativeBuildInputs = [ (pkgs.systemd.override { withUkify = true; }) ];
  } ''
    export SOURCE_DATE_EPOCH=1
    mkdir -p $out

    ukify build \
      --linux=${kernel}/bzImage \
      --initrd=${initfs}/initrd \
      --cmdline="init=/init console=ttyS0,115200 earlyprintk=serial debug ignore_loglevel rw" \
      --os-release=@${osRelease} \
      --output=$out/BOOTX64.EFI
  '';

  boot-image = stdenv.mkDerivation {
    name = "tdx-boot-image";
    dontUnpack = true;
    dontConfigure = true;
    dontPatch = true;
    nativeBuildInputs = with pkgs; [ mtools gptfdisk dosfstools util-linux ];

    MTOOLSRC = pkgs.writeText "mtoolsrc" ''
      mtools_skip_check=1
      mtools_fat_compatibility=1
    '';

    buildPhase = ''
      # Reproducibility: Force all timestamps to epoch
      export SOURCE_DATE_EPOCH=1
      export MTOOLSRC=$MTOOLSRC

      # Calculate required disk size based on UKI size with overhead
      UKI_SIZE=$(stat -c %s ${unifiedKernelImage}/BOOTX64.EFI)

      # Overhead breakdown:
      # - 1 MiB for GPT tables and alignment (2048 sectors before partition)
      # - 512 KiB for FAT32 metadata (FAT tables, root directory)
      # - 34 sectors (17 KiB) for backup GPT at end
      # - Round up to next 16 MiB boundary for clean sector alignment
      GPT_OVERHEAD=$((1024 * 1024))  # 1 MiB for primary GPT + alignment
      FAT_OVERHEAD=$((512 * 1024))   # 512 KiB for FAT32 structures
      BACKUP_GPT=$((34 * 512))       # 17 KiB for backup GPT

      MIN_SIZE=$((UKI_SIZE + GPT_OVERHEAD + FAT_OVERHEAD + BACKUP_GPT))
      ROUND_TO=$((16 * 1024 * 1024))
      DISK_SIZE=$(( ((MIN_SIZE + ROUND_TO - 1) / ROUND_TO) * ROUND_TO ))

      echo "=== Disk Size Calculation ==="
      echo "UKI size: $(numfmt --to=iec $UKI_SIZE)"
      echo "Disk size (16 MiB aligned): $(numfmt --to=iec $DISK_SIZE)"
      echo ""

      SECTOR_SIZE=512
      GPT_START=2048  # 1 MiB alignment
      TOTAL_SECTORS=$((DISK_SIZE / SECTOR_SIZE))
      GPT_END_RESERVED=34  # 33 sectors for backup GPT table + 1 for backup header
      USABLE_END=$((TOTAL_SECTORS - GPT_END_RESERVED - 1))
      PART_SECTORS=$((USABLE_END - GPT_START + 1))

      # Deterministic UUIDs for reproducibility
      DISK_UUID="12345678-1234-1234-1234-123456789abc"
      PART_UUID="87654321-4321-4321-4321-cba987654321"

      # Create disk image with calculated size
      dd if=/dev/zero of=boot.img bs=512 count=$TOTAL_SECTORS status=none

      # Verify size before proceeding
      ACTUAL_SIZE=$(stat -c %s boot.img)
      if [ "$ACTUAL_SIZE" -ne "$DISK_SIZE" ]; then
        echo "ERROR: Created image is $ACTUAL_SIZE bytes, expected $DISK_SIZE bytes" >&2
        exit 1
      fi

      # Create GPT partition table
      sgdisk --clear \
        --disk-guid=$DISK_UUID \
        --new=1:$GPT_START:$USABLE_END \
        --typecode=1:EF00 \
        --partition-guid=1:$PART_UUID \
        --change-name=1:"EFI System" \
        boot.img

      # Create FAT32 filesystem
      mkfs.vfat -F 32 -n ESP -i 12345678 --invariant --offset $GPT_START boot.img $PART_SECTORS

      ESP_OFFSET=$((GPT_START * SECTOR_SIZE))

      # Create EFI directory structure and copy UKI
      mmd -i "boot.img@@$ESP_OFFSET" ::/EFI ::/EFI/BOOT
      mcopy -i "boot.img@@$ESP_OFFSET" ${unifiedKernelImage}/BOOTX64.EFI ::/EFI/BOOT/

      echo ""
      echo "TDX UKI Boot Image: $(numfmt --to=iec $FINAL_SIZE)"
      echo "   Partition size: $(numfmt --to=iec $((PART_SECTORS * SECTOR_SIZE)))"
      echo "   Free space: $(numfmt --to=iec $((PART_SECTORS * SECTOR_SIZE - UKI_SIZE)))"
    '';

    installPhase = ''
      mkdir -p $out
      mv boot.img $out/boot.img
    '';
  };

  extract-measurements = pkgs.writeScriptBin "extract-tdx-measurements" ''
    #!${pkgs.bash}/bin/bash
    set -euo pipefail

    IP_OCTET=19
    SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
    ARTIFACT_DIR="$(dirname "$SCRIPT_DIR")"
    BOOT_IMAGE="$ARTIFACT_DIR/boot.img"
    FIRMWARE="$ARTIFACT_DIR/OVMF_DEBUG.fd"
    CACHE_DIR="/tmp/guardian-rtmr-cache"

    # Parse options
    while [[ $# -gt 0 ]]; do
      case $1 in
        --ip-octet)
          IP_OCTET="$2"
          shift 2
          ;;
        *)
          echo "Unknown option: $1" >&2
          exit 1
          ;;
      esac
    done

    # Verify boot image exists
    if [ ! -f "$BOOT_IMAGE" ]; then
      echo "ERROR: Boot image not found at $BOOT_IMAGE" >&2
      exit 1
    fi

    # Compute boot image hash for cache key
    BOOT_HASH=$(sha256sum "$BOOT_IMAGE" | cut -d' ' -f1)
    CACHE_FILE="$CACHE_DIR/$BOOT_HASH/measurements.txt"

    # Cache hit
    if [ -f "$CACHE_FILE" ]; then
      echo "Cache hit: ''${BOOT_HASH:0:16}..." >&2
      exit 0
    fi

    echo "Cache miss: ''${BOOT_HASH:0:16}... - booting extraction VM" >&2

    MAC="52:54:00:12:34:$(printf '%02x' "$IP_OCTET")"
    CID="$IP_OCTET"
    IP="192.168.122.$IP_OCTET"

    TEMP_DIR=$(mktemp -d)
    cleanup() {
      pkill -f "qemu-system.*tdx-extract-$IP_OCTET" 2>/dev/null || true
      rm -rf "$TEMP_DIR"
    }
    trap cleanup EXIT

    # Check for MAC address collision
    if pgrep -f "mac=$MAC" > /dev/null; then
      echo "ERROR: MAC $MAC already in use" >&2
      exit 1
    fi

    # Copy boot image (QEMU needs writable copy)
    cp "$BOOT_IMAGE" "$TEMP_DIR/boot.img"

    # Boot extraction VM
    echo "Booting VM at $IP (MAC: $MAC, CID: $CID)..." >&2
    qemu-system-x86_64 \
      -name "tdx-extract-$IP_OCTET" \
      -machine q35,accel=kvm,kernel-irqchip=split,confidential-guest-support=tdx,hpet=off \
      -cpu host -smp 4 -m 8G \
      -object '{"qom-type":"tdx-guest","id":"tdx","quote-generation-socket":{"type":"vsock","cid":"2","port":"4050"}}' \
      -bios "$FIRMWARE" \
      -drive file="$TEMP_DIR/boot.img",format=raw,if=virtio \
      -netdev bridge,id=net0,br=br0 \
      -device virtio-net-pci,netdev=net0,mac="$MAC" \
      -device vhost-vsock-pci,guest-cid="$CID" \
      -display none -daemonize

    # Wait for SSH
    echo "Waiting for SSH at $IP..." >&2
    for i in {1..60}; do
      if ssh-keyscan -T 2 "$IP" 2>/dev/null | grep -q "ssh-ed25519"; then
        sleep 2

        # Extract measurements
        MEASUREMENTS=$(ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
          -o LogLevel=ERROR "$IP" '/bin/tdx-info')

        # Write to cache (atomic via temp file)
        mkdir -p "$CACHE_DIR/$BOOT_HASH"
        echo "$MEASUREMENTS" > "$CACHE_FILE.tmp"
        mv "$CACHE_FILE.tmp" "$CACHE_FILE"

        echo "Measurements extracted and cached at:" >&2
        echo "  $CACHE_FILE" >&2
        exit 0
      fi
      sleep 2
    done

    echo "ERROR: Timeout waiting for SSH at $IP" >&2
    exit 1
  '';

  all = stdenv.mkDerivation {
    pname = "tdx-vm-artifacts";
    version = "1.0.0-preview";
    dontUnpack = true;

    installPhase = ''
      mkdir -p $out

      ln -s ${kernel}/bzImage $out/bzImage
      ln -s ${initfs}/initrd $out/rootfs.img
      ln -s ${firmware}/OVMF.fd $out/OVMF_DEBUG.fd
      ln -s ${boot-image}/boot.img $out/boot.img
      ln -s ${extract-measurements}/bin/extract-tdx-measurements $out/extract-tdx-measurements
    '';
  };

in {
  # Expose individual components for reproducibility testing
  inherit kernel initfs firmware boot-image extract-measurements rustWorkspace all;

  # Convenience: default output points to combined artifacts
  default = all;
}
