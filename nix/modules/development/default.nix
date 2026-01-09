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


# Guardian Development Environment Module
#
# Provides development infrastructure for Guardian TEE hosts including:
#   - Nebula overlay network with VM bridge
#   - User certificate API for developer access
#   - Docker services for test infrastructure
#   - Development packages and tooling
#
# Configuration Example:
#   guardian.development = {
#     enable = true;
#     nebula = {
#       hostId = 1;
#       teeType = "tdx";
#       publicAddress = "51.210.181.125";
#       userCertApi = true;  # Enable on one host only
#     };
#   };

{ config, lib, pkgs, ... }:

let
  inherit (lib) mkEnableOption mkOption mkIf types;

  cfg = config.guardian.development;

in {
  imports = [
    ./nebula.nix
    ./nebula-cert-server.nix
    ./nebula-user-cert-api.nix
    ./docker-services.nix
  ];

  options.guardian.development = {
    enable = mkEnableOption "Guardian development environment";

    nebula = {
      hostId = mkOption {
        type = types.int;
        description = "Nebula host ID (1=TDX, 2=SEV). Determines subnet 10.42.{hostId}.0/24";
        example = 1;
      };

      teeType = mkOption {
        type = types.enum [ "tdx" "sev" ];
        description = "TEE type for this host";
        example = "tdx";
      };

      publicAddress = mkOption {
        type = types.str;
        description = "Public IP address of this host";
        example = "51.210.181.125";
      };

      lighthouseHosts = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "Map of Nebula IPs to public host:port for other lighthouses";
        example = { "10.42.1.1" = "51.210.181.125:4242"; };
      };

      userCertApi = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Enable the user certificate API on this host.
          Only one host should have this enabled.
          This host will issue short-lived certificates to developers.
        '';
      };

      runDockerServices = mkOption {
        type = types.bool;
        default = false;
        description = "Whether this host runs the Docker test infrastructure";
      };
    };
  };

  config = mkIf cfg.enable {
    # Nebula Network Service
    services.guardian-nebula = {
      enable = true;
      hostId = cfg.nebula.hostId;
      teeType = cfg.nebula.teeType;
      publicAddress = cfg.nebula.publicAddress;
      lighthouseHosts = cfg.nebula.lighthouseHosts;
      advertiseDockerSubnet = cfg.nebula.runDockerServices;
      # CA files from sops secrets (configured per-host)
      caCertFile = config.sops.secrets."nebula-ca-crt".path;
      caKeyFile = config.sops.secrets."nebula-ca-key".path;
    };

    # VM Cert Server (for VMs to fetch certs by MAC address)
    services.nebula-cert-server = {
      enable = true;
      listenAddress = "192.168.122.1";
      port = 9999;
    };

    # User Cert API (only on designated host)
    services.nebula-user-cert-api = mkIf cfg.nebula.userCertApi {
      enable = true;
      listenAddress = "127.0.0.1";
      port = 8443;
      caFile = config.sops.secrets."nebula-ca-crt".path;
      caKeyFile = config.sops.secrets."nebula-ca-key".path;
      lighthouseAddress = "${cfg.nebula.publicAddress}:4242";
    };

    # Docker Services
    services.guardian-docker-services.enable = cfg.nebula.runDockerServices;

    # Development Packages
    environment.systemPackages = with pkgs; [
      # TEE/VM tools
      nebula
      qemu-coco
      dmidecode
      pciutils
      kmod
      numactl

      # Debugging
      strace
      lsof
      traceroute
      nmap

      # Development
      openssl
      socat
      neovim
      tmux
      btop
      git
      tree
      lazygit
      jq

      # Languages/Build
      uv
      rustc
      cargo
      gcc
      docker-compose

      # Benchmarking
      stress-ng
      sysbench
      iperf3
      config.boot.kernelPackages.perf
    ];

    # nix-ld for uv Python
    programs.nix-ld = {
      enable = true;
      libraries = with pkgs; [
        stdenv.cc.cc.lib
        zlib
        openssl
        curl
        libz
        glibc
      ];
    };

    # Environment
    environment = {
      variables = {
        CARGO_HOME = "$HOME/.cargo";
      };
      extraInit = ''
        export PATH="$HOME/.cargo/bin:$PATH"
        export PATH="$HOME/.local/bin:$PATH"
      '';
    };

    nix.settings.experimental-features = [ "nix-command" "flakes" ];

    # Networking
    networking = {
      useNetworkd = true;
      enableIPv6 = false;
      firewall.enable = true;
      firewall.trustedInterfaces = [ "br0" ];
      nat = {
        enable = true;
        internalIPs = [ "192.168.122.0/24" ];
      };
    };

    boot.kernel.sysctl = {
      "net.ipv6.conf.all.disable_ipv6" = 1;
      "net.ipv6.conf.default.disable_ipv6" = 1;
      "net.ipv6.conf.lo.disable_ipv6" = 1;
    };

    services.openssh.settings.AllowTcpForwarding = true;

    # Docker
    virtualisation.docker = {
      enable = true;
      daemon.settings.features.buildkit = true;
      enableOnBoot = true;
      autoPrune = {
        enable = true;
        dates = "weekly";
        flags = [ "--all" ];
      };
    };

    # QEMU Bridge Helper
    environment.etc."qemu/bridge.conf" = {
      text = "allow br0";
      mode = "0644";
    };
  };
}
