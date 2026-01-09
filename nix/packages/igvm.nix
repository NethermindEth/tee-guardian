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


# IGVM - Independent Guest Virtual Machine Library
#
# C library for parsing and creating IGVM files used by confidential
# computing guests (AMD SEV-SNP, Intel TDX). Required by QEMU for
# --enable-igvm support.
#
# Build approach:
# 1. Build the Rust library with cargo (staticlib + cdylib)
# 2. Generate C headers using cbindgen with upstream's config files
# 3. Run upstream's post_process.sh to add IGVM_ prefixes
# 4. Create pkg-config file for QEMU's meson build system
#
# Source: https://github.com/microsoft/igvm

{ lib
, stdenv
, fetchFromGitHub
, rustPlatform
, rust-cbindgen
, cargo
, rustc
}:

stdenv.mkDerivation rec {
  pname = "libigvm";
  version = "0.4.0";

  src = fetchFromGitHub {
    owner = "microsoft";
    repo = "igvm";
    rev = "igvm-v${version}";
    hash = "sha256-R1l28h7QJjYsa0vfzyc+kHEWfvXQiAtj2/ClwR7jAYo=";
  };

  # Upstream doesn't commit Cargo.lock, so we vendor dependencies ourselves
  cargoDeps = rustPlatform.importCargoLock {
    lockFile = ./igvm-cargo.lock;
  };

  nativeBuildInputs = [
    cargo
    rustc
    rust-cbindgen
    rustPlatform.cargoSetupHook
  ];

  postPatch = ''
    # Upstream doesn't have Cargo.lock, provide our own
    cp ${./igvm-cargo.lock} Cargo.lock

    # Fix shebangs for Nix build environment
    patchShebangs igvm_c/scripts/
  '';

  buildPhase = ''
    runHook preBuild

    # Build the Rust library with C API exports
    cargo build \
      --release \
      --frozen \
      --offline \
      --manifest-path=igvm/Cargo.toml \
      --features igvm-c

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    # Install static library
    install -Dm644 target/release/libigvm.a $out/lib/libigvm.a

    # Generate C headers using upstream's cbindgen configuration
    mkdir -p $out/include/igvm
    cbindgen -q -c igvm_c/cbindgen_igvm.toml igvm -o $out/include/igvm/igvm.h
    cbindgen -q -c igvm_c/cbindgen_igvm_defs.toml igvm_defs -o $out/include/igvm/igvm_defs.h

    # Run upstream's post-processing script to add IGVM_ prefixes
    # This ensures compatibility with QEMU's expected symbol names
    bash igvm_c/scripts/post_process.sh $out/include/igvm

    # Create pkg-config file for QEMU's meson build system
    mkdir -p $out/lib/pkgconfig
    cat > $out/lib/pkgconfig/igvm.pc << EOF
    prefix=$out
    exec_prefix=\''${prefix}
    libdir=\''${prefix}/lib
    includedir=\''${prefix}/include

    Name: igvm
    Description: IGVM file format parser library
    Version: ${version}
    Libs: -L\''${libdir} -ligvm
    Libs.private: -ldl -lpthread -lm
    Cflags: -I\''${includedir}/igvm
    EOF

    runHook postInstall
  '';

  # Skip tests - they require CUnit and additional test data generation
  doCheck = false;

  meta = with lib; {
    description = "Independent Guest Virtual Machine (IGVM) file format library";
    longDescription = ''
      The IGVM file format is designed to encapsulate all information required
      to launch a virtual machine on any given virtualization stack, with support
      for different isolation technologies such as AMD SEV-SNP and Intel TDX.

      This package provides the C API library (libigvm) which is required by
      QEMU for IGVM support.
    '';
    homepage = "https://github.com/microsoft/igvm";
    changelog = "https://github.com/microsoft/igvm/releases/tag/igvm-v${version}";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    maintainers = [ ];
  };
}
