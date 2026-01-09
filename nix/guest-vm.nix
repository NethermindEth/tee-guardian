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


{ pkgs ? import <nixpkgs> {} }:

let
  inherit (pkgs) lib stdenv fetchFromGitHub applyPatches fetchpatch buildPackages;

  nixosConfig = import "${pkgs.path}/nixos/lib/eval-config.nix" {
    inherit (pkgs) system;
    modules = [
      "${pkgs.path}/nixos/modules/profiles/minimal.nix"
      ({ config, pkgs, lib, ... }: {
        # Add trustauthority-cli to package set
        nixpkgs.overlays = [
          (final: prev: {
            trustauthority-cli = prev.callPackage ./packages/trustauthority-cli.nix { };
          })
        ];

        # Kernel configuration
        boot.kernelPackages = pkgs.linuxPackages_latest;
        boot.kernelParams = [
          "console=ttyS0,115200"
          "earlyprintk=serial,ttyS0,115200"
          "debug"
          "rw"
        ];

        # Boot configuration
        boot.initrd = {
          enable = true;
          systemd.enable = true;
          availableKernelModules = [
            "virtio_pci"
            "virtio_blk"
            "virtio_net"
            "virtio_console"
            "vmw_vsock_virtio_transport_common"
            "vsock"
          ];
        };

        boot.loader = {
          grub.enable = false;
          systemd-boot.enable = true;
          timeout = 0;
          efi.canTouchEfiVariables = lib.mkForce false;
        };

        # Root filesystem on disk partition 2
        fileSystems."/" = {
          device = "/dev/vda2";
          fsType = "ext4";
        };

        # ESP partition on partition 1
        fileSystems."/boot" = {
          device = "/dev/vda1";
          fsType = "vfat";
        };

        # System packages
        environment.systemPackages = with pkgs; [
          # Dev & Utils
          neovim
          gcc
          rsync
          tmux
          pkg-config
          openssl.dev
          strace
          trustauthority-cli

          # Perf & Monitoring
          btop
          stress-ng
          iperf3
          sysbench
          numactl
          linuxPackages_latest.perf
          fio
        ];

        # add dynamic system libs for compiling rust test quickly
        programs.nix-ld = {
          enable = true;
          libraries = with pkgs; [
            stdenv.cc.cc.lib
            zlib
            openssl.dev
            curl
            libz
            glibc
          ];
        };

        # Set PKG_CONFIG_PATH so pkg-config can find openssl.pc
        environment.variables = {
          PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" [
            pkgs.openssl.dev
          ];
        };

        # Enable Nix experimental features (flakes, nix-command)
        nix.settings.experimental-features = [ "nix-command" "flakes" ];

        # Set NIX_PATH so nix-shell works
        nix.nixPath = [
          "nixpkgs=${pkgs.path}"
        ];

        # Network configuration
        networking = {
          hostName = "tdx-guest";
          useDHCP = true;
          firewall.enable = false;
        };

        # SSH access - configure for guest VM
        services.openssh = {
          enable = true;
          settings = {
            PasswordAuthentication = false;
            KbdInteractiveAuthentication = false;
            PermitRootLogin = "prohibit-password";
            AllowTcpForwarding = true;
            UseDns = false;
            GSSAPIAuthentication = false;
          };
        };

        # Prometheus Node Exporter - System Metrics
        # Most collectors (cpu, memory, network, filesystem, etc.) are enabled by default
        # We only enable non-default collectors that are useful for TDX VMs
        services.prometheus.exporters.node = {
          enable = true;
          enabledCollectors = [ "diskstats" ];  # Disk I/O metrics (disabled by default)
        };

        users = {
          mutableUsers = false;
          allowNoPasswordLogin = true;
          users.root = {
            openssh.authorizedKeys.keys = [
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG0TA+6GXPF5JNx2gs86fPM2znIS1Yv6cUauUjjQhQdM devops@nethermind.io"
              "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDgDA8a/EFrgf2Vzr7+Qnh1UBzu/l5xX1e/vMtNs1hiwdPCfjv/MisPidTlvU5X1tUvAGUZodX871FdnNX1EfRbWxX2kvURaM0GPJRhzCI+vmohH365qix4/HDUCVCFMGwDV8J6n3SgOYoOfGTOaFt+Q1Xmw8hHQfGOdxrh2AYWsGEjOhen4lPhZVDKzUB6+ZQmFnDWS9nd7ds8YOJ6ryxgdEICaD+rPSCDaRDJy5iHM4hyNITTm50pCR+oeYZ1Ay8q5ec3XEmpFGQSw4Roz5LV95TIfb0U7In8TTPGFrIPkxsvrEhBIdAVTcJXctHC4Ei2kOCAz0ArM0qA/L/Lpu7BNb/7eNHICEekTGx7v2tPqiE8+zTU8r7P2f5jWLcVYcJX8Xmj9xzBccR8Jo21+oujwo9Z2Yae94cdDkQeSQpASi/lZo7u7X7dfmUU70pypaDJhNwJv2GGRjRUPHFxVDMkRWJTGI0+QG8MoPMneOuolfOi7oSfrJ8/BrW3SlOOFgd73pvplZ4op/EwPCNKPgsig8oh24KOPxOD3C4hOPVr5OK7TVhG0KuHGeOkUgbtdC7RBcmwXWCKbmZ6xfrxXwvtuagWp5/6d3Cu96K2Q3dhVbh/DSaJH1uMKnEW0fsuB8xXj/YI5GrpaLIFNBpIibMiwOh3EJQQCawldKBJFRN3WQ== elicb@elicb-xps-wsl"
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAnDfz+63L2/HxDvEFBWmk9+0p09fB3aIW9FtH8k99HX franco@nethermind.io"
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHlesQZ1Z4xARl7yN72VgZTa4KDpNfyxfPMnEvF01F2K"
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGLiZbJ4OvpkXhw7WrKpZbB8CRaZF4WiTpPoljHcexy1"
              "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK3Od6tYI09lcB2IPFft31NrpzskA4lLIcVE6rZrzXbl"
            ];
            hashedPassword = "!";
            shell = pkgs.bash;
          };
        };

        # Minimal services
        services.getty.autologinUser = "root";

        # Disable unnecessary services
        documentation.enable = false;
        documentation.nixos.enable = false;
        programs.command-not-found.enable = false;

        # Enable direnv with bash hook for dev environment caching
        programs.direnv = {
          enable = true;
          silent = true;
        };

        systemd.services.sshd.wantedBy = lib.mkForce [ "multi-user.target" ];

        system.stateVersion = "25.05";
      })
    ];
  };

  edk2-tdx =
  let
    srcWithVendoring = fetchFromGitHub {
      owner = "tianocore";
      repo = "edk2";
      rev = "edk2-stable202505";
      fetchSubmodules = true;
      hash = "sha256-VuiEqVpG/k7pfy0cOC6XmY+8NBtU/OHdDB9Y52tyNe8=";
    };
  in
  stdenv.mkDerivation {
    name = "edk2-tdx-debug";

    src = applyPatches {
      name = "edk2-tdx-202505-unvendored-src";
      src = srcWithVendoring;

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

    nativeBuildInputs = [
     pkgs.python3 pkgs.nasm pkgs.perl pkgs.bc pkgs.acpica-tools
    ];
    buildInputs = [ pkgs.util-linuxMinimal.dev ];

    depsBuildBuild = [ buildPackages.stdenv.cc buildPackages.bash ];
    depsHostHost = [ ];
    strictDeps = true;

    GCC5_X64_PREFIX = stdenv.cc.targetPrefix;
    PYTHON_COMMAND = "${buildPackages.python3}/bin/python3";

    env.NIX_CFLAGS_COMPILE =
      "-Wno-return-type" +
      lib.optionalString (stdenv.cc.isGNU) " -Wno-error=stringop-truncation";

    hardeningDisable = [
      "format"
      "fortify"
    ];

    preConfigure = ''
      export WORKSPACE="$PWD"
      export PACKAGES_PATH="$WORKSPACE"
      export EDK_TOOLS_PATH="$WORKSPACE/BaseTools"

      export BUILD_CC="${buildPackages.stdenv.cc}/bin/gcc"
      export CC="${stdenv.cc}/bin/${stdenv.cc.targetPrefix}gcc"
      export CXX="${stdenv.cc}/bin/${stdenv.cc.targetPrefix}g++"
      export AR="${stdenv.cc.bintools.bintools}/bin/${stdenv.cc.targetPrefix}ar"
      export LD="${stdenv.cc.bintools.bintools}/bin/${stdenv.cc.targetPrefix}ld"

      if [ ! -f CryptoPkg/Library/OpensslLib/openssl/README.md ]; then
        echo "❌ OpenSSL submodule missing!"
        exit 1
      fi

      echo "✅ OpenSSL submodule verified"
    '';

    buildPhase = ''
      runHook preBuild

      make -C BaseTools \
        CC="${buildPackages.stdenv.cc}/bin/gcc" \
        CXX="${buildPackages.stdenv.cc}/bin/g++" \
        AS="${buildPackages.stdenv.cc}/bin/as" \
        AR="${buildPackages.stdenv.cc.bintools.bintools}/bin/ar" \
        LD="${buildPackages.stdenv.cc.bintools.bintools}/bin/ld" \
        -j$(nproc)

      for i in BaseTools/BinWrappers/PosixLike/*; do
        if [ -f "$i" ]; then
          chmod +x "$i"
          patchShebangs --build "$i"
        fi
      done

      patchShebangs --build BaseTools/BinPipWrappers/PosixLike/* || true
      patchShebangs --build BaseTools/Source/Python/* || true

      source ./edksetup.sh BaseTools

      echo "🔧 Building TDX Debug firmware..."

      build -a X64 -b DEBUG -t GCC5 -p OvmfPkg/IntelTdx/IntelTdxX64.dsc -n $(nproc) \
        -D SECURE_BOOT_ENABLE=FALSE \
        -D BUILD_SHELL=TRUE

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out/share/edk2-tdx/debug

      if [ -f Build/IntelTdx/DEBUG_GCC5/FV/OVMF.fd ]; then
        cp Build/IntelTdx/DEBUG_GCC5/FV/OVMF.fd $out/share/edk2-tdx/debug/
        echo "✅ TDX Debug firmware: $(numfmt --to=iec $(wc -c < $out/share/edk2-tdx/debug/OVMF.fd))"
      else
        echo "❌ TDX Debug firmware not found"
        exit 1
      fi

      runHook postInstall
    '';

    enableParallelBuilding = true;

    meta = with lib; {
      description = "EDK2 TDX Debug firmware";
      platforms = platforms.linux;
      license = licenses.bsd2;
    };
  };

  diskImage = import "${pkgs.path}/nixos/lib/make-disk-image.nix" {
    inherit pkgs lib;
    config = nixosConfig.config;
    format = "raw";
    partitionTableType = "efi";
    diskSize = "auto";
    additionalSpace = "20G";  # Add space for rust toolchain and compiling tests in guest
    installBootLoader = true;
    copyChannel = false;
  };

in stdenv.mkDerivation {
  pname = "tdx-nixos-vm";
  version = "2.0.0";

  dontUnpack = true;

  installPhase = ''
    runHook preInstall

    mkdir -p $out

    cp ${edk2-tdx}/share/edk2-tdx/debug/OVMF.fd $out/OVMF_DEBUG.fd
    cp ${diskImage}/nixos.img $out/boot.img

    echo ""
    echo "🎉 TDX NixOS VM 🎉"
    echo ""
    echo "📦 OVMF_DEBUG.fd: $(numfmt --to=iec $(wc -c < $out/OVMF_DEBUG.fd))"
    echo "📦 boot.img: $(numfmt --to=iec $(wc -c < $out/boot.img))"
    echo ""

    runHook postInstall
  '';
}
