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


# Generic builder for Intel SGX/TDX packages from pre-unpacked Debian repository
# All packages use the same pattern: reference unpacked .deb contents, autoPatchelf, install
{ lib, stdenv, autoPatchelfHook, makeWrapper ? null, sgxDebianRepo }:

{ pname
, version
, debPath          # e.g., "pool/main/libs/libsgx-enclave-common"
, buildInputs ? []
, nativeBuildInputs ? []
, installPhase ? null
, dontPatchELF ? false
, dontStrip ? false
, dontAutoPatchelf ? false
, meta ? {}
}:

stdenv.mkDerivation {
  inherit pname version dontPatchELF dontStrip dontAutoPatchelf;
  
  # No source needed - we reference the pre-unpacked repo directly
  dontUnpack = true;
  
  nativeBuildInputs = 
    (if dontAutoPatchelf then [] else [ autoPatchelfHook ]) 
    ++ (if makeWrapper != null then [ makeWrapper ] else [])
    ++ nativeBuildInputs;
  
  buildInputs = [ stdenv.cc.cc.lib ] ++ buildInputs;
  
  # Default installPhase: copy libraries from unpacked location
  # Can be overridden for packages with special needs (binaries, configs, etc.)
  installPhase = if installPhase != null then installPhase else ''
    runHook preInstall
    
    mkdir -p $out/lib
    cp -r ${sgxDebianRepo}/sgx_debian_local_repo/${debPath}/unpacked/usr/lib/x86_64-linux-gnu/* $out/lib/ || true
    
    runHook postInstall
  '';
  
  meta = with lib; {
    homepage = "https://github.com/intel/linux-sgx";
    license = licenses.bsd3;
    platforms = platforms.linux;
  } // meta;
}
