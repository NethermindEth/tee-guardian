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


# AMD SEV-SNP Host Module
#
# Configures a NixOS host for AMD SEV-SNP (Secure Encrypted Virtualization -
# Secure Nested Paging) confidential computing.
#
# Requirements:
#   - AMD EPYC Milan (7003) or Genoa (9004) processor
#   - Linux kernel 6.11+ (mainline SEV-SNP support with guest_memfd)
#   - BIOS: SNP Support = Enabled, SNP Memory Coverage = Auto, IOMMU = Enabled
#
# Key Features:
#   - RMP (Reverse Map Table) for memory integrity protection
#   - VCEK certificate caching from AMD KDS
#   - SVSM (Secure VM Service Module) support via IGVM
#   - vhost-vsock for guest attestation communication

{ config, lib, pkgs, ... }:

let
  inherit (lib)
    mkEnableOption mkOption mkIf mkDefault mkMerge mkForce
    types optional optionals optionalString;

  cfg = config.services.snp-host;

  # === Health Check Script ===
  healthCheckScript = pkgs.writeShellScriptBin "check-snp-host" ''
    set -euo pipefail

    echo "=== AMD SEV-SNP Host Health Check ==="
    echo ""

    ERRORS=0

    # Check architecture
    echo -n "Architecture (x86_64): "
    if [ "$(uname -m)" = "x86_64" ]; then
      echo "OK"
    else
      echo "FAIL - $(uname -m)"
      ERRORS=$((ERRORS + 1))
    fi

    # Check kernel module parameter
    echo -n "KVM AMD SEV-SNP enabled: "
    if [ -f /sys/module/kvm_amd/parameters/sev_snp ]; then
      if grep -q 'Y' /sys/module/kvm_amd/parameters/sev_snp 2>/dev/null; then
        echo "OK"
      else
        echo "FAIL - disabled"
        ERRORS=$((ERRORS + 1))
      fi
    else
      echo "FAIL - parameter missing"
      ERRORS=$((ERRORS + 1))
    fi

    # Check RMP allocation
    echo -n "RMP table allocated: "
    if dmesg 2>/dev/null | grep -q "RMP table physical address"; then
      echo "OK"
    else
      echo "FAIL - not found in dmesg"
      echo "  Verify BIOS: SNP Support = Enabled, SNP Memory Coverage = Auto"
      ERRORS=$((ERRORS + 1))
    fi

    # Check SEV device
    echo -n "SEV device (/dev/sev): "
    if [ -c /dev/sev ]; then
      echo "OK"
    else
      echo "FAIL - not found"
      ERRORS=$((ERRORS + 1))
    fi

    # Check vhost-vsock device
    echo -n "VSOCK device (/dev/vhost-vsock): "
    if [ -c /dev/vhost-vsock ]; then
      echo "OK"
    else
      echo "FAIL - not found"
      ERRORS=$((ERRORS + 1))
    fi

    # Check GHCB certificate blob
    echo -n "GHCB certificate blob: "
    GHCB_FILE="${cfg.attestation.certCachePath}/ghcb-certs.bin"
    if [ -s "$GHCB_FILE" ]; then
      echo "OK ($(stat -c%s "$GHCB_FILE") bytes)"
    else
      echo "FAIL - missing or empty"
      ERRORS=$((ERRORS + 1))
    fi

    # Check snphost status
    echo -n "Platform SNP status: "
    if ${pkgs.snphost}/bin/snphost ok 2>/dev/null; then
      echo "OK"
    else
      echo "FAIL"
      ERRORS=$((ERRORS + 1))
    fi

    # Check ASID availability for SNP
    echo ""
    echo "=== SNP ASID Availability ==="
    ASID_INFO=$(dmesg 2>/dev/null | grep -i "SEV-SNP.*ASID" | tail -1)
    if [ -n "$ASID_INFO" ]; then
      echo "$ASID_INFO"
      # Extract ASID count - format is typically "SEV-SNP enabled (ASIDs 1 - 99)"
      SNP_ASIDS=$(echo "$ASID_INFO" | grep -oP 'ASIDs \K[0-9]+' | head -1)
      if [ -n "$SNP_ASIDS" ] && [ "$SNP_ASIDS" -eq 0 ]; then
        echo "WARNING: SNP ASIDs = 0, feature effectively disabled"
        echo "  Check BIOS: SNP Memory Coverage = Auto"
        ERRORS=$((ERRORS + 1))
      elif [ -n "$SNP_ASIDS" ]; then
        echo "SNP ASIDs available: $SNP_ASIDS"
      fi
    else
      echo "No SNP ASID info in dmesg (checking generic SEV)"
      dmesg 2>/dev/null | grep -i "SEV.*ASID" | tail -1 || echo "No ASID info found"
    fi

    echo ""
    if [ $ERRORS -eq 0 ]; then
      echo "=== All checks passed ==="
      exit 0
    else
      echo "=== FAILED: $ERRORS check(s) failed ==="
      exit 1
    fi
  '';

  # === Certificate Fetch Script ===
  fetchCertsScript = pkgs.writeShellScript "snp-fetch-certs" ''
    set -euo pipefail

    CERT_DIR="${cfg.attestation.certCachePath}/certs"
    GHCB_FILE="${cfg.attestation.certCachePath}/ghcb-certs.bin"

    echo "=== AMD SEV-SNP Certificate Management ==="

    echo "Checking SEV-SNP platform status..."
    if ! ${pkgs.snphost}/bin/snphost ok; then
      echo "ERROR: Platform not in valid SNP state"
      echo "Verify BIOS settings:"
      echo "  - SNP Support = Enabled"
      echo "  - SNP Memory Coverage = Auto"
      echo "  - IOMMU = Enabled"
      exit 1
    fi

    mkdir -p "$CERT_DIR"

    echo "Fetching AMD certificate chain (ARK/ASK)..."
    ${pkgs.snphost}/bin/snphost fetch ca pem "$CERT_DIR"

    echo "Fetching VCEK certificate for current TCB..."
    ${pkgs.snphost}/bin/snphost fetch vek pem "$CERT_DIR"

    echo "Verifying certificate chain integrity..."
    ${pkgs.snphost}/bin/snphost verify "$CERT_DIR"

    echo "Generating GHCB certificate blob for QEMU..."
    ${pkgs.snphost}/bin/snphost import "$CERT_DIR" "$GHCB_FILE"

    chown -R root:${cfg.sevGroup} "${cfg.attestation.certCachePath}"
    chmod 750 "${cfg.attestation.certCachePath}"
    chmod 640 "$GHCB_FILE"

    echo "=== Certificate update complete ==="
    echo "GHCB blob: $GHCB_FILE"
    ls -la "$CERT_DIR"
  '';

  # === QEMU Helper Script ===
  # Generates recommended QEMU arguments for SNP guests
  qemuHelperScript = pkgs.writeShellScriptBin "snp-qemu-args" ''
    SVSM_ENABLED="${if cfg.svsm.enable then "true" else "false"}"
    IGVM_PATH="${if cfg.svsm.igvmPath != null then cfg.svsm.igvmPath else ""}"

    cat <<EOF
