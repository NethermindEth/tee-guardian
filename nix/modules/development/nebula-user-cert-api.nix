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


# Nebula User Certificate API
#
# HTTP API for dynamic developer certificate allocation.
# Developers request certs via SSH: nebula-user-cert allocate '<identity>'
#
# Features:
#   - Dynamic slot allocation (1-64)
#   - 24-hour certificate TTL
#   - Identity tracking in registry
#   - Auto-cleanup of expired allocations
#
# Endpoints:
#   POST /allocate - Allocate a new cert for identity
#   GET /status    - List current allocations
#
# CLI Tool (nebula-user-cert):
#   nebula-user-cert allocate "user@hostname"
#   nebula-user-cert status
#   nebula-user-cert cleanup

{ config, lib, pkgs, ... }:

let
  inherit (lib) mkEnableOption mkOption mkIf types optionalString;

  cfg = config.services.nebula-user-cert-api;
  nebulaCfg = config.services.guardian-nebula;

  registryFile = "/var/lib/nebula/dev-registry.json";
  devCertsDir = "/var/lib/nebula/dev-certs";
  certTtlHours = 24;

  # Lua script for the HTTP API
  luaScript = ''
    local cjson = require("cjson")
    local nebula_bin = "${pkgs.nebula}/bin/nebula-cert"
    local ca_crt = "${cfg.caFile}"
    local ca_key = "${cfg.caKeyFile}"
    local certs_dir = "${devCertsDir}"
    local registry_file = "${registryFile}"
    local ttl_hours = ${toString certTtlHours}
    local lighthouse = "${cfg.lighthouseAddress}"

    local function read_file(path)
      local f = io.open(path, "r")
      if not f then return nil end
      local content = f:read("*a")
      f:close()
      return content
    end

    local function write_file(path, content)
      local f = io.open(path, "w")
      if not f then return false end
      f:write(content)
      f:close()
      return true
    end

    local function load_registry()
      local content = read_file(registry_file)
      if not content or content == "" then
        return { allocations = {} }
      end
      local ok, data = pcall(cjson.decode, content)
      if not ok then
        return { allocations = {} }
      end
      return data
    end

    local function save_registry(registry)
      write_file(registry_file, cjson.encode(registry))
    end

    local function current_time()
      return os.time()
    end

    local function is_expired(allocation)
      local expires_at = allocation.expires_at or 0
      return current_time() > expires_at
    end

    local function cleanup_expired(registry)
      local cleaned = { allocations = {} }
      for slot, alloc in pairs(registry.allocations) do
        if not is_expired(alloc) then
          cleaned.allocations[slot] = alloc
        else
          -- Delete expired cert files
          os.remove(certs_dir .. "/slot-" .. slot .. ".crt")
          os.remove(certs_dir .. "/slot-" .. slot .. ".key")
        end
      end
      return cleaned
    end

    local function find_available_slot(registry)
      for slot = 1, 64 do
        local key = tostring(slot)
        if not registry.allocations[key] then
          return slot
        end
      end
      return nil
    end

    local function generate_cert(slot, identity)
      local ip = "10.42.0." .. slot .. "/24"
      local crt_path = certs_dir .. "/slot-" .. slot .. ".crt"
      local key_path = certs_dir .. "/slot-" .. slot .. ".key"
      local duration = tostring(ttl_hours) .. "h"

      -- Ensure certs directory exists
      os.execute("mkdir -p " .. certs_dir)

      local cmd = string.format(
        "%s sign -ca-crt %s -ca-key %s -name '%s' -ip '%s' -groups 'developers' -duration '%s' -out-crt '%s' -out-key '%s' 2>&1",
        nebula_bin, ca_crt, ca_key, identity, ip, duration, crt_path, key_path
      )

      local handle = io.popen(cmd)
      local result = handle:read("*a")
      local success = handle:close()

      if not success then
        return nil, "cert generation failed: " .. result
      end

      os.execute("chmod 644 " .. crt_path)
      os.execute("chmod 600 " .. key_path)

      return {
        cert = read_file(crt_path),
        key = read_file(key_path),
      }
    end

    local function handle_allocate()
      ngx.req.read_body()
      local body = ngx.req.get_body_data()

      if not body then
        ngx.status = 400
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "missing request body"}')
        return
      end

      local ok, req = pcall(cjson.decode, body)
      if not ok or not req.identity then
        ngx.status = 400
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "invalid request, expected {identity: string}"}')
        return
      end

      local identity = req.identity

      -- Load and cleanup registry
      local registry = load_registry()
      registry = cleanup_expired(registry)

      -- Check if identity already has an allocation
      for slot, alloc in pairs(registry.allocations) do
        if alloc.identity == identity and not is_expired(alloc) then
          -- Return existing allocation
          local crt_path = certs_dir .. "/slot-" .. slot .. ".crt"
          local key_path = certs_dir .. "/slot-" .. slot .. ".key"
          local ca = read_file(ca_crt)
          local cert = read_file(crt_path)
          local key = read_file(key_path)

          if cert and key and ca then
            ngx.header.content_type = "application/json"
            ngx.say(cjson.encode({
              ip = "10.42.0." .. slot,
              cert = ngx.encode_base64(cert),
              key = ngx.encode_base64(key),
              ca = ngx.encode_base64(ca),
              lighthouse = lighthouse,
              expires_at = os.date("!%Y-%m-%dT%H:%M:%SZ", alloc.expires_at),
            }))
            return
          end
        end
      end

      -- Find available slot
      local slot = find_available_slot(registry)
      if not slot then
        ngx.status = 503
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "no slots available (all 64 in use)"}')
        return
      end

      -- Generate certificate
      local cert_data, err = generate_cert(slot, identity)
      if not cert_data then
        ngx.status = 500
        ngx.header.content_type = "application/json"
        ngx.say(cjson.encode({ error = err }))
        return
      end

      local ca = read_file(ca_crt)
      if not ca then
        ngx.status = 500
        ngx.header.content_type = "application/json"
        ngx.say('{"error": "CA certificate not found"}')
        return
      end

      -- Record allocation
      local expires_at = current_time() + (ttl_hours * 3600)
      registry.allocations[tostring(slot)] = {
        identity = identity,
        allocated_at = current_time(),
        expires_at = expires_at,
      }
      save_registry(registry)

      ngx.header.content_type = "application/json"
      ngx.say(cjson.encode({
        ip = "10.42.0." .. slot,
        cert = ngx.encode_base64(cert_data.cert),
        key = ngx.encode_base64(cert_data.key),
        ca = ngx.encode_base64(ca),
        lighthouse = lighthouse,
        expires_at = os.date("!%Y-%m-%dT%H:%M:%SZ", expires_at),
      }))
    end

    local function handle_status()
      local registry = load_registry()
      registry = cleanup_expired(registry)
      save_registry(registry)

      local allocations = {}
      for slot, alloc in pairs(registry.allocations) do
        table.insert(allocations, {
          slot = tonumber(slot),
          ip = "10.42.0." .. slot,
          identity = alloc.identity,
          allocated_at = os.date("!%Y-%m-%dT%H:%M:%SZ", alloc.allocated_at),
          expires_at = os.date("!%Y-%m-%dT%H:%M:%SZ", alloc.expires_at),
        })
      end

      -- Sort by slot
      table.sort(allocations, function(a, b) return a.slot < b.slot end)

      ngx.header.content_type = "application/json"
      ngx.say(cjson.encode({ allocations = allocations }))
    end

    -- Router
    local method = ngx.req.get_method()
    local uri = ngx.var.uri

    if uri == "/allocate" and method == "POST" then
      handle_allocate()
    elseif uri == "/status" and method == "GET" then
      handle_status()
    else
      ngx.status = 404
      ngx.header.content_type = "application/json"
      ngx.say('{"error": "not found"}')
    end
  '';

  luaFile = pkgs.writeText "nebula-user-cert-api.lua" luaScript;

  nginxConfig = ''
    worker_processes 1;
    error_log stderr info;
    pid /run/nebula-user-cert-api/nginx.pid;

    events {
      worker_connections 64;
    }

    http {
      access_log off;
      client_body_temp_path /run/nebula-user-cert-api/client_body;
      proxy_temp_path /run/nebula-user-cert-api/proxy;
      fastcgi_temp_path /run/nebula-user-cert-api/fastcgi;
      uwsgi_temp_path /run/nebula-user-cert-api/uwsgi;
      scgi_temp_path /run/nebula-user-cert-api/scgi;

      server {
        listen ${cfg.listenAddress}:${toString cfg.port};

        location / {
          default_type application/json;
          content_by_lua_file ${luaFile};
        }

        location /health {
          return 200 '{"status": "ok"}';
        }
      }
    }
  '';

  nginxConfigFile = pkgs.writeText "nebula-user-cert-api-nginx.conf" nginxConfig;

  # CLI tool for SSH-based access
  cliScript = pkgs.writeShellScriptBin "nebula-user-cert" ''
    set -euo pipefail

    API_URL="http://${cfg.listenAddress}:${toString cfg.port}"

    case "''${1:-}" in
      allocate)
        IDENTITY="''${2:-}"
        if [ -z "$IDENTITY" ]; then
          echo '{"error": "usage: nebula-user-cert allocate <identity>"}'
          exit 1
        fi
        curl -s -X POST "$API_URL/allocate" \
          -H "Content-Type: application/json" \
          -d "{\"identity\": \"$IDENTITY\"}"
        ;;
      status)
        curl -s "$API_URL/status"
        ;;
      cleanup)
        # Trigger cleanup by fetching status (cleanup happens on read)
        curl -s "$API_URL/status" > /dev/null
        echo '{"status": "cleanup triggered"}'
        ;;
      *)
        echo "Usage: nebula-user-cert <allocate|status|cleanup> [args]"
        echo ""
        echo "Commands:"
        echo "  allocate <identity>  - Allocate a certificate for identity"
        echo "  status               - Show current allocations"
        echo "  cleanup              - Remove expired allocations"
        exit 1
        ;;
    esac
  '';

