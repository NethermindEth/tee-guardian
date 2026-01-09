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


# Common NixOS Configuration
#
# Base configuration shared by all Guardian hosts including boot drives,
# RAID1 disk layout, SSH configuration, user management, and VFIO passthrough.

{ config, pkgs, lib, ... }:

{
  # === Host Options ===

  options.host = {
    bootDrives = lib.mkOption {
      type = lib.types.attrs;
      description = "Boot drives configuration (primary/mirror)";
      example = {
        primary = { id = "nvme-SAMSUNG..."; pciAddress = "0000:be:00.0"; };
        mirror = { id = "nvme-SAMSUNG..."; pciAddress = "0000:bf:00.0"; };
      };
    };

    passthroughDrives = lib.mkOption {
      type = lib.types.listOf lib.types.attrs;
      default = [];
      description = "NVMe drives to passthrough to VMs";
    };

    extraAuthorizedKeys = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Additional SSH keys for this host";
    };
  };

  # === Disk Layout (RAID1 Boot) ===

  config.disko.devices = {
    disk = {
      nvme0 = {
        type = "disk";
        device = "/dev/disk/by-id/${config.host.bootDrives.primary.id}";
        content = {
          type = "gpt";
          partitions = {
            ESP = {
              size = "512M";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
                mountpoint = "/boot";
                mountOptions = [ "umask=0077" ];
              };
            };
            root = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "root_raid";
              };
            };
          };
        };
      };
      nvme1 = {
        type = "disk";
        device = "/dev/disk/by-id/${config.host.bootDrives.mirror.id}";
        content = {
          type = "gpt";
          partitions = {
            ESP_backup = {
              size = "512M";
              type = "EF00";
              content = {
                type = "filesystem";
                format = "vfat";
              };
            };
            root = {
              size = "100%";
              content = {
                type = "mdraid";
                name = "root_raid";
              };
            };
          };
        };
      };
    };
    mdadm = {
      root_raid = {
        type = "mdadm";
        level = 1;
        content = {
          type = "filesystem";
          format = "ext4";
          mountpoint = "/";
          mountOptions = [ "defaults" "noatime" ];
        };
      };
    };
  };

  # === Pre-deployment Assertions ===

  config.system.preSwitchChecks = {
    verify-boot-drives = ''
      echo "Verifying boot drives..."
      for drive in "${config.host.bootDrives.primary.id}" "${config.host.bootDrives.mirror.id}"; do
        if [ ! -e "/dev/disk/by-id/$drive" ]; then
          echo "DEPLOYMENT ABORTED: Boot drive not found: $drive"
          exit 1
        fi
        echo "Boot drive verified: $drive"
      done
    '';
  };

  # === Base System Configuration ===

  config.system.stateVersion = "25.05";
  config.time.timeZone = "UTC";
  config.i18n.defaultLocale = "en_US.UTF-8";

  config.users = {
    mutableUsers = false;
    allowNoPasswordLogin = true;

    # QEMU group for VM management (used by both TDX and SNP hosts)
    groups.qemu = {};

    users.root = {
      openssh.authorizedKeys.keys = [
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIG0TA+6GXPF5JNx2gs86fPM2znIS1Yv6cUauUjjQhQdM devops@nethermind.io"
      ] ++ config.host.extraAuthorizedKeys;
      hashedPassword = "!";
      shell = pkgs.bash;
      extraGroups = [ "qemu" ];
    };
  };

  config.services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "prohibit-password";
      X11Forwarding = false;
      GatewayPorts = "no";
      MaxAuthTries = 2;
      MaxStartups = "2:30:10";
      LoginGraceTime = 30;
      ClientAliveInterval = 300;
      ClientAliveCountMax = 2;
    };
  };

  config.boot.loader = {
    systemd-boot.enable = true;
    efi.canTouchEfiVariables = true;
  };

  # === VFIO PCI Passthrough ===

  config.boot.kernelModules = lib.mkIf (config.host.passthroughDrives != []) [ "vfio-pci" ];

  config.systemd.services.vfio-nvme-unbind = lib.mkIf (config.host.passthroughDrives != []) {
    description = "Unbind NVMe devices for VFIO passthrough";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-modules-load.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = ''
      PCI_ADDRS=(${lib.concatMapStringsSep " " (drive: ''"${drive.pciAddress}"'') config.host.passthroughDrives})

      for PCI_ADDR in "''${PCI_ADDRS[@]}"; do
        echo "vfio-pci" > /sys/bus/pci/devices/$PCI_ADDR/driver_override

        if [ -e /sys/bus/pci/devices/$PCI_ADDR/driver ]; then
          CURRENT_DRIVER=$(basename $(readlink /sys/bus/pci/devices/$PCI_ADDR/driver))
          if [ "$CURRENT_DRIVER" = "vfio-pci" ]; then
            continue
          fi
          echo $PCI_ADDR > /sys/bus/pci/devices/$PCI_ADDR/driver/unbind
        fi

        echo $PCI_ADDR > /sys/bus/pci/drivers_probe
      done
    '';
  };
}
