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


# Nebula Overlay Network Module
#
# Configures Nebula mesh VPN for multi-TEE test infrastructure.
#
# Architecture:
#   - Shared CA across all hosts (via sops-nix secrets)
#   - All hosts act as lighthouses
#   - One host runs the user cert API (for developer certs)
#   - Host certs auto-generated on first boot
#   - VM certs pre-generated (slots 10-109)
#
# Network Layout:
#   10.42.0.0/24   - Developers (10.42.0.1-64, dynamic allocation, 24h TTL)
#   10.42.1.0/24   - TDX host (10.42.1.1) + TDX VMs (10.42.1.10-109)
#   10.42.2.0/24   - SEV host (10.42.2.1) + SEV VMs (10.42.2.10-109)
#   10.42.100.0/24 - Docker services (routed via host running them)
#
# Secrets Required (via sops-nix):
#   nebula-ca-crt - CA certificate (public, all hosts)
#   nebula-ca-key - CA private key (secret, only on cert API host)

{ config, lib, pkgs, ... }:

let
  inherit (lib)
    mkEnableOption mkOption mkIf mkMerge types
    optional optionalString optionalAttrs attrNames
    genList toLower toHexString splitString last toInt head attrValues
    concatMapStringsSep;

  cfg = config.services.guardian-nebula;

  # Network Configuration
  nebulaIp = "10.42.${toString cfg.hostId}.1";
  vmSubnet = "10.42.${toString cfg.hostId}.0/24";
  dockerSubnet = "10.42.100.0/24";
  devSubnet = "10.42.0.0/24";

  # Certificate Paths
  caDir = "/var/lib/nebula/ca";
  caCert = "${caDir}/ca.crt";
  caKey = "${caDir}/ca.key";
  hostCert = "/var/lib/nebula/host.crt";
  hostKey = "/var/lib/nebula/host.key";
  vmCertsDir = "/var/lib/nebula/vm-certs";

  # Build lighthouse hosts map including self
  allLighthouseHosts = cfg.lighthouseHosts // {
    "${nebulaIp}" = "${cfg.publicAddress}:${toString cfg.listenPort}";
  };

  # Nebula Configuration
  nebulaConfig = {
    pki = { ca = caCert; cert = hostCert; key = hostKey; };
    static_host_map = cfg.lighthouseHosts;
    lighthouse = {
      am_lighthouse = true;  # All hosts are lighthouses
      interval = 60;
      hosts = [];  # Empty when lighthouse
    };
    listen = { host = "0.0.0.0"; port = cfg.listenPort; };
    punchy = { punch = true; respond = true; };
    tun = {
      disabled = false;
      dev = cfg.interfaceName;
      drop_local_broadcast = false;
      drop_multicast = false;
      tx_queue = 500;
      mtu = 1300;
      unsafe_routes =
        [{ route = vmSubnet; via = nebulaIp; }]
        ++ [{ route = devSubnet; via = nebulaIp; }]
        ++ optional cfg.advertiseDockerSubnet { route = dockerSubnet; via = nebulaIp; };
    };
    firewall = {
      outbound = [{ port = "any"; proto = "any"; host = "any"; }];
      inbound = [
        { port = "any"; proto = "any"; groups = ["guardian-vms"]; }
        { port = "any"; proto = "any"; groups = ["tee-hosts"]; }
        { port = "any"; proto = "any"; groups = ["developers"]; }
        { port = "any"; proto = "icmp"; host = "any"; }
      ];
    };
    logging = { level = cfg.logLevel; format = "json"; };
  };

  nebulaConfigFile = pkgs.writeText "nebula-config.yaml" (builtins.toJSON nebulaConfig);

  # Certificate Setup Script
  # Copies CA from sops secrets, generates host cert if missing
  certSetupScript = pkgs.writeShellScript "nebula-cert-setup" ''
    set -euo pipefail

    CA_DIR="${caDir}"
    VM_CERTS_DIR="${vmCertsDir}"

    mkdir -p "$CA_DIR" /var/lib/nebula "$VM_CERTS_DIR"

    # Copy CA cert from sops secret
    if [ -f "${cfg.caCertFile}" ]; then
      cp "${cfg.caCertFile}" "$CA_DIR/ca.crt"
      chmod 644 "$CA_DIR/ca.crt"
      echo "CA certificate installed from sops secret"
    else
      echo "ERROR: CA certificate not found at ${cfg.caCertFile}"
      echo "Ensure sops secret 'nebula-ca-crt' is configured"
      exit 1
    fi

    # Copy CA key if available (only on cert API host)
    if [ -f "${cfg.caKeyFile}" ]; then
      cp "${cfg.caKeyFile}" "$CA_DIR/ca.key"
      chmod 600 "$CA_DIR/ca.key"
      echo "CA key installed from sops secret"
    fi

    # Generate host cert if missing
    if [ ! -f "${hostCert}" ]; then
      if [ ! -f "$CA_DIR/ca.key" ]; then
        echo "ERROR: Cannot generate host cert without CA key"
        echo "This host needs the 'nebula-ca-key' sops secret"
        exit 1
      fi

      echo "Generating host certificate..."
      ${pkgs.nebula}/bin/nebula-cert sign \
        -ca-crt "$CA_DIR/ca.crt" \
        -ca-key "$CA_DIR/ca.key" \
        -name "${config.networking.hostName}" \
        -ip "${nebulaIp}/24" \
        -groups "tee-hosts,${cfg.teeType}" \
        -duration "8760h" \
        -out-crt "${hostCert}" \
        -out-key "${hostKey}"
      chmod 600 "${hostKey}"
      echo "Host certificate generated: ${nebulaIp}"
    fi

    # Generate VM certs (slots 10-109) if CA key is available
    if [ -f "$CA_DIR/ca.key" ]; then
      echo "Generating VM certificates..."
      for slot in $(seq 10 109); do
        if [ ! -f "$VM_CERTS_DIR/slot-$slot.crt" ]; then
          ${pkgs.nebula}/bin/nebula-cert sign \
            -ca-crt "$CA_DIR/ca.crt" \
            -ca-key "$CA_DIR/ca.key" \
            -name "vm-$slot" \
            -ip "10.42.${toString cfg.hostId}.$slot/24" \
            -groups "guardian-vms,${cfg.teeType}" \
            -duration "8760h" \
            -out-crt "$VM_CERTS_DIR/slot-$slot.crt" \
            -out-key "$VM_CERTS_DIR/slot-$slot.key"
          chmod 644 "$VM_CERTS_DIR/slot-$slot.crt"
          chmod 600 "$VM_CERTS_DIR/slot-$slot.key"
        fi
      done
      echo "VM certificates ready"
    fi

    echo "Certificate setup complete"
  '';

  # DHCP Host Generation for VMs
  generateTestVMs = start: end:
    genList (i: let
      n = start + i;
      mac = "52:54:00:12:34:${toLower (toHexString n)}";
    in {
      inherit mac;
      ip = "192.168.122.${toString n}";
      hostname = "vm-${toString n}";
      nebulaIp = "10.42.${toString cfg.hostId}.${toString n}";
    }) (end - start + 1);

  testVMs = generateTestVMs 10 109;

  mkDhcpHost = vm:
    let slot = toInt (last (splitString "." vm.ip));
    in "${vm.mac},set:vm${toString slot},${vm.ip},${vm.hostname},infinite";

  mkDhcpOption = vm:
    let slot = toInt (last (splitString "." vm.ip));
    in "tag:vm${toString slot},224,${last (splitString "." vm.ip)}";

