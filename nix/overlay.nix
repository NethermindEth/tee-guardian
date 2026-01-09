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


# Provides packages for both Intel TDX and AMD SEV-SNP confidential computing hosts.

final: prev:

let
  sgxPackages = final.callPackage ./packages/sgx { };
in
{
  # === Intel SGX/TDX Packages ===

  inherit (sgxPackages)
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

  # === IGVM Library (required for QEMU IGVM support) ===

  libigvm = final.callPackage ./packages/igvm.nix { };

  # Single QEMU package with full confidential computing support (TDX + SEV-SNP + IGVM)
  qemu-coco = final.callPackage ./packages/qemu.nix { };

  # === AMD SEV-SNP Utilities ===

  snphost = final.callPackage ./packages/snphost.nix { };

  # === Intel SGX/TDX Utilities ===

  trustauthority-cli = prev.callPackage ./packages/trustauthority-cli.nix { };
}
