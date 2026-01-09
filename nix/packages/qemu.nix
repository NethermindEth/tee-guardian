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


# QEMU with Confidential Computing Support (TDX + SEV-SNP + IGVM)
#
# Single QEMU build for all confidential computing workloads.
# No feature flags - everything is always enabled.

{ lib
, stdenv
, fetchurl
, makeWrapper
, removeReferencesTo
, pkg-config
, meson
, ninja
, python3
, bison
, flex
, perl
, dtc
, glib
, gnutls
, zlib
, pixman
, libslirp
, numactl
, liburing
, libcap_ng
, libcap
, attr
, libaio
, libigvm
}:

stdenv.mkDerivation rec {
  pname = "qemu-coco";
  version = "10.2.0";

  src = fetchurl {
    url = "https://download.qemu.org/qemu-${version}.tar.xz";
    hash = "sha256-njCtG4ufe0RjABWC0aspfznPzOpdCFQMDKbWZyeFiDo=";
  };

  dontUseMesonConfigure = true;
  dontAddStaticConfigureFlags = true;

  nativeBuildInputs = [
    makeWrapper removeReferencesTo pkg-config meson ninja python3 bison flex perl dtc
  ];

  buildInputs = [
    glib gnutls zlib pixman dtc libslirp numactl liburing libcap_ng libcap attr libaio
    libigvm  # Required for --enable-igvm (IGVM file format support)
  ];

  postPatch = ''
    substituteInPlace configs/devices/i386-softmmu/default.mak \
      --replace '#CONFIG_TDX=n' 'CONFIG_TDX=y'
  '';

  preConfigure = ''
    unset CPP
    chmod +x ./scripts/shaderinclude.py
    patchShebangs .
    mv VERSION QEMU_VERSION
    substituteInPlace configure --replace '$source_path/VERSION' '$source_path/QEMU_VERSION'
    substituteInPlace meson.build --replace "'VERSION'" "'QEMU_VERSION'"
  '';

  configureFlags = [
    "--target-list=x86_64-softmmu"
    "--prefix=${placeholder "out"}"
    "--datadir=${placeholder "out"}/share"
    "--firmwarepath=${placeholder "out"}/share/qemu-firmware"
    # Enabled
    "--enable-kvm"
    "--enable-numa"
    "--enable-linux-aio"
    "--enable-linux-io-uring"
    "--enable-gnutls"
    "--enable-slirp"
    "--enable-pixman"
    "--enable-attr"
    "--enable-igvm"
    "--enable-malloc-trim"
    "--enable-coroutine-pool"
    "--with-coroutine=ucontext"
    # Disabled
    "--disable-user"
    "--disable-linux-user"
    "--disable-bsd-user"
    "--disable-guest-agent"
    "--disable-docs"
    "--disable-gtk"
    "--disable-sdl"
    "--disable-cocoa"
    "--disable-curses"
    "--disable-vnc"
    "--disable-spice"
    "--disable-pa"
    "--disable-alsa"
    "--disable-jack"
    "--disable-pipewire"
    "--disable-coreaudio"
    "--disable-dsound"
    "--disable-oss"
    "--disable-smartcard"
    "--disable-usb-redir"
    "--disable-libusb"
    "--disable-opengl"
    "--disable-virglrenderer"
    "--disable-rutabaga-gfx"
    "--disable-rbd"
    "--disable-glusterfs"
    "--disable-libiscsi"
    "--disable-libnfs"
    "--disable-libssh"
    "--disable-curl"
    "--disable-gcrypt"
    "--disable-nettle"
    "--localstatedir=/var"
    "--sysconfdir=/etc"
  ];

  preBuild = "cd build";
  doCheck = false;

  postInstall = ''
    # Validate CC features
    $out/bin/qemu-system-x86_64 -object help 2>/dev/null | grep -q "tdx-guest" || exit 1
    $out/bin/qemu-system-x86_64 -object help 2>/dev/null | grep -q "sev-snp-guest" || exit 1
    $out/bin/qemu-system-x86_64 -object help 2>/dev/null | grep -q "igvm-cfg" || exit 1
    $out/bin/qemu-system-x86_64 -machine q35,help 2>/dev/null | grep -q "confidential-guest-support" || exit 1
    mkdir -p $out/share/qemu $out/share/qemu-firmware
  '';

  meta = with lib; {
    description = "QEMU with TDX, SEV-SNP, and IGVM support";
    homepage = "https://www.qemu.org/";
    license = licenses.gpl2Plus;
    platforms = [ "x86_64-linux" ];
    mainProgram = "qemu-system-x86_64";
  };
}