in {
  options.services.guardian-nebula = {
    enable = mkEnableOption "Guardian Nebula overlay network";

    hostId = mkOption {
      type = types.int;
      description = "Host ID (1=TDX, 2=SEV, etc). Determines subnet 10.42.{hostId}.0/24";
    };

    teeType = mkOption {
      type = types.enum [ "tdx" "sev" ];
      description = "TEE type for this host";
    };

    publicAddress = mkOption {
      type = types.str;
      description = "Public IP address of this host (for lighthouse)";
      example = "51.210.181.125";
    };

    lighthouseHosts = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Map of Nebula IPs to public host:port for other lighthouses";
      example = { "10.42.1.1" = "51.210.181.125:4242"; };
    };

    listenPort = mkOption {
      type = types.port;
      default = 4242;
      description = "UDP port for Nebula";
    };

    caCertFile = mkOption {
      type = types.path;
      description = "Path to CA certificate (from sops secret)";
      example = "/run/secrets/nebula-ca-crt";
    };

    caKeyFile = mkOption {
      type = types.path;
      default = "/run/secrets/nebula-ca-key";
      description = "Path to CA private key (from sops secret, optional for non-cert-API hosts)";
    };

    advertiseDockerSubnet = mkOption {
      type = types.bool;
      default = false;
      description = "Advertise Docker services subnet via Nebula";
    };

    logLevel = mkOption {
      type = types.enum [ "debug" "info" "warn" "error" ];
      default = "info";
      description = "Nebula logging level";
    };

    interfaceName = mkOption {
      type = types.str;
      default = "nebula-guardian";
      description = "Name of the Nebula tunnel interface";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ pkgs.nebula ];

    # Firewall
    networking.firewall.allowedUDPPorts = [ cfg.listenPort ];

    # IP Forwarding
    boot.kernel.sysctl = {
      "net.ipv4.ip_forward" = 1;
      "net.ipv4.conf.all.forwarding" = 1;
    };

    # Directories
    systemd.tmpfiles.rules = [
      "d /var/lib/nebula 0750 root root -"
      "d ${caDir} 0700 root root -"
      "d ${vmCertsDir} 0755 root root -"
    ];

    # Certificate Setup Service
    systemd.services.nebula-cert-setup = {
      description = "Setup Nebula certificates from sops secrets";
      wantedBy = [ "nebula.service" ];
      before = [ "nebula.service" ];
      after = [ "sops-nix.service" ];
      wants = [ "sops-nix.service" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = certSetupScript;
        RemainAfterExit = true;
      };
    };

    # Nebula Service
    systemd.services.nebula = {
      description = "Nebula overlay network";
      after = [ "network.target" "nebula-cert-setup.service" ];
      wants = [ "nebula-cert-setup.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.nebula}/bin/nebula -config ${nebulaConfigFile}";
        Restart = "always";
        RestartSec = "5s";
        AmbientCapabilities = [ "CAP_NET_ADMIN" ];
        CapabilityBoundingSet = [ "CAP_NET_ADMIN" ];
      };
    };

    # VM Bridge (br0)
    systemd.network.netdevs."40-br0" = {
      netdevConfig = { Kind = "bridge"; Name = "br0"; };
    };

    systemd.network.networks."40-br0" = {
      matchConfig.Name = "br0";
      networkConfig = {
        Address = "192.168.122.1/24";
        ConfigureWithoutCarrier = true;
        DHCPServer = false;
        IPv6AcceptRA = false;
      };
      linkConfig.RequiredForOnline = false;
    };

    # DHCP for VMs
    services.dnsmasq = {
      enable = true;
      settings = {
        interface = "br0";
        bind-interfaces = true;
        domain = "guardian.local";
        expand-hosts = true;
        local = "/guardian.local/";
        dhcp-range = "192.168.122.128,192.168.122.254,infinite";
        dhcp-host = [ "52:54:00:12:34:0a,set:dev,192.168.122.10,dev-vm,infinite" ]
          ++ map mkDhcpHost testVMs;
        dhcp-option = [ "option:router,192.168.122.1" "option:dns-server,192.168.122.1" ];
        dhcp-option-force = [ "tag:dev,224,10" ] ++ map mkDhcpOption testVMs;
        server = [ "8.8.8.8" "1.1.1.1" ];
        address = [
          "/measurement-registry.guardian.local/10.42.100.2"
          "/openobserve.guardian.local/10.42.100.100"
        ];
        cache-size = 1000;
        no-negcache = true;
        localise-queries = true;
        log-dhcp = true;
      };
    };

    # Firewall Rules
    networking.firewall.extraCommands = ''
      iptables -A FORWARD -i ${cfg.interfaceName} -o br0 -j ACCEPT
      iptables -A FORWARD -i br0 -o ${cfg.interfaceName} -j ACCEPT
    '' + optionalString cfg.advertiseDockerSubnet ''
      iptables -A FORWARD -i ${cfg.interfaceName} -o br-guardian-infra -j ACCEPT 2>/dev/null || true
      iptables -A FORWARD -i br-guardian-infra -o ${cfg.interfaceName} -j ACCEPT 2>/dev/null || true
    '';
  };
}
