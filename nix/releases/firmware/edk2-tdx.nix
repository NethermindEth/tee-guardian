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


# EDK2 OVMF Firmware for Intel TDX
#
# Builds OVMF with Intel TDX guest support (IntelTdxX64.dsc platform).
# This firmware is required for launching TDX confidential VMs.
#
# Output: OVMF.fd - Combined code and variable store firmware image
#
# Usage in QEMU:
#   -bios /path/to/OVMF.fd
#   OR
#   -machine kernel-irqchip=split,confidential-guest-support=tdx0

{ lib
, stdenv
, buildPackages
, fetchFromGitHub
, fetchpatch
, applyPatches
, python3
, nasm
, perl
, bc
, acpica-tools
, util-linuxMinimal

  # Build configuration
, buildType ? "DEBUG"  # DEBUG or RELEASE
, secureBoot ? false
, buildShell ? true
}:

let
  version = "edk2-stable202505";

  src = applyPatches {
    name = "edk2-tdx-src";
    src = fetchFromGitHub {
      owner = "tianocore";
      repo = "edk2";
      rev = version;
      fetchSubmodules = true;
      hash = "sha256-VuiEqVpG/k7pfy0cOC6XmY+8NBtU/OHdDB9Y52tyNe8=";
    };

    patches = [
      # Cross-compilation support from Fedora
      (fetchpatch {
        url = "https://src.fedoraproject.org/rpms/edk2/raw/08f2354cd280b4ce5a7888aa85cf520e042955c3/f/0021-Tweak-the-tools_def-to-support-cross-compiling.patch";
        hash = "sha256-E1/fiFNVx0aB1kOej2DJ2DlBIs9tAAcxoedym2Zhjxw=";
      })
      # Fix ANTLR/DLG cross-compilation
      (fetchpatch {
        name = "fix-cross-compilation-antlr-dlg.patch";
        url = "https://github.com/tianocore/edk2/commit/a34ff4a8f69a7b8a52b9b299153a8fac702c7df1.patch";
        hash = "sha256-u+niqwjuLV5tNPykW4xhb7PW2XvUmXhx5uvftG1UIbU=";
      })
    ];

    postPatch = ''
      # Fix clang warning treated as error
      substituteInPlace BaseTools/Conf/tools_def.template --replace-fail \
        'DEFINE CLANGPDB_WARNING_OVERRIDES    = ' \
        'DEFINE CLANGPDB_WARNING_OVERRIDES    = -Wno-unneeded-internal-declaration '
    '';
  };

in
stdenv.mkDerivation {
  pname = "edk2-tdx";
  inherit version;
  inherit src;

  # Reproducible build
  SOURCE_DATE_EPOCH = "1";

  nativeBuildInputs = [
    python3
    nasm
    perl
    bc
    acpica-tools
  ];

  buildInputs = [
    util-linuxMinimal
    util-linuxMinimal.dev
  ];

  depsBuildBuild = [
    buildPackages.stdenv.cc
    buildPackages.bash
  ];

  strictDeps = true;

  GCC5_X64_PREFIX = stdenv.cc.targetPrefix;
  PYTHON_COMMAND = "${buildPackages.python3}/bin/python3";

  env.NIX_CFLAGS_COMPILE = "-Wno-return-type"
    + lib.optionalString stdenv.cc.isGNU " -Wno-error=stringop-truncation";

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
    runHook preBuild

    # Build base tools
    make -C BaseTools -j$(nproc)

    # Patch shebangs for build wrappers
    for i in BaseTools/BinWrappers/PosixLike/*; do
      [ -f "$i" ] && chmod +x "$i" && patchShebangs --build "$i"
    done
    patchShebangs --build BaseTools/BinPipWrappers/PosixLike/* || true
    patchShebangs --build BaseTools/Source/Python/* || true

    # Set up EDK2 environment
    source ./edksetup.sh BaseTools

    # Build TDX firmware
    echo "Building EDK2 TDX firmware (${buildType})..."
    build -a X64 -b ${buildType} -t GCC5 \
      -p OvmfPkg/IntelTdx/IntelTdxX64.dsc \
      -n $(nproc) \
      -D SECURE_BOOT_ENABLE=${if secureBoot then "TRUE" else "FALSE"} \
      -D BUILD_SHELL=${if buildShell then "TRUE" else "FALSE"}

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out

    # Copy firmware image
    cp Build/IntelTdx/${buildType}_GCC5/FV/OVMF.fd $out/OVMF.fd

    # Create metadata
    cat > $out/metadata.json <<EOF
    {
      "platform": "IntelTdx",
      "buildType": "${buildType}",
      "secureBoot": ${if secureBoot then "true" else "false"},
      "version": "${version}"
    }
    EOF

    echo "Built TDX firmware: $(ls -lh $out/OVMF.fd | awk '{print $5}')"

    runHook postInstall
  '';

  enableParallelBuilding = true;

  passthru = {
    inherit buildType secureBoot;
  };

  meta = with lib; {
    description = "EDK2 OVMF firmware for Intel TDX confidential VMs";
    homepage = "https://github.com/tianocore/edk2";
    license = licenses.bsd2;
    platforms = [ "x86_64-linux" ];
  };
}
