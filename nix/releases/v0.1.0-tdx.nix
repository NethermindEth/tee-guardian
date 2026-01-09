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
    # Pin nixpkgs for deterministic, reproducible builds
    # NixOS 25.11 release (commit: d355f89e)
    # Updated: 2025-01-05
    url = "https://github.com/NixOS/nixpkgs/archive/d355f89e0014e51c9511298089d7ab55fd6f7056.tar.gz";
    sha256 = "01nxz6adq7lbpsidak1y2nrlvi8f7ds89zfn33bwfqfy4r2gkzhr";
  }) { system = "x86_64-linux"; }
}:

let
  # Build TEE Guardian as a single static binary
  teeGuardian = pkgs.pkgsStatic.rustPlatform.buildRustPackage rec {
    pname = "tee-guardian";
    version = "0.1.0";

    src = pkgs.lib.cleanSource ../../.;

    cargoLock = {
      lockFile = ../../Cargo.lock;
      outputHashes = {
        # From guest-components repo (https://github.com/confidential-containers/guest-components v0.16.0)
        "attester-0.1.0" = "sha256-LpI/u6NJiBxMPB3hLNrgloSZzb4Q3DLmiY08LAM2n4g=";
        "crypto-0.1.0" = "sha256-LpI/u6NJiBxMPB3hLNrgloSZzb4Q3DLmiY08LAM2n4g=";
        # From trustee repo (https://github.com/confidential-containers/trustee v0.16.0)
        "eventlog-0.1.0" = "sha256-jiW4+t7LIA1csTViFOGHnCoEwWdVl9+CyNUmaYzHGCg=";
        "verifier-0.1.0" = "sha256-jiW4+t7LIA1csTViFOGHnCoEwWdVl9+CyNUmaYzHGCg=";
        # From SGXDataCenterAttestationPrimitives repo
        "intel-tee-quote-verification-rs-0.3.0" = "sha256-OmSm2ojfanJOekLqygxPDlVRovqF7dqiL8mQtOdbfFM=";
      };
    };

    # Build entire workspace as monolithic binary
    cargoBuildFlags = [ "--bin" "verification" ];

    nativeBuildInputs = with pkgs; [
      pkg-config
    ];

    buildInputs = with pkgs.pkgsStatic; [
      openssl
    ];

    RUSTFLAGS = "-C target-cpu=x86-64 -C link-arg=-static";
    PKG_CONFIG_ALL_STATIC = "1";

    meta = with pkgs.lib; {
      description = "TEE Guardian - Minimal TDX service";
      license = licenses.mit;
      platforms = platforms.linux;
    };
  };

  # No busybox - TEE Guardian handles init directly
  initramfs = pkgs.makeInitrd {
    contents = [
      {
        object = "${teeGuardian}/bin/verification";
        symlink = "/init";
      }
    ];
    # Force deterministic timestamps and compression
    compressor = "gzip -9n";  # -n = no timestamp in gzip
  };

  guardianKernel = pkgs.stdenv.mkDerivation rec {
    pname = "tee-guardian-system";
    version = "6.15.4";

  src = pkgs.fetchurl {
    url = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${version}.tar.xz";
    sha256 = "sha256-Dq/WJ7YC9Y1zkX0A5PwxlroYy6Z99plaQqp0dE2O+hY=";
  };

  # Minimal Firecracker + TDX kernel config
  kernelConfig = pkgs.writeText "kernel-config" ''
    # TDX Requirements
    CONFIG_INTEL_TDX_GUEST=y
    CONFIG_TDX_GUEST_DRIVER=y
    CONFIG_CC_PLATFORM=y
    CONFIG_ARCH_HAS_CC_PLATFORM=y

    # Firecracker Critical Requirements
    CONFIG_VIRTIO_MMIO=y
    CONFIG_VIRTIO_BLK=y
    CONFIG_VIRTIO_NET=y
    CONFIG_BLK_DEV_INITRD=y
    CONFIG_KVM_GUEST=y

    # Essential x86-64
    CONFIG_64BIT=y
    CONFIG_SMP=y
    CONFIG_X86_64=y
    CONFIG_CPU_SUP_INTEL=y
    CONFIG_X86_LOCAL_APIC=y
    CONFIG_X86_IO_APIC=y

    # Platform Minimal
    CONFIG_ACPI=y
    CONFIG_PCI=y
    CONFIG_SERIAL_8250=y
    CONFIG_SERIAL_8250_CONSOLE=y

    # Security Essentials
    CONFIG_SECURITY=y
    CONFIG_SECURITY_LOCKDOWN_LSM=y
    CONFIG_SECCOMP=y
    CONFIG_RETPOLINE=y

    # Network Minimal
    CONFIG_NET=y
    CONFIG_INET=y
    CONFIG_PACKET=y
    CONFIG_UNIX=y

    # Filesystem Minimal
    CONFIG_BINFMT_ELF=y
    CONFIG_ELF_CORE=y
    CONFIG_MMU=y
    CONFIG_PROC_FS=y
    CONFIG_SYSFS=y

    # Crypto Minimal
    CONFIG_CRYPTO=y

    CONFIG_BUILD_SALT=""
    CONFIG_LOCALVERSION=""
    CONFIG_LOCALVERSION_AUTO=n
    # CONFIG_MODULES is not set
    # CONFIG_DEBUG_INFO is not set
    # CONFIG_DEBUG_INFO_DWARF_TOOLCHAIN_DEFAULT is not set
    CONFIG_STRIP_ASM_SYMS=y
    # CONFIG_IKHEADERS is not set
    # CONFIG_MODULE_SIG_ALL is not set
    # CONFIG_SYSTEM_TRUSTED_KEYRING is not set
    # CONFIG_SYSTEM_TRUSTED_KEYS is not set
    # CONFIG_RANDOMIZE_BASE is not set
    # CONFIG_RANDOMIZE_MEMORY is not set
    # CONFIG_RANDSTRUCT is not set
  '';

  nativeBuildInputs = with pkgs; [
    bc
    bison
    flex
    gmp
    libmpc
    mpfr
    openssl
    perl
    python3
    rsync
    gnutar
    gzip
    bzip2
    xz
    zstd
    cpio
    elfutils
    zlib
    ncurses
    pkg-config
    pahole
  ];

  # CRITICAL: Remove all knowledge of output paths
  preConfigure = ''
    # Kernel build must never know about $out
    export REAL_OUT="$out"
    unset out
    export out=""

    # Set up clean, isolated environment (CRITICAL for determinism)
    export TMPDIR="$(mktemp -d)"
    export HOME="$TMPDIR"
    export XDG_CONFIG_HOME="$TMPDIR/.config"

    # Clear all Nix environment variables that could leak into build
    unset nixpkgs
    export NIX_CFLAGS_COMPILE=""
    export NIX_LDFLAGS=""
    export NIX_CFLAGS_COMPILE_BEFORE=""
    export NIX_LDFLAGS_BEFORE=""

    # Set reproducible build environment
    export KBUILD_BUILD_HOST="localhost"
    export KBUILD_BUILD_USER="nixbld"
    export KBUILD_BUILD_TIMESTAMP="1980-01-01T00:00:00"
    export SOURCE_DATE_EPOCH="315532800"

    # CRITICAL: Force static module paths that don't reference store
    export INSTALL_MOD_PATH="/lib/modules"
    export MODLIB="/lib/modules/${version}"
    export INSTALL_MOD_STRIP="1"

    # Set deterministic locale and timezone
    export LC_ALL=C
    export TZ=UTC

    # Copy in the config
    cp ${kernelConfig} .config

    # Make sure config is properly processed
    make olddefconfig
  '';

  buildPhase = ''
    # Build with controlled environment to prevent store path leakage
    make -j$(nproc) \
      ARCH=x86_64 \
      CROSS_COMPILE="" \
      HOSTCC=gcc \
      HOSTCXX=g++ \
      CC=gcc \
      LD=ld \
      AR=ar \
      NM=nm \
      STRIP=strip \
      OBJCOPY=objcopy \
      OBJDUMP=objdump \
      READELF=readelf \
      MODLIB="/lib/modules/${version}" \
      INSTALL_MOD_PATH="/lib/modules" \
      KBUILD_BUILD_HOST="localhost" \
      KBUILD_BUILD_USER="nixbld" \
      KBUILD_BUILD_TIMESTAMP="1980-01-01T00:00:00" \
      SOURCE_DATE_EPOCH="315532800" \
      bzImage
  '';

  installPhase = ''
    # Restore output path for installation
    export out="$REAL_OUT"

    mkdir -p $out
    cp arch/x86/boot/bzImage $out/bzImage
    cp ${initramfs}/initrd $out/initramfs.gz
    cp System.map $out/System.map
    cp .config $out/config

    # Verify no store paths leaked into the kernel
    if strings $out/bzImage | grep -q "/nix/store"; then
      echo "ERROR: Store paths found in kernel binary:"
      strings $out/bzImage | grep "/nix/store"
      exit 1
    fi

    echo "SUCCESS: Clean kernel build with no store path references"
  '';

  # Additional verification
  doCheck = true;
  checkPhase = ''
    # Final verification
    if strings arch/x86/boot/bzImage | grep -q "/nix/store"; then
      echo "FAILED: Store paths detected in kernel"
      exit 1
    fi
    echo "PASSED: No store paths in kernel binary"
  '';

    meta = with pkgs.lib; {
      description = "TEE Guardian - Minimal TDX system";
      platforms = platforms.linux;
      license = licenses.gpl2Only;
    };
  };

in {
  # Expose individual components for reproducibility testing
  inherit guardianKernel initramfs teeGuardian;
  
  # Combined system (kernel + initramfs)
  system = guardianKernel;
  
  # Default output
  default = guardianKernel;
}