in {
  options.services.nebula-user-cert-api = {
    enable = mkEnableOption "Nebula user certificate API for developers";

    listenAddress = mkOption {
      type = types.str;
      default = "127.0.0.1";
      description = "IP address to listen on";
    };

    port = mkOption {
      type = types.port;
      default = 8443;
      description = "Port to listen on";
    };

    caFile = mkOption {
      type = types.path;
      default = "/var/lib/nebula/ca/ca.crt";
      description = "Path to Nebula CA certificate";
    };

    caKeyFile = mkOption {
      type = types.path;
      default = "/var/lib/nebula/ca/ca.key";
      description = "Path to Nebula CA private key (required for signing)";
    };

    lighthouseAddress = mkOption {
      type = types.str;
      description = "Public address of lighthouse (host:port) to include in cert response";
      example = "51.210.181.125:4242";
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ cliScript pkgs.curl ];

    systemd.tmpfiles.rules = [
      "d /var/lib/nebula/dev-certs 0700 root root -"
      "f ${registryFile} 0600 root root -"
    ];

    systemd.services.nebula-user-cert-api = {
      description = "Nebula user certificate API";
      after = [ "network.target" "nebula-cert-gen.service" ];
      wants = [ "nebula-cert-gen.service" ];
      wantedBy = [ "multi-user.target" ];

      preStart = ''
        mkdir -p /run/nebula-user-cert-api/{client_body,proxy,fastcgi,uwsgi,scgi}
        # Ensure registry file exists
        [ -f ${registryFile} ] || echo '{"allocations":{}}' > ${registryFile}
      '';

      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.openresty}/bin/openresty -c ${nginxConfigFile} -g 'daemon off;'";
        Restart = "always";
        RestartSec = "2s";
        RuntimeDirectory = "nebula-user-cert-api";
      };
    };

    # Cleanup timer - run every hour to remove expired allocations
    systemd.timers.nebula-user-cert-cleanup = {
      description = "Cleanup expired Nebula user certificates";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnCalendar = "hourly";
        Persistent = true;
      };
    };

    systemd.services.nebula-user-cert-cleanup = {
      description = "Cleanup expired Nebula user certificates";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${cliScript}/bin/nebula-user-cert cleanup";
      };
    };
  };
}