# AMD SEV-SNP QEMU Arguments
# Generated by snp-host module
# SVSM/vTPM: $SVSM_ENABLED

# SEV-SNP guest object
-object sev-snp-guest,id=sev0,cbitpos=${toString cfg.guestDefaults.cbitpos},reduced-phys-bits=${toString cfg.guestDefaults.reducedPhysBits},policy=${cfg.guestDefaults.policy},sev-snp-certs=${cfg.attestation.certCachePath}/ghcb-certs.bin

EOF

    if [ "$SVSM_ENABLED" = "true" ] && [ -n "$IGVM_PATH" ]; then
      cat <<EOF
# SVSM/vTPM enabled - using IGVM firmware loading
# NOTE: When using IGVM, do NOT use -bios. IGVM contains SVSM + OVMF.
-object igvm-cfg,id=igvm0,file=$IGVM_PATH
-machine q35,confidential-guest-support=sev0,igvm-cfg=igvm0,vmport=off

EOF
    else
      cat <<EOF
# Standard firmware loading (no SVSM/vTPM)
# MUST use -bios for guest_memfd compatibility, NOT pflash
-machine q35,confidential-guest-support=sev0,vmport=off
-bios /path/to/OVMF.fd

EOF
    fi

    cat <<EOF
# VSOCK for attestation (adjust guest-cid per VM, must be unique)
-device vhost-vsock-pci,guest-cid=3
EOF
  '';

