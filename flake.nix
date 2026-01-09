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


{
  description = "TEE Guardian - Confidential Computing Infrastructure";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    nixpkgs-unstable.url = "github:NixOS/nixpkgs/nixos-unstable";
    disko = { url = "github:nix-community/disko"; inputs.nixpkgs.follows = "nixpkgs"; };
    sops-nix = { url = "github:Mic92/sops-nix"; inputs.nixpkgs.follows = "nixpkgs"; };
    rust-overlay = { url = "github:oxalica/rust-overlay"; inputs.nixpkgs.follows = "nixpkgs-unstable"; };
  };

  outputs = { self, nixpkgs, nixpkgs-unstable, disko, sops-nix, rust-overlay }:
    let
      system = "x86_64-linux";

      # Development packages (unstable + rust overlay)
      devPkgs = import nixpkgs-unstable {
        inherit system;
        overlays = [ (import rust-overlay) ];
      };

      rustToolchain = devPkgs.rust-bin.selectLatestNightlyWith (toolchain:
        toolchain.default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" "rust-analyzer" ];
        }
      );

      # Intel DCAP development package (headers + stub library for bindgen)
      intel-dcap = devPkgs.callPackage ./nix/packages/intel-dcap.nix { };

      # Host configuration factory
      mkHost = import ./nix/lib/mk-host.nix {
        inherit nixpkgs system disko sops-nix self;
      };

    in {
      devShells.${system}.default = devPkgs.mkShell {
        nativeBuildInputs = with devPkgs; [ rustToolchain pkg-config clang ];
        buildInputs = with devPkgs; [ openssl openssl.dev libclang.lib intel-dcap.dev ];

        packages = with devPkgs; [
          rust-analyzer
          uv
          nixos-anywhere
          diffoscopeMinimal
          nix-diff
          nebula
        ];

        OPENSSL_DIR = "${devPkgs.openssl.dev}";
        OPENSSL_LIB_DIR = "${devPkgs.openssl.out}/lib";
        OPENSSL_INCLUDE_DIR = "${devPkgs.openssl.dev}/include";
        PKG_CONFIG_PATH = "${devPkgs.openssl.dev}/lib/pkgconfig";
        LIBCLANG_PATH = "${devPkgs.libclang.lib}/lib";
        BINDGEN_EXTRA_CLANG_ARGS = "-I${intel-dcap.dev}/include";
        LIBRARY_PATH = "${intel-dcap.dev}/lib";
        LD_LIBRARY_PATH = "${intel-dcap.dev}/lib";
        RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        RUST_BACKTRACE = "1";

        shellHook = ''
          export PATH="$PWD/.venv/bin:$PATH"
          echo "-- TEE Guardian Dev Environment ---"
          echo "  guardian nebula join        - Connect to Nebula overlay network"
          echo "  guardian test unit          - Run Rust tests in TDX guest VM"
          echo "  guardian test integration   - Run service integration tests"
          echo ""
        '';
      };

      overlays.default = import ./nix/overlay.nix;

      packages.${system} =
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
            overlays = [ self.overlays.default ];
          };
        in {
          # Intel SGX/TDX packages
          inherit (pkgs)
            sgxDebianRepo
            libsgx-enclave-common
            libsgx-urts
            libsgx-qe3-logic
            libsgx-pce-logic
            libsgx-tdx-logic
            libsgx-dcap-ql
            libsgx-dcap-default-qpl
            sgx-enclaves
            tdx-qgs
            tdx-qgs-daemon
            sgx-dcap-pccs
            libsgx-ra-uefi
            libsgx-ra-network
            sgx-ra-service
            sgx-pck-id-retrieval-tool;

          # QEMU
          inherit (pkgs) libigvm;
          inherit (pkgs) qemu-coco;

          # AMD SEV-SNP packages
          inherit (pkgs) snphost;

          # Other tools
          inherit (pkgs) trustauthority-cli;

          # Kexec installer tarballs for nixos-anywhere
          kexec-open-metal-tdx-dev = self.nixosConfigurations.open-metal-tdx-dev-kexec.config.system.build.kexecTarball;
        };

      nixosConfigurations = {
        # Intel TDX hosts
        open-metal-tdx-dev = mkHost "open-metal-tdx-dev" { };
        open-metal-tdx-dev-kexec = mkHost "open-metal-tdx-dev" { kexecInstaller = true; };

        # AMD SEV-SNP hosts
        hetzner-sev-dev = mkHost "hetzner-sev-dev" { };
      };

      hydraJobs = {
        v0-1-0-tdx = import ./nix/releases/v0.1.0-tdx.nix {};
        test-vm = import ./nix/releases/test-vm.nix {};
      };
    };
}
