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


# snphost - AMD SEV-SNP Host Management Tool
#
# Manages SEV-SNP platform attestation certificates (VCEK) from the AMD Key
# Distribution Service (KDS). Required for guests to verify attestation reports.
#
# Commands:
#   snphost ok              - Verify platform is in valid SNP state
#   snphost fetch ca pem    - Download ARK/ASK certificate chain
#   snphost fetch vek pem   - Download VCEK certificate for current TCB
#   snphost verify          - Verify certificate chain integrity
#   snphost import          - Generate GHCB certificate blob for QEMU
#
# Source: https://github.com/virtee/snphost

{ lib
, rustPlatform
, fetchFromGitHub
, pkg-config
, openssl
}:

rustPlatform.buildRustPackage rec {
  pname = "snphost";
  version = "0.5.0";

  src = fetchFromGitHub {
    owner = "virtee";
    repo = "snphost";
    rev = "v${version}";
    hash = "sha256-GaeNoLx/fV/NNUS2b2auGvylhW6MOFp98Xi0sdDV3VM=";
  };

  cargoHash = "sha256-11D26PqCcKPoyCk4Zx29pkc6/B8DR+9+y+RJAq6ZbCs=";

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    openssl
  ];

  # snphost requires /dev/sev access for some operations
  # Those will fail during build, but the binary will work at runtime
  doCheck = false;

  meta = with lib; {
    description = "AMD SEV-SNP host platform management tool";
    longDescription = ''
      snphost manages the AMD SEV-SNP platform attestation infrastructure.
      It fetches and caches VCEK certificates from AMD's Key Distribution
      Service, which are required for guests to verify attestation reports.

      Key operations:
      - Fetch ARK/ASK/VCEK certificate chain from AMD KDS
      - Generate GHCB certificate blob for QEMU sev-snp-certs parameter
      - Verify certificate chain integrity
      - Check platform SNP status
    '';
    homepage = "https://github.com/virtee/snphost";
    license = licenses.asl20;
    platforms = [ "x86_64-linux" ];
    mainProgram = "snphost";
  };
}
