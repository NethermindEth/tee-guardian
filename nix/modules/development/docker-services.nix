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


# Docker Services Module for Guardian Test Infrastructure
#
# Runs the test infrastructure Docker services on a dedicated subnet routed
# via Nebula overlay network, allowing VMs on any TEE host to reach services.
#
# Services:
#   - measurement-registry proxy (10.42.100.2)
#   - registry-cluster-{0-9} backends (10.42.100.10-19)
#   - openobserve (10.42.100.100)

{ config, lib, pkgs, ... }:

let
  inherit (lib) mkEnableOption mkOption mkIf types range listToAttrs concatMapStrings concatStringsSep;

  cfg = config.services.guardian-docker-services;

  dockerSubnet = "10.42.100.0/24";
  dockerGateway = "10.42.100.1";
  nebulaInterface = "nebula-guardian";
  registryClusterCount = 10;

  # === Service IP Assignments ===
  serviceIps = {
    measurement-registry = "10.42.100.2";
    openobserve = "10.42.100.100";
  } // listToAttrs (map (i: {
    name = "registry-cluster-${toString i}";
    value = "10.42.100.${toString (10 + i)}";
  }) (range 0 (registryClusterCount - 1)));

  # === Docker Compose Override Generation ===
  mkServiceEntry = name: ip: ''
      ${name}:
        networks:
          guardian-infra:
            ipv4_address: ${ip}
  '';

  registryClusterEntries = concatMapStrings
    (i: mkServiceEntry "registry-cluster-${toString i}" serviceIps."registry-cluster-${toString i}")
    (range 0 (registryClusterCount - 1));

  composeOverride = pkgs.writeText "docker-compose.nebula.yml" ''
    # Auto-generated override for Nebula routing
    # Services get static IPs on ${dockerSubnet} subnet

    services:
    ${mkServiceEntry "measurement-registry" serviceIps.measurement-registry}
    ${registryClusterEntries}
      openobserve:
        networks:
          guardian-infra:
            ipv4_address: ${serviceIps.openobserve}
        ports: []

    networks:
      guardian-infra:
        driver: bridge
        ipam:
          config:
            - subnet: ${dockerSubnet}
              gateway: ${dockerGateway}

      guardian:
        external: false
        name: guardian-legacy
  '';

  # === Scripts ===
  createNetworkScript = pkgs.writeShellScript "guardian-docker-network-create" ''
    set -euo pipefail
    ${pkgs.docker}/bin/docker network rm guardian-infra 2>/dev/null || true
    ${pkgs.docker}/bin/docker network inspect guardian-infra >/dev/null 2>&1 || \
      ${pkgs.docker}/bin/docker network create \
        --driver bridge \
        --subnet ${dockerSubnet} \
        --gateway ${dockerGateway} \
        guardian-infra
  '';

  composeUp = pkgs.writeShellScript "guardian-docker-compose-up" ''
    set -euo pipefail
    cd "${cfg.composeDir}"
    exec ${pkgs.docker-compose}/bin/docker-compose \
      -f docker-compose.yml \
      -f ${composeOverride} \
      up -d
  '';

  composeDown = pkgs.writeShellScript "guardian-docker-compose-down" ''
    set -euo pipefail
    cd "${cfg.composeDir}"
    exec ${pkgs.docker-compose}/bin/docker-compose \
      -f docker-compose.yml \
      -f ${composeOverride} \
      down
  '';

in {
  options.services.guardian-docker-services = {
    enable = mkEnableOption "Guardian Docker test services with Nebula routing";

    composeDir = mkOption {
      type = types.path;
      default = ../../../tests/docker;
      description = "Path to directory containing docker-compose.yml";
    };

    autoStart = mkOption {
      type = types.bool;
      default = true;
      description = "Start services automatically at boot";
    };
  };

  config = mkIf cfg.enable {
    virtualisation.docker.enable = true;

    # === Docker Network Setup ===
    systemd.services.guardian-docker-network = {
      description = "Create guardian-infra Docker network";
      after = [ "docker.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = createNetworkScript;
      };
    };

    # === Docker Compose Service ===
    systemd.services.guardian-docker-services = mkIf cfg.autoStart {
      description = "Guardian test infrastructure Docker services";
      after = [ "docker.service" "guardian-docker-network.service" "nebula.service" ];
      wants = [ "guardian-docker-network.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        WorkingDirectory = cfg.composeDir;
        ExecStart = composeUp;
        ExecStop = composeDown;
      };
    };

    # === Firewall Rules for Nebula <-> Docker Routing ===
    networking.firewall.extraCommands = ''
      iptables -A FORWARD -i ${nebulaInterface} -d ${dockerSubnet} -j ACCEPT
      iptables -A FORWARD -s ${dockerSubnet} -o ${nebulaInterface} -j ACCEPT
      iptables -t nat -A POSTROUTING -s ${dockerSubnet} -o ${nebulaInterface} -j MASQUERADE
    '';

    # === Environment Variables ===
    environment.variables = {
      GUARDIAN_MEASUREMENT_REGISTRY = "http://${serviceIps.measurement-registry}:9000";
      GUARDIAN_OPENOBSERVE = "http://${serviceIps.openobserve}:5080";
    };

    environment.etc."guardian/docker-services.json".text = builtins.toJSON {
      network = dockerSubnet;
      services = serviceIps;
    };
  };
}
