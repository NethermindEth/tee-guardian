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


# Intel TDX Host Module
#
# Configures a NixOS host for Intel TDX (Trust Domain Extensions) confidential computing.
# Includes TDX-enabled kernel, Quote Generation Service (QGS), Multi-Package Registration
# Agent (MPA), and Provisioning Certificate Caching Service (PCCS).
#
# Requirements:
#   - Intel Xeon 4th/5th Gen (Sapphire Rapids / Emerald Rapids)
#   - BIOS: TDX enabled, TME enabled
#   - Intel PCS API key (https://api.portal.trustedservices.intel.com/)

{ config, lib, pkgs, ... }:

let
  inherit (lib)
    mkEnableOption mkOption mkIf mkDefault mkForce
    types optional optionalString optionalAttrs
    makeLibraryPath concatMapStringsSep;

  cfg = config.services.tdx-host;

  # === MPA Configuration ===
  mpaConfigFile = pkgs.writeText "mpa_registration.conf" ''
    ${optionalString (cfg.mpaSubscriptionKey != null) "subscription key = ${cfg.mpaSubscriptionKey}"}

    ${if cfg.mpaProxyType == "manual" then ''
    proxy type  = manual
    proxy url   = ${cfg.mpaProxyUrl}
    '' else if cfg.mpaProxyType == "default" then ''
    proxy type  = default
    '' else ''
    proxy type  = direct
    ''}

    log level = info
  '';

  # === PCCS Configuration ===
  pccsConfigTemplate = {
    HTTPS_PORT = cfg.pccs.port;
    hosts = cfg.pccs.listenAddress;
    uri = "https://api.trustedservices.intel.com/sgx/certification/v4/";
    ApiKey = "";
    proxy = "";
    RefreshSchedule = "0 0 1 * * *";
    UserTokenHash = "";
    AdminTokenHash = "";
    CachingFillMode = "LAZY";
    OPENSSL_FIPS_MODE = false;
    LogLevel = cfg.pccs.logLevel;
    DB_CONFIG = "sqlite";
    sqlite = {
      database = "pckcache";
      username = "pccs";
      password = "";
      options = {
        host = "localhost";
        dialect = "sqlite";
        pool = { max = 5; min = 0; acquire = 30000; idle = 10000; };
        define.freezeTableName = true;
        logging = false;
        storage = "/var/lib/pccs/pckcache.db";
      };
    };
  };

  # === Scripts ===
  pccsPreStartScript = pkgs.writeShellScript "pccs-prestart" ''
    set -euo pipefail

    mkdir -p /var/lib/pccs/{config,logs,ssl_key}
    chown -R pccs:pccs /var/lib/pccs

    # Load API key
    API_KEY=""
    ${optionalString (cfg.pccs.apiKeyFile != null) ''
    if [ -f "${cfg.pccs.apiKeyFile}" ]; then
      API_KEY=$(cat "${cfg.pccs.apiKeyFile}" | tr -d '\n')
    else
      echo "ERROR: API key file not found: ${cfg.pccs.apiKeyFile}"
      exit 1
    fi
    ''}
    ${optionalString (cfg.pccs.apiKeyFile == null && cfg.pccs.apiKey != "") ''
    API_KEY="${cfg.pccs.apiKey}"
    ''}

    if [ -z "$API_KEY" ]; then
      echo "ERROR: Intel PCS API key required (set pccs.apiKeyFile or pccs.apiKey)"
      exit 1
    fi

    # Generate config with API key
    ${pkgs.jq}/bin/jq --arg apikey "$API_KEY" \
      '.ApiKey = $apikey' \
      /etc/sgx-dcap-pccs/default.json.template \
      > /var/lib/pccs/config/default.json

    ln -sf /var/lib/pccs/config/default.json /var/lib/pccs/config/production.json
    chmod 640 /var/lib/pccs/config/default.json
    chown pccs:pccs /var/lib/pccs/config/default.json
    ln -sfn ${pkgs.sgx-dcap-pccs}/lib/node_modules/PCCS/migrations /var/lib/pccs/migrations

    # Generate SSL cert if needed
    if [ ! -f /var/lib/pccs/ssl_key/private.pem ] || [ ! -f /var/lib/pccs/ssl_key/file.crt ]; then
      ${pkgs.openssl}/bin/openssl genrsa -out /var/lib/pccs/ssl_key/private.pem 2048
      ${pkgs.openssl}/bin/openssl req -new \
        -key /var/lib/pccs/ssl_key/private.pem \
        -out /var/lib/pccs/ssl_key/csr.pem \
        -subj "/C=US/ST=State/L=City/O=Intel/CN=localhost"
      ${pkgs.openssl}/bin/openssl x509 -req -days 365 \
        -in /var/lib/pccs/ssl_key/csr.pem \
        -signkey /var/lib/pccs/ssl_key/private.pem \
        -out /var/lib/pccs/ssl_key/file.crt
      chown pccs:pccs /var/lib/pccs/ssl_key/*
      chmod 600 /var/lib/pccs/ssl_key/private.pem
      chmod 644 /var/lib/pccs/ssl_key/file.crt
    fi
  '';

  pccsHealthCheckScript = pkgs.writeShellScript "pccs-healthcheck" ''
    set -euo pipefail
    for i in {1..30}; do
      if ${pkgs.netcat}/bin/nc -z ${cfg.pccs.listenAddress} ${toString cfg.pccs.port} 2>/dev/null; then
        exit 0
      fi
      sleep 1
    done
    echo "PCCS did not start within 30 seconds"
    exit 1
  '';

  qgsPreStartScript = pkgs.writeShellScript "qgs-prestart" ''
    set -euo pipefail
    for dev in sgx_enclave sgx_provision sgx_vepc; do
      if [ ! -c /dev/$dev ]; then
        echo "ERROR: /dev/$dev not found. Is SGX/TDX enabled in BIOS?"
        exit 1
      fi
    done

    if [ ! -d "${pkgs.sgx-enclaves}/lib" ]; then
      echo "ERROR: Intel SGX enclaves not found at ${pkgs.sgx-enclaves}/lib"
      exit 1
    fi
  '';

  mpaPreStartScript = pkgs.writeShellScript "mpa-prestart" ''
    set -euo pipefail
    for dev in sgx_enclave sgx_provision; do
      if [ ! -c /dev/$dev ]; then
        echo "ERROR: /dev/$dev not found. Is SGX/TDX enabled in BIOS?"
        exit 1
      fi
    done
    touch /var/log/mpa_registration.log
    chmod 600 /var/log/mpa_registration.log
  '';

  mpaPostStartScript = pkgs.writeShellScript "mpa-poststart" ''
    sleep 2
    if [ -f /var/log/mpa_registration.log ]; then
      if grep -q "passed successfully" /var/log/mpa_registration.log; then
        echo "Platform registration completed successfully"
      elif grep -q "ERROR" /var/log/mpa_registration.log; then
        echo "Platform registration encountered errors. Check /var/log/mpa_registration.log"
      fi
    fi
  '';

  # === QGS Library Path ===
  qgsLibraryPath = makeLibraryPath [
    pkgs.libsgx-dcap-ql
    pkgs.libsgx-dcap-default-qpl
    pkgs.libsgx-tdx-logic
    pkgs.libsgx-qe3-logic
    pkgs.libsgx-pce-logic
    pkgs.libsgx-urts
    pkgs.libsgx-enclave-common
    pkgs.sgx-enclaves
    pkgs.openssl
    pkgs.protobuf
    pkgs.curl
    pkgs.boost183
    pkgs.systemd
    pkgs.stdenv.cc.cc.lib
  ];

  mpaLibraryPath = makeLibraryPath [
    pkgs.libsgx-ra-uefi
    pkgs.libsgx-ra-network
    pkgs.curl
    pkgs.stdenv.cc.cc.lib
  ];

in {
  options.services.tdx-host = {
    enable = mkEnableOption "Intel TDX confidential computing host";

    # === Kernel Configuration ===
    kernel = {
      version = mkOption {
        type = types.str;
        description = "Linux kernel version (must be 6.8+ for TDX host support)";
      };

      modDirVersion = mkOption {
        type = types.str;
        default = "${cfg.kernel.version}-tdx";
        description = "Module directory version (defaults to kernel version with -tdx suffix)";
      };

      sha256 = mkOption {
        type = types.str;
        description = "SHA256 hash of the kernel source tarball";
      };
    };

    # === QGS Options ===
    vsockPort = mkOption {
      type = types.port;
      default = 4050;
      description = "Vsock port for guest-to-host QGS communication";
    };

    useSecureCert = mkOption {
      type = types.bool;
      default = true;
      description = "Verify PCCS TLS certificate";
    };

    logLevel = mkOption {
      type = types.enum [ "error" "warning" "info" "debug" ];
      default = "info";
      description = "TDX attestation service log level";
    };

    # === MPA Options ===
    enableMpaRegistration = mkOption {
      type = types.bool;
      default = true;
      description = "Enable Intel Multi-Package Registration on boot";
    };

    mpaProxyType = mkOption {
      type = types.enum [ "direct" "default" "manual" ];
      default = "direct";
      description = "MPA proxy configuration";
    };

    mpaProxyUrl = mkOption {
      type = types.str;
      default = "";
      example = "http://proxy.example.com:8080";
      description = "Proxy URL when mpaProxyType is manual";
    };

    mpaSubscriptionKey = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Optional subscription key for Intel registration service";
    };

    # === PCCS Options ===
    pccs = {
      port = mkOption {
        type = types.port;
        default = 8081;
        description = "Port for PCCS HTTPS service";
      };

      listenAddress = mkOption {
        type = types.str;
        default = "127.0.0.1";
        example = "0.0.0.0";
        description = "Listen address for PCCS";
      };

      apiKey = mkOption {
        type = types.str;
        default = "";
        description = "Intel PCS API key (WARNING: stored in Nix store, use apiKeyFile instead)";
      };

      apiKeyFile = mkOption {
        type = types.nullOr types.path;
        default = null;
        example = "/run/secrets/intel-pcs-api-key";
        description = "Path to file containing Intel PCS API key (recommended)";
      };

      logLevel = mkOption {
        type = types.enum [ "error" "warn" "info" "debug" ];
        default = "info";
        description = "PCCS logging level";
      };
    };
  };

  config = mkIf cfg.enable {

    # === Assertions ===
    assertions = [
      {
        assertion = pkgs.stdenv.isx86_64;
        message = "TDX attestation requires x86_64 architecture";
      }
      {
        assertion = cfg.mpaProxyType != "manual" || cfg.mpaProxyUrl != "";
        message = "mpaProxyUrl must be set when mpaProxyType is 'manual'";
      }
      {
        assertion = cfg.pccs.apiKeyFile != null || cfg.pccs.apiKey != "";
        message = "Intel PCS API key required. Set services.tdx-host.pccs.apiKeyFile or pccs.apiKey";
      }
    ];

    warnings =
      optional (!cfg.useSecureCert)
        "TDX attestation: TLS certificate verification disabled (insecure for production)"
      ++ optional (cfg.mpaSubscriptionKey != null)
        "MPA registration: subscriptionKey in plain text. Use secure secrets management."
      ++ optional (cfg.pccs.apiKey != "")
        "TDX: pccs.apiKey is stored in Nix store. Use pccs.apiKeyFile with sops-nix instead.";

    # === TDX Kernel Configuration ===
    boot.kernelPackages = let
      linux_tdx_pkg = { fetchurl, buildLinux, ... } @ args:
        buildLinux (args // rec {
          version = cfg.kernel.version;
          modDirVersion = cfg.kernel.modDirVersion;

          src = pkgs.fetchurl {
            url = "mirror://kernel/linux/kernel/v6.x/linux-${version}.tar.xz";
            sha256 = cfg.kernel.sha256;
          };

          structuredExtraConfig = with lib.kernel; {
            LOCALVERSION = freeform "-tdx";

            # Kexec=n requirement removed in 6.18

            # KEXEC = mkForce no;
            # KEXEC_FILE = mkForce unset;
            # KEXEC_JUMP = mkForce unset;
            # CRASH_DUMP = mkForce unset;
            # PROC_VMCORE = mkForce unset;

            CPU_SUP_INTEL = yes;
            X86_X2APIC = yes;
            X86_MCE = yes;
            KVM = yes;
            KVM_INTEL = yes;
            MEMORY_ISOLATION = yes;
            CMA = yes;
            COMPACTION = yes;

            INTEL_TDX_HOST = yes;
            VFIO_DEVICE_CDEV = yes;

            CRC32_SELFTEST = mkForce unset;
            CRYPTO_TEST = mkForce unset;
            POWER_RESET_GPIO = mkForce unset;
            POWER_RESET_GPIO_RESTART = mkForce unset;
            REISERFS_FS_POSIX_ACL = mkForce unset;
            REISERFS_FS_SECURITY = mkForce unset;
            REISERFS_FS_XATTR = mkForce unset;
            EXT3_FS_POSIX_ACL = mkForce unset;
            EXT3_FS_SECURITY = mkForce unset;
            XEN_SAVE_RESTORE = mkForce unset;
          };
        });

      linux_tdx = pkgs.callPackage linux_tdx_pkg {};
    in
      pkgs.recurseIntoAttrs (pkgs.linuxPackagesFor linux_tdx);

    boot.kernelParams = [
      "kvm_intel.tdx=1"
      "loglevel=7"
      "nohibernate"
      "iommu=pt"
      "intel_iommu=on"
    ];

    powerManagement.cpuFreqGovernor = "performance";
    hardware.cpu.intel.updateMicrocode = mkDefault true;

    # === SGX/TDX Hardware Configuration ===
    hardware.cpu.intel.sgx = {
      provision = {
        enable = true;
        mode = "0660";
        group = "sgx_prv";
      };
      enableDcapCompat = mkDefault true;
    };

    # === Users and Groups ===
    users.groups.sgx = {};
    users.groups.sgx_prv = {};
    users.groups.qgsd = {};
    users.groups.pccs = {};

    users.users.qgsd = {
      isSystemUser = true;
      group = "qgsd";
      description = "Intel TDX Quote Generation Service";
      home = "/var/opt/qgsd";
      createHome = true;
      shell = "${pkgs.shadow}/sbin/nologin";
      extraGroups = [ "sgx_prv" "sgx" ];
    };

    users.users.pccs = {
      isSystemUser = true;
      group = "pccs";
      description = "SGX DCAP PCCS service user";
      home = "/var/lib/pccs";
    };

    # === Udev Rules ===
    services.udev.extraRules = ''
      SUBSYSTEM=="misc", KERNEL=="sgx_enclave", MODE="0660", GROUP="sgx", TAG+="systemd"
      SUBSYSTEM=="misc", KERNEL=="sgx_provision", MODE="0660", GROUP="sgx_prv", TAG+="systemd"
      SUBSYSTEM=="misc", KERNEL=="sgx_vepc", MODE="0660", GROUP="sgx", TAG+="systemd"
      SUBSYSTEM=="misc", KERNEL=="sgx_*", TAG+="systemd", ENV{SYSTEMD_WANTS}+="qgsd.service"
    '';

    # === Directories and Config Files ===
    systemd.tmpfiles.rules = [ "d /var/opt/qgsd 0755 qgsd qgsd -" ];

    environment.etc."qgs.conf".text = ''
      port = ${toString cfg.vsockPort}
      number_threads = 4
    '';

    environment.etc."sgx_default_qcnl.conf".text = builtins.toJSON {
      pccs_url = "https://localhost:${toString cfg.pccs.port}/sgx/certification/v4/";
      use_secure_cert = cfg.useSecureCert;
      retry_times = 6;
      retry_delay = 10;
      local_pck_url_cache_expire_hours = 24;
      collateral_cache_expire_hours = 168;
      verify_collateral_cache_expire_hours = 168;
      custom_request_options = {
        get_cert = { headers = {}; };
        get_collateral = { headers = {}; };
      };
    };

    environment.etc."sgx-dcap-pccs/default.json.template" = {
      mode = "0644";
      text = builtins.toJSON pccsConfigTemplate;
    };

    # TDX-specific environment variables
    environment.variables = {
      TDX_QGS_SOCKET = "/var/run/tdx-qgs/qgs.socket";
    };

    environment.systemPackages = [
      pkgs.libsgx-dcap-ql
      pkgs.libsgx-dcap-default-qpl
      pkgs.sgx-enclaves
      pkgs.tdx-qgs-daemon
      pkgs.sgx-dcap-pccs
      pkgs.sgx-pck-id-retrieval-tool
      pkgs.msr-tools
    ] ++ optional cfg.enableMpaRegistration pkgs.sgx-ra-service;

    # === PCCS Service ===
    systemd.services.pccs = {
      description = "Intel SGX DCAP Provisioning Certificate Caching Service";
      documentation = [ "https://github.com/intel/SGXDataCenterAttestationPrimitives" ];
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        Type = "simple";
        User = "pccs";
        Group = "pccs";
        StateDirectory = "pccs";
        WorkingDirectory = "/var/lib/pccs";
        ExecStartPre = "+${pccsPreStartScript}";
        ExecStart = "${pkgs.sgx-dcap-pccs}/bin/pccs";
        ExecStartPost = pccsHealthCheckScript;
        Environment = [
          "NODE_ENV=production"
          "PCCS_LOG_DIR=/var/lib/pccs/logs"
          "NODE_CONFIG_DIR=/var/lib/pccs/config"
          "NODE_PATH=${pkgs.sgx-dcap-pccs}/lib/node_modules"
        ];
        Restart = "on-failure";
        RestartSec = "5s";
        PrivateTmp = true;
        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "pccs";
      };
    };

    networking.firewall.allowedTCPPorts =
      mkIf (cfg.pccs.listenAddress != "127.0.0.1" && cfg.pccs.listenAddress != "localhost")
        [ cfg.pccs.port ];

    # === QGS Service ===
    systemd.services.qgsd = {
      description = "Intel SGX DCAP Quote Generation Service";
      documentation = [ "https://github.com/intel/SGXDataCenterAttestationPrimitives" ];
      after = [ "network.target" "pccs.service" ];
      wants = [ "pccs.service" ];
      wantedBy = [ "multi-user.target" ];

      unitConfig.ConditionPathExists = [
        "/dev/sgx_enclave"
        "/dev/sgx_provision"
        "/dev/sgx_vepc"
      ];

      restartIfChanged = true;

      environment = {
        LD_LIBRARY_PATH = qgsLibraryPath;
        QGS_LOG_LEVEL = cfg.logLevel;
      };

      serviceConfig = {
        Type = "forking";
        ExecStartPre = qgsPreStartScript;
        ExecStart = "${pkgs.tdx-qgs}/bin/qgs";
        Restart = "on-failure";
        RestartSec = "5s";
        WorkingDirectory = "/var/opt/qgsd";
        User = "qgsd";
        Group = "qgsd";
        SupplementaryGroups = [ "sgx_prv" "sgx" ];
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/opt/qgsd" ];
        ReadOnlyPaths = [ "/etc/qgs.conf" "/etc/sgx_default_qcnl.conf" ];
        LimitNOFILE = 65536;
        LimitNPROC = 512;
        LimitCORE = 0;
        DeviceAllow = [
          "/dev/sgx_enclave rw"
          "/dev/sgx_provision rw"
          "/dev/sgx_vepc rw"
        ];
        AmbientCapabilities = [];
        CapabilityBoundingSet = [];
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" "AF_VSOCK" ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "qgsd";
      };
    };

    # === MPA Registration Service ===
    environment.etc."mpa_registration.conf" = mkIf cfg.enableMpaRegistration {
      source = mpaConfigFile;
      mode = "0600";
    };

    systemd.services.mpa-registration = mkIf cfg.enableMpaRegistration {
      description = "Intel Multi-Package Registration Agent (MPA)";
      documentation = [ "https://github.com/intel/SGXDataCenterAttestationPrimitives" ];
      wants = [ "network-online.target" ];
      after = [ "network.target" "auditd.service" "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      unitConfig.ConditionPathExists = [
        "/dev/sgx_enclave"
        "/dev/sgx_provision"
      ];

      restartIfChanged = true;

      environment.LD_LIBRARY_PATH = mpaLibraryPath;

      serviceConfig = {
        Type = "oneshot";
        ExecStartPre = mpaPreStartScript;
        ExecStart = "${pkgs.sgx-ra-service}/bin/mpa_registration";
        ExecStartPost = mpaPostStartScript;
        RemainAfterExit = false;
        StandardOutput = "journal";
        StandardError = "journal";
        SyslogIdentifier = "mpa-registration";
        LogsDirectory = "mpa-registration";
        StateDirectory = "mpa-registration";
        User = "root";
        Group = "root";
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/log" ];
        ReadOnlyPaths = [ "/etc/mpa_registration.conf" ];
        SupplementaryGroups = [ "sgx_prv" "sgx" ];
        DeviceAllow = [
          "/dev/sgx_enclave rw"
          "/dev/sgx_provision rw"
        ];
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" ];
        ProtectKernelTunables = false;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        NoNewPrivileges = false;
        TimeoutStartSec = "5min";
      };
    };

    services.logrotate.settings.mpa-registration = mkIf cfg.enableMpaRegistration {
      files = [ "/var/log/mpa_registration.log" ];
      frequency = "weekly";
      rotate = 4;
      compress = true;
      missingok = true;
      notifempty = true;
      su = "root root";
    };
  };
}
