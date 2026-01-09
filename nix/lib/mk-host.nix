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


# Host Configuration Factory
#
# Creates NixOS configurations with common infrastructure (disko, sops-nix, overlay)
# pre-configured.
#
# Usage:
#   mkHost = import ./nix/lib/mk-host.nix { inherit nixpkgs system disko sops-nix self; };
#   nixosConfigurations.my-host = mkHost "my-host" { };
#   nixosConfigurations.my-host-kexec = mkHost "my-host" { kexecInstaller = true; };

{ nixpkgs, system, disko, sops-nix, self }:

hostName: {
  # Kexec installer mode - builds a kexec tarball for nixos-anywhere
  kexecInstaller ? false,
  # Additional modules to include
  extraModules ? [],
  ...
}:

let
  hostFile = ../hosts/${hostName}.nix;
in
nixpkgs.lib.nixosSystem {
  inherit system;

  # Pass through to host modules
  specialArgs = {
    inherit nixpkgs system disko sops-nix self kexecInstaller;
  };

  modules = [
    # Apply overlay to make CC packages available in pkgs
    {
      nixpkgs.overlays = [ self.overlays.default ];
      nixpkgs.config.allowUnfree = true;
    }

    # Core infrastructure modules
    disko.nixosModules.disko
    sops-nix.nixosModules.sops

    # VM compatibility (enables build-vm with sops-nix)
    ../../nix/modules/vm-compat.nix

    # Host-specific configuration
    hostFile
  ]
  # Additional modules passed by caller
  ++ extraModules;
}
