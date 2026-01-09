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


{ config, lib, pkgs, ... }:

{
  imports = [
    ../common.nix
    ../modules/snp-host.nix
    ../modules/development
    ../hardware-configs/hetzner-ax-162.nix
  ];

  networking.hostName = "hetzner-sev-dev";

  # === Nebula Overlay Network ===

  guardian.development = {
    enable = true;
    nebula = {
      hostId = 1;  # Primary SEV host
      teeType = "sev";
      publicAddress = "PLACEHOLDER";  # TODO: Set actual public IP
      userCertApi = true;  # This host runs the developer cert API
      runDockerServices = true;
      lighthouseHosts = {
        # Add other lighthouses here when they come online
      };
    };
  };

  # === SEV-SNP Host Configuration ===

  services.snp-host = {

    kernel = {
      version = "6.18.3";
      sha256 = "sha256-eoh5FnuJxLrgd9bznE8hMHafBdva0qrZFK2rmvt9f5o=";
    };

    enable = true;

    # VCEK certificate management
    attestation = {
      enable = true;
      certCachePath = "/var/lib/amd-sev";
      refreshSchedule = "weekly";
    };

    # SVSM for vTPM support
    svsm.enable = true;

    # Device access
    sevUsers = [ "qemu" ];

    # Guest defaults
    guestDefaults = {
      policy = "0x30000";  # Production (Debug disabled, Migration disabled)
    };
  };

  # === Secrets ===

  sops = {
    defaultSopsFile = ../secrets/hetzner-sev-dev.yaml;
    age.keyFile = "/var/lib/sops-nix/key.txt";

    # Nebula CA (shared across all hosts)
    secrets."nebula-ca-crt" = {
      sopsFile = ../secrets/shared.yaml;
      restartUnits = [ "nebula-cert-setup.service" "nebula.service" ];
      mode = "0644";
    };

    secrets."nebula-ca-key" = {
      sopsFile = ../secrets/shared.yaml;
      restartUnits = [ "nebula-cert-setup.service" "nebula-user-cert-api.service" ];
      mode = "0600";
    };
  };

  # === Hardware Configuration ===

  host = {
    bootDrives = {
      primary = {
        id = "PLACEHOLDER";  # TODO: Set actual NVMe drive ID
        pciAddress = "PLACEHOLDER";  # TODO: Set actual PCI address
      };
      mirror = {
        id = "PLACEHOLDER";  # TODO: Set actual NVMe drive ID
        pciAddress = "PLACEHOLDER";  # TODO: Set actual PCI address
      };
    };

    passthroughDrives = [
      # TODO: Add passthrough drives if needed
    ];

    extraAuthorizedKeys = [
      # Nethermind
      "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQDgDA8a/EFrgf2Vzr7+Qnh1UBzu/l5xX1e/vMtNs1hiwdPCfjv/MisPidTlvU5X1tUvAGUZodX871FdnNX1EfRbWxX2kvURaM0GPJRhzCI+vmohH365qix4/HDUCVCFMGwDV8J6n3SgOYoOfGTOaFt+Q1Xmw8hHQfGOdxrh2AYWsGEjOhen4lPhZVDKzUB6+ZQmFnDWS9nd7ds8YOJ6ryxgdEICaD+rPSCDaRDJy5iHM4hyNITTm50pCR+oeYZ1Ay8q5ec3XEmpFGQSw4Roz5LV95TIfb0U7In8TTPGFrIPkxsvrEhBIdAVTcJXctHC4Ei2kOCAz0ArM0qA/L/Lpu7BNb/7eNHICEekTGx7v2tPqiE8+zTU8r7P2f5jWLcVYcJX8Xmj9xzBccR8Jo21+oujwo9Z2Yae94cdDkQeSQpASi/lZo7u7X7dfmUU70pypaDJhNwJv2GGRjRUPHFxVDMkRWJTGI0+QG8MoPMneOuolfOi7oSfrJ8/BrW3SlOOFgd73pvplZ4op/EwPCNKPgsig8oh24KOPxOD3C4hOPVr5OK7TVhG0KuHGeOkUgbtdC7RBcmwXWCKbmZ6xfrxXwvtuagWp5/6d3Cu96K2Q3dhVbh/DSaJH1uMKnEW0fsuB8xXj/YI5GrpaLIFNBpIibMiwOh3EJQQCawldKBJFRN3WQ== elicb@elicb-xps-wsl"
    ];
  };
}
