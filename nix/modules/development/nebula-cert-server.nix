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


# Nebula Certificate Server Module
#
# Minimal HTTP server that serves pre-generated Nebula certificates to VMs.
# VMs request their cert by MAC address, server returns cert+key as JSON.
#
# Endpoint: GET /cert?mac=52:54:00:12:34:XX
# Response: {"cert": "<base64>", "key": "<base64>", "ca": "<base64>"}

{ config, lib, pkgs, ... }:

let
  inherit (lib) mkEnableOption mkOption mkIf types;

  cfg = config.services.nebula-cert-server;

  luaScript = ''
    local certs_dir = "${cfg.certsDir}"
    local ca_file = "${cfg.caFile}"

    local function read_file(path)
      local f = io.open(path, "r")
      if not f then return nil end
      local content = f:read("*a")
      f:close()
      return content
    end

    local function serve_cert()
      local mac = ngx.var.arg_mac
      if not mac then
        ngx.status = 400
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "missing mac parameter"}')
        return
      end

      local last_octet = mac:match(":(%x%x)$")
      if not last_octet then
        ngx.status = 400
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "invalid mac format, expected XX:XX:XX:XX:XX:XX"}')
        return
      end

      local slot = tonumber(last_octet, 16)
      if not slot or slot < 10 or slot > 109 then
        ngx.status = 400
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "slot out of range (10-109), got: ' .. tostring(slot) .. '"}')
        return
      end

      local cert_path = string.format("%s/slot-%d.crt", certs_dir, slot)
      local key_path = string.format("%s/slot-%d.key", certs_dir, slot)

      local cert = read_file(cert_path)
      local key = read_file(key_path)
      local ca = read_file(ca_file)

      if not cert or not key then
        ngx.status = 404
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "cert not found for slot ' .. slot .. '"}')
        return
      end

      if not ca then
        ngx.status = 500
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "CA certificate not found"}')
        return
      end

      ngx.header.content_type = "application/json"
      ngx.say(string.format('{"cert": "%s", "key": "%s", "ca": "%s"}',
        ngx.encode_base64(cert),
        ngx.encode_base64(key),
        ngx.encode_base64(ca)))
    end

    serve_cert()
  '';

  luaFile = pkgs.writeText "nebula-cert-server.lua" luaScript;

  nginxConfig = ''
    worker_processes 1;
    error_log stderr info;
    pid /run/nebula-cert-server/nginx.pid;

    events {
      worker_connections 64;
    }

    http {
      access_log off;
      client_body_temp_path /run/nebula-cert-server/client_body;
      proxy_temp_path /run/nebula-cert-server/proxy;
      fastcgi_temp_path /run/nebula-cert-server/fastcgi;
      uwsgi_temp_path /run/nebula-cert-server/uwsgi;
      scgi_temp_path /run/nebula-cert-server/scgi;

      server {
        listen ${cfg.listenAddress}:${toString cfg.port};

        location /cert {
          default_type application/json;
          content_by_lua_file ${luaFile};
        }

        location /health {
          return 200 '{"status": "ok"}';
        }
      }
    }
  '';

  nginxConfigFile = pkgs.writeText "nebula-cert-server-nginx.conf" nginxConfig;

in {
  options.services.nebula-cert-server = {
    enable = mkEnableOption "Nebula certificate server for VMs";

    listenAddress = mkOption {
      type = types.str;
      default = "192.168.122.1";
      description = "IP address to listen on (should be VM bridge IP)";
    };

    port = mkOption {
      type = types.port;
      default = 9999;
      description = "Port to listen on";
    };

    certsDir = mkOption {
      type = types.path;
      default = "/var/lib/nebula/vm-certs";
      description = "Directory containing VM certificates";
    };

    caFile = mkOption {
      type = types.path;
      default = "/var/lib/nebula/ca/ca.crt";
      description = "Path to Nebula CA certificate";
    };
  };

  config = mkIf cfg.enable {
    systemd.services.nebula-cert-server = {
      description = "Nebula certificate server for VMs";
      after = [ "network.target" "nebula-cert-gen.service" ];
      wants = [ "nebula-cert-gen.service" ];
      wantedBy = [ "multi-user.target" ];

      preStart = ''
        mkdir -p /run/nebula-cert-server/{client_body,proxy,fastcgi,uwsgi,scgi}
      '';

      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.openresty}/bin/openresty -c ${nginxConfigFile} -g 'daemon off;'";
        Restart = "always";
        RestartSec = "2s";
        RuntimeDirectory = "nebula-cert-server";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        ReadOnlyPaths = [ cfg.certsDir cfg.caFile ];
      };
    };
  };
}