in {
  options.services.snp-host = {
    enable = mkEnableOption "AMD SEV-SNP confidential computing host";

    # === Kernel Configuration ===
    kernel = {
      version = mkOption {
        type = types.str;
        description = "Linux kernel version (must be 6.11+ for mainline SEV-SNP with guest_memfd)";
      };

      modDirVersion = mkOption {
        type = types.str;
        default = "${cfg.kernel.version}-snp";
        description = "Module directory version (defaults to kernel version with -snp suffix)";
      };

      sha256 = mkOption {
        type = types.str;
        description = "SHA256 hash of the kernel source tarball";
      };
    };

    # === Attestation Configuration ===
    attestation = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = "Enable VCEK certificate management from AMD KDS";
      };

      certCachePath = mkOption {
        type = types.path;
        default = "/var/lib/amd-sev";
        description = "Directory for caching VCEK certificates and GHCB blob";
      };

      refreshSchedule = mkOption {
        type = types.str;
        default = "weekly";
        example = "daily";
        description = "Certificate refresh schedule (systemd calendar syntax)";
      };
    };

    # === SVSM Configuration ===
    svsm = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Enable Coconut-SVSM (Secure VM Service Module) support.
          SVSM provides a vTPM at VMPL0, enabling measured boot for SNP guests.
          Requires IGVM firmware loading instead of standard -bios.
        '';
      };

      igvmPath = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/var/lib/amd-sev/coconut-svsm.igvm";
        description = ''
          Path to IGVM file containing SVSM + OVMF.
          If null, users must provide their own IGVM file.
        '';
      };
    };

    # === Device Permissions ===
    sevGroup = mkOption {
      type = types.str;
      default = "sev";
      description = "Group with access to /dev/sev and certificate cache";
    };

    sevUsers = mkOption {
      type = types.listOf types.str;
      default = [];
      example = [ "qemu" "libvirtd" ];
      description = "Users to add to the SEV group for device access";
    };

    # === Guest Configuration Defaults ===
    guestDefaults = {
      policy = mkOption {
        type = types.str;
        default = "0x30000";
        description = ''
          Default SNP guest policy bitmask.

          Common values:
            0x30000 = Debug allowed, SMT allowed (development/testing)
            0x0     = Fully locked down (production - debug disabled)

          Policy bits:
            Bit 17 (0x20000): SMT (Simultaneous Multi-Threading) allowed
            Bit 16 (0x10000): Debug allowed (INSECURE - disables attestation guarantees)

          For production deployments requiring strict attestation, use 0x0.
        '';
      };

      cbitpos = mkOption {
        type = types.int;
        default = 51;
        description = ''
          C-bit position for memory encryption.
          Usually 51 on EPYC Milan/Genoa. Query via CPUID if unsure.
        '';
      };

      reducedPhysBits = mkOption {
        type = types.int;
        default = 1;
        description = "Number of physical address bits used for encryption metadata";
      };
    };
  };

  config = mkIf cfg.enable {

    # === Assertions ===
    assertions = [
      {
        assertion = pkgs.stdenv.isx86_64;
        message = "SEV-SNP requires x86_64 architecture";
      }
      {
        # Parse major.minor from version string and verify >= 6.11
        assertion = let
          versionParts = lib.splitString "." cfg.kernel.version;
          major = lib.toInt (builtins.elemAt versionParts 0);
          minor = lib.toInt (builtins.elemAt versionParts 1);
        in (major > 6) || (major == 6 && minor >= 11);
        message = "SEV-SNP requires Linux kernel >= 6.11 for mainline guest_memfd support. Configured: ${cfg.kernel.version}";
      }
    ];

    # === Warnings ===
    # Check if debug bit (bit 19) might be set
    warnings =
      optional (lib.hasPrefix "0x1" cfg.guestDefaults.policy || lib.hasPrefix "0x8" cfg.guestDefaults.policy)
        "SEV-SNP: Guest policy ${cfg.guestDefaults.policy} may have debug enabled. Verify bit 19 is clear for production!"
      ++ optional (!cfg.svsm.enable)
        "SEV-SNP: SVSM disabled. Guests will not have vTPM support for measured boot.";

    # === Kernel Configuration ===
    boot.kernelPackages = let
      linux_snp_pkg = { fetchurl, buildLinux, ... } @ args:
        buildLinux (args // rec {
          version = cfg.kernel.version;
          modDirVersion = cfg.kernel.modDirVersion;

          src = pkgs.fetchurl {
            url = "mirror://kernel/linux/kernel/v6.x/linux-${version}.tar.xz";
            sha256 = cfg.kernel.sha256;
          };

          # SEV-SNP host kernel configuration
          # Reference: Linux 6.11+ mainline requirements
          structuredExtraConfig = with lib.kernel; {
            LOCALVERSION = freeform "-snp";

            CPU_SUP_AMD = yes;
            X86_X2APIC = yes;  # Required for high core counts
            X86_MCE = yes;
            X86_MCE_AMD = yes;

            # KVM_AMD_SEV is enabled in common-config for 6.11+
            # KVM_GENERIC_PRIVATE_MEM (guest_memfd) is auto-selected by KVM_AMD_SEV

            # === Memory Encryption ===
            AMD_MEM_ENCRYPT = yes;
            MEMORY_ISOLATION = yes;  # Required for guest_memfd

            # === AMD Secure Processor (ASP/PSP) ===
            CRYPTO_DEV_CCP = yes;
            CRYPTO_DEV_SP_PSP = yes;  # Platform Security Processor

            # === VSOCK for attestation ===
            VSOCKETS = yes;
            VHOST_VSOCK = module;

            # === Memory management ===
            CMA = yes;
            COMPACTION = yes;
            TRANSPARENT_HUGEPAGE = yes;

            # === Unset options from common-config.nix ===
            CRC32_SELFTEST = mkForce unset;
            CRYPTO_TEST = mkForce unset;
            EXT3_FS_POSIX_ACL = mkForce unset;
            EXT3_FS_SECURITY = mkForce unset;
            POWER_RESET_GPIO = mkForce unset;
            POWER_RESET_GPIO_RESTART = mkForce unset;
            REISERFS_FS_POSIX_ACL = mkForce unset;
            REISERFS_FS_SECURITY = mkForce unset;
            REISERFS_FS_XATTR = mkForce unset;
            XEN_SAVE_RESTORE = mkForce unset;
          };
        });

      linux_snp = pkgs.callPackage linux_snp_pkg {};
    in
      pkgs.recurseIntoAttrs (pkgs.linuxPackagesFor linux_snp);

    # === Boot Parameters ===
    boot.kernelParams = [
      "mem_encrypt=on"      # Enable memory encryption
      "kvm_amd.sev=1"       # Enable SEV
      "kvm_amd.sev_es=1"    # Enable SEV-ES (Encrypted State)
      "kvm_amd.sev_snp=1"   # Enable SEV-SNP
      "iommu=pt"            # IOMMU pass-through mode for performance
    ];

    # === Kernel Modules ===
    boot.kernelModules = [
      "kvm-amd"       # KVM with AMD support
      "ccp"           # Cryptographic Coprocessor (ASP interface)
      "vhost-vsock"   # Host-side vsock for attestation
    ];

    # === Hardware Configuration ===
    hardware.cpu.amd.updateMicrocode = mkDefault true;

    # === Users and Groups ===
    users.groups.${cfg.sevGroup} = {};

    users.users = mkMerge ([
      {
        qemu = {
          isSystemUser = true;
          group = "qemu";
          description = "QEMU user for SEV-SNP VMs";
        };
      }
    ] ++ map (user: {
      ${user}.extraGroups = [ cfg.sevGroup ];
    }) cfg.sevUsers);

    # === Udev Rules ===
    services.udev.extraRules = ''
      # SEV device - required for SNP guest management
      KERNEL=="sev", OWNER="root", GROUP="${cfg.sevGroup}", MODE="0660", TAG+="systemd"

      # VSOCK device - required for guest attestation communication
      KERNEL=="vhost-vsock", OWNER="root", GROUP="${cfg.sevGroup}", MODE="0660"
    '';

    # === Certificate Management Service ===
    systemd.tmpfiles.rules = mkIf cfg.attestation.enable [
      "d ${cfg.attestation.certCachePath} 0750 root ${cfg.sevGroup} -"
      "d ${cfg.attestation.certCachePath}/certs 0750 root ${cfg.sevGroup} -"
    ];

    systemd.services.snp-certs = mkIf cfg.attestation.enable {
      description = "AMD SEV-SNP Certificate Cache Manager";
      documentation = [
        "https://github.com/virtee/snphost"
        "https://www.amd.com/en/developer/sev.html"
      ];

      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      # Only run if /dev/sev exists
      unitConfig.ConditionPathExists = [ "/dev/sev" ];

      path = [ pkgs.snphost ];

      serviceConfig = {
        Type = "oneshot";
        ExecStart = fetchCertsScript;
        RemainAfterExit = true;
        User = "root";
        Group = "root";
        StateDirectory = "amd-sev";

        # Security hardening
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.attestation.certCachePath ];
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        DeviceAllow = [ "/dev/sev rw" ];

        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "snp-certs";
      };
    };

    # === Certificate Refresh Timer ===
    systemd.timers.snp-certs = mkIf cfg.attestation.enable {
      description = "Periodic AMD SEV-SNP certificate refresh";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = cfg.attestation.refreshSchedule;
        Persistent = true;
        RandomizedDelaySec = "1h";
      };
    };

    # === Packages ===
    environment.systemPackages = [
      pkgs.snphost
      healthCheckScript
      qemuHelperScript
    ];

    # === Environment Variables ===
    environment.variables = mkIf cfg.attestation.enable {
      SEV_SNP_CERTS = "${cfg.attestation.certCachePath}/ghcb-certs.bin";
    };

    # === SVSM Configuration ===
    environment.etc = mkIf (cfg.svsm.enable && cfg.svsm.igvmPath != null) {
      "amd-sev/svsm.conf".text = ''
        # Coconut-SVSM Configuration
        # IGVM file path for QEMU: ${cfg.svsm.igvmPath}
        #
        # QEMU usage with SVSM (replaces -bios):
        #   -object igvm-cfg,id=igvm0,file=${cfg.svsm.igvmPath}
        #   -machine confidential-guest-support=sev0,igvm-cfg=igvm0,vmport=off
        #
        # Note: When using IGVM, do NOT use -bios. The IGVM file contains
        # both SVSM and OVMF firmware, pre-measured for attestation.
      '';
    };
  };
}
