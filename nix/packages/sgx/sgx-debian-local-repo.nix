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


{ fetchurl, stdenv, dpkg }:

# Pre-unpack all Intel SGX .deb packages from the Debian repo
# This allows downstream packages to skip the unpackPhase entirely
stdenv.mkDerivation {
  name = "sgx-debian-repo-unpacked";

  src = fetchurl {
    url = "https://download.01.org/intel-sgx/latest/linux-latest/distro/ubuntu24.04-server/sgx_debian_local_repo.tgz";
    sha256 = "sha256-vgswowpRgD7mS8P7MGfjfn54UzqZr/b1UEKXedd7vEE=";
  };

  nativeBuildInputs = [ dpkg ];

  buildPhase = ''
    runHook preBuild

    # Unpack all .deb files in place within the source directory
    # Each .deb gets unpacked to a subdirectory named "unpacked" next to it
    find pool -name "*.deb" | while read deb; do
      dir=$(dirname "$deb")
      pkgname=$(basename "$deb" .deb)
      mkdir -p "$dir/unpacked"
      dpkg-deb -x "$deb" "$dir/unpacked"
    done

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/sgx_debian_local_repo
    # Copy the entire repo structure with unpacked contents
    cp -r * $out/sgx_debian_local_repo/

    runHook postInstall
  '';

  meta = {
    description = "Intel SGX/TDX Debian repository with pre-unpacked .deb packages";
  };
}
