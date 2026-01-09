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


# Intel SGX DCAP Development Package

{ lib, stdenv, fetchFromGitHub, targetPlatform ? null }:

let
  version = {
    sgxSdk = "2.25";
    dcap = "1.23";
  };

  # Intel SGX SDK headers (sgx_key.h, sgx_quote.h, sgx_report.h, etc.)
  # Required by DCAP headers which depend on SGX base types
  sgxSdkHeaders = stdenv.mkDerivation {
    pname = "intel-sgx-sdk-headers";
    version = version.sgxSdk;

    src = fetchFromGitHub {
      owner = "intel";
      repo = "linux-sgx";
      rev = "sgx_${version.sgxSdk}";
      sha256 = "sha256-xw4S1nw7mij82UotxEocIo9jfYzJE8Cd1xupf1oH1fw=";
    };

    dontConfigure = true;
    dontBuild = true;

    installPhase = ''
      mkdir -p $out/include
      cp -r common/inc/* $out/include/
    '';

    meta = {
      description = "Intel SGX SDK headers";
      homepage = "https://github.com/intel/linux-sgx";
      license = lib.licenses.bsd3;
      platforms = [ "x86_64-linux" ];
    };
  };

  # Intel DCAP headers (sgx_dcap_quoteverify.h, sgx_qve_header.h, sgx_ql_quote.h, etc.)
  # These are the headers needed by intel-tee-quote-verification-sys for bindgen
  dcapHeaders = stdenv.mkDerivation {
    pname = "intel-dcap-headers";
    version = version.dcap;

    src = fetchFromGitHub {
      owner = "intel";
      repo = "SGXDataCenterAttestationPrimitives";
      rev = "DCAP_${version.dcap}";
      sha256 = "sha256-1WlJSQ7/WzsRgphBa2hU7xLd0hLWpuYCtdJerCqkCs4=";
    };

    dontConfigure = true;
    dontBuild = true;

    installPhase = ''
      mkdir -p $out/include

      # Quote Verification headers (main API)
      cp -r QuoteVerification/QvE/Include/* $out/include/
      cp -r QuoteVerification/dcap_quoteverify/inc/* $out/include/

      # Quote Generation headers (types and structures)
      cp -r QuoteGeneration/quote_wrapper/common/inc/* $out/include/ || true
      cp -r QuoteGeneration/pce_wrapper/inc/* $out/include/ || true
      cp -r QuoteGeneration/common/inc/sgx_quote_3.h $out/include/ || true
      cp -r QuoteGeneration/common/inc/sgx_quote_4.h $out/include/ || true
      cp -r QuoteGeneration/common/inc/sgx_quote_5.h $out/include/ || true
      cp -r QuoteGeneration/common/inc/sgx_ql_quote.h $out/include/ || true
    '';

    meta = {
      description = "Intel SGX DCAP headers";
      homepage = "https://github.com/intel/SGXDataCenterAttestationPrimitives";
      license = lib.licenses.bsd3;
      platforms = [ "x86_64-linux" ];
    };
  };

in rec {
  # Combined development package with all headers + stub library
  # Use this for building Rust code that depends on intel-tee-quote-verification-sys
  dev = stdenv.mkDerivation {
    pname = "intel-dcap-dev";
    version = version.dcap;

    dontUnpack = true;
    dontConfigure = true;

    buildPhase = ''
      mkdir -p $out/{include,lib}

      # Merge headers: SGX SDK first, then DCAP (DCAP can override)
      cp -r ${sgxSdkHeaders}/include/* $out/include/
      cp -r ${dcapHeaders}/include/* $out/include/

      # Create stub library with all required symbols for linking
      # The real library is loaded at runtime on TDX hardware
      cat > stub.c << 'EOF'
      /* Stub implementations - real library loaded at runtime on TDX hardware */
      #include <stdint.h>
      #include <stddef.h>

      /* Quote verification functions */
      int sgx_qv_set_enclave_load_policy(int policy) { return 0; }
      int tee_get_supplemental_data_version_and_size(const uint8_t *quote, uint32_t quote_size, uint32_t *version, uint32_t *size) { return 0; }
      int tee_verify_quote(const uint8_t *quote, uint32_t quote_size, void *collateral, uint32_t expiration_check_date, uint32_t *collateral_expiration_status, int *quote_verification_result, void *supplemental_data, uint32_t supplemental_data_size) { return 0; }
      int tee_qv_get_collateral(const uint8_t *quote, uint32_t quote_size, void **collateral) { return 0; }
      int tee_qv_free_collateral(void *collateral) { return 0; }

      /* Additional QVL functions that may be needed */
      int sgx_qv_get_quote_supplemental_data_size(uint32_t *size) { return 0; }
      int sgx_qv_verify_quote(const uint8_t *quote, uint32_t quote_size, void *collateral, uint32_t expiration_check_date, uint32_t *collateral_expiration_status, int *quote_verification_result, void *supplemental_data, uint32_t supplemental_data_size) { return 0; }
      EOF

      # Create both shared and static stub libraries
      $CC -c -fPIC -o stub.o stub.c
      $CC -shared -o $out/lib/libsgx_dcap_quoteverify.so stub.o
      $AR rcs $out/lib/libsgx_dcap_quoteverify.a stub.o
    '';

    installPhase = "true";

    meta = {
      description = "Intel SGX DCAP development headers and stub library for bindgen";
      homepage = "https://github.com/intel/SGXDataCenterAttestationPrimitives";
      license = lib.licenses.bsd3;
      platforms = [ "x86_64-linux" ];
    };
  };

  # Expose components for debugging/advanced use
  inherit sgxSdkHeaders dcapHeaders version;

  # Default output is the dev package
  default = dev;
}
