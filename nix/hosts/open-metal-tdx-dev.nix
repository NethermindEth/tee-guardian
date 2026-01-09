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


{ config, lib, pkgs, nixpkgs, kexecInstaller ? false, modulesPath, ... }:

{
  imports = [
    ../common.nix
    ../modules/tdx-host.nix
    ../modules/development
  ] ++ lib.optionals (!kexecInstaller) [
    ../hardware-configs/open-metal-xl-v4.nix
  ] ++ lib.optionals kexecInstaller [
    (modulesPath + "/installer/netboot/netboot-minimal.nix")
  ];

  disko.devices = lib.mkIf kexecInstaller (lib.mkForce { });

  # Kexec installer identity - nixos-anywhere detects this and skips its own kexec
  system.nixos.variant_id = lib.mkIf kexecInstaller "installer";

  networking.hostName = "open-metal-tdx-dev";

  # === Nebula Overlay Network ===

  guardian.development = {
    enable = true;
    nebula = {
      hostId = 2;  # TDX host
      teeType = "tdx";
      publicAddress = "173.231.232.149";  # Public IP from bond0.250
      userCertApi = false;  # Secondary host, no cert API
      runDockerServices = false;
      lighthouseHosts = {
        "10.42.1.1" = "PLACEHOLDER:4242";  # Hetzner SEV (primary) - TODO: Set actual IP
      };
    };
  };

  # === TDX Attestation Services ===
  # Disabled for kexec installer (uses standard kernel)

  services.tdx-host = {
    enable = !kexecInstaller;

    kernel = {
      version = "6.18.3";
      sha256 = "sha256-eoh5FnuJxLrgd9bznE8hMHafBdva0qrZFK2rmvt9f5o=";
    };

    vsockPort = 4050;
    useSecureCert = false;
    logLevel = "info";
    enableMpaRegistration = true;
    mpaProxyType = "direct";

    pccs = {
      port = 8081;
      listenAddress = "127.0.0.1";
      logLevel = "info";
      # TODO: This host needs its own sops secret or use a shared PCCS
      apiKey = "placeholder-needs-real-key";
    };
  };

  # === Secrets ===

  sops = {
    defaultSopsFile = ../secrets/open-metal-tdx-dev.yaml;
    age.keyFile = "/var/lib/sops-nix/key.txt";

    # Nebula CA (shared across all hosts)
    # This host only needs the cert, not the key (no cert API)
    secrets."nebula-ca-crt" = {
      sopsFile = ../secrets/shared.yaml;
      restartUnits = [ "nebula-cert-setup.service" "nebula.service" ];
      mode = "0644";
    };

    # CA key needed for generating host cert on first boot
    secrets."nebula-ca-key" = {
      sopsFile = ../secrets/shared.yaml;
      restartUnits = [ "nebula-cert-setup.service" ];
      mode = "0600";
    };
  };

  # === Network Configuration (Bonding + VLANs) ===

  networking = {
    nameservers = [ "8.8.8.8" "1.1.1.1" ];

    bonds.bond0 = {
      driverOptions = {
        mode = "802.3ad";
        miimon = "100";
        xmit_hash_policy = "layer3+4";
      };
      interfaces = [ "ens7f0np0" "ens7f1np1" ];
    };

    vlans = {
      "bond0.8"   = { id = 8;   interface = "bond0"; };
      "bond0.250" = { id = 250; interface = "bond0"; };
      "bond0.251" = { id = 251; interface = "bond0"; };
      "bond0.252" = { id = 252; interface = "bond0"; };
      "bond0.253" = { id = 253; interface = "bond0"; };
      "bond0.254" = { id = 254; interface = "bond0"; };
    };

    interfaces = {
      ens7f0np0.mtu = 9000;
      ens7f1np1.mtu = 9000;
      bond0.mtu = 9000;

      "bond0.8" = {
        useDHCP = true;
        mtu = 1500;
      };

      "bond0.250" = {
        mtu = 1500;
        ipv4.addresses = [{
          address = "173.231.232.149";
          prefixLength = 28;
        }];
      };

      "bond0.251" = {
        mtu = 9000;
        ipv4.addresses = [{
          address = "192.168.2.4";
          prefixLength = 24;
        }];
      };

      "bond0.252" = {
        mtu = 1500;
        ipv4.addresses = [{
          address = "192.168.3.4";
          prefixLength = 24;
        }];
      };

      "bond0.253" = {
        mtu = 9000;
        ipv4.addresses = [{
          address = "192.168.4.4";
          prefixLength = 24;
        }];
      };

      "bond0.254" = {
        mtu = 9000;
        ipv4.addresses = [{
          address = "192.168.5.4";
          prefixLength = 24;
        }];
      };
    };

    defaultGateway = {
      address = "173.231.232.146";
      interface = "bond0.250";
    };
  };

  systemd.network.links = {
    "10-ens7f0np0" = {
      matchConfig.MACAddress = "90:5a:08:0b:c8:32";
      linkConfig.Name = "ens7f0np0";
    };
    "10-ens7f1np1" = {
      matchConfig.MACAddress = "90:5a:08:0b:c8:33";
      linkConfig.Name = "ens7f1np1";
    };
  };

  # === Hardware Configuration ===

  host = {
    bootDrives = {
      primary = {
        id = "nvme-Micron_7450_MTFDKBA960TFR_24374B1B65F5";
        pciAddress = "0000:5a:00.0";
      };
      mirror = {
        id = "nvme-Micron_7450_MTFDKBA960TFR_24374B1B65E5";
        pciAddress = "0000:5b:00.0";
      };
    };

    passthroughDrives = [
      { id = "nvme-MTFDKCC6T4TGQ-1BK1JABYY_4024105E3C19"; pciAddress = "0000:49:00.0"; }
      { id = "nvme-MTFDKCC6T4TGQ-1BK1JABYY_4024105E3D27"; pciAddress = "0000:4a:00.0"; }
      { id = "nvme-MTFDKCC6T4TGQ-1BK1JABYY_4024105E3C2A"; pciAddress = "0000:4b:00.0"; }
      { id = "nvme-MTFDKCC6T4TGQ-1BK1JABYY_4024105E3D73"; pciAddress = "0000:4c:00.0"; }
    ];

    extraAuthorizedKeys = [
      # Nethermind
      "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDgDA8a/EFrgf2Vzr7+Qnh1UBzu/l5xX1e/vMtNs1hiwdPCfjv/MisPidTlvU5X1tUvAGUZodX871FdnNX1EfRbWxX2kvURaM0GPJRhzCI+vmohH365qix4/HDUCVCFMGwDV8J6n3SgOYoOfGTOaFt+Q1Xmw8hHQfGOdxrh2AYWsGEjOhen4lPhZVDKzUB6+ZQmFnDWS9nd7ds8YOJ6ryxgdEICaD+rPSCDaRDJy5iHM4hyNITTm50pCR+oeYZ1Ay8q5ec3XEmpFGQSw4Roz5LV95TIfb0U7In8TTPGFrIPkxsvrEhBIdAVTcJXctHC4Ei2kOCAz0ArM0qA/L/Lpu7BNb/7eNHICEekTGx7v2tPqiE8+zTU8r7P2f5jWLcVYcJX8Xmj9xzBccR8Jo21+oujwo9Z2Yae94cdDkQeSQpASi/lZo7u7X7dfmUU70pypaDJhNwJv2GGRjRUPHFxVDMkRWJTGI0+QG8MoPMneOuolfOi7oSfrJ8/BrW3SlOOFgd73pvplZ4op/EwPCNKPgsig8oh24KOPxOD3C4hOPVr5OK7TVhG0KuHGeOkUgbtdC7RBcmwXWCKbmZ6xfrxXwvtuagWp5/6d3Cu96K2Q3dhVbh/DSaJH1uMKnEW0fsuB8xXj/YI5GrpaLIFNBpIibMiwOh3EJQQCawldKBJFRN3WQ== elicb@elicb-xps-wsl"
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAnDfz+63L2/HxDvEFBWmk9+0p09fB3aIW9FtH8k99HX franco@nethermind.io"
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHlesQZ1Z4xARl7yN72VgZTa4KDpNfyxfPMnEvF01F2K"  # Sam Fan

      # Flashbots
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGLiZbJ4OvpkXhw7WrKpZbB8CRaZF4WiTpPoljHcexy1"  # Igor
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK3Od6tYI09lcB2IPFft31NrpzskA4lLIcVE6rZrzXbl"  # Nico
    ];
  };
}
