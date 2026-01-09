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


# VM Compatibility Module
#
# Enables `nixos-rebuild build-vm` for hosts that use sops-nix secrets.
# Disables sops and secret-dependent services in vmVariant while providing
# a test user for VM access.
#
# This module is automatically included in all hosts via mk-host.nix.

{ config, lib, ... }:

{
  virtualisation.vmVariant = {
    # Disable sops entirely for VMs (secrets can't be decrypted without host keys)
    sops.defaultSopsFile = lib.mkForce null;
    sops.secrets = lib.mkForce {};
    sops.templates = lib.mkForce {};

    # Disable services that require secrets
    guardian.development.enable = lib.mkForce false;

    # Add a test user for VM console access
    users.users.test = {
      isNormalUser = true;
      initialPassword = "test";
      extraGroups = [ "wheel" ];
    };

    # Allow passwordless sudo for test user
    security.sudo.wheelNeedsPassword = lib.mkForce false;

    # VM resources
    virtualisation = {
      memorySize = 4096;
      cores = 2;
      graphics = false;
    };
  };
}
