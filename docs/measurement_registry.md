# Measurement Registry

## Overview

The Measurement Registry provides two critical functions for Guardian cluster operation:

1. **Measurement Verification**: Validate that a node's TEE measurements are trusted
2. **Node Discovery**: Provide initial bootstrap IP addresses for cluster formation

The registry is read-only from Guardian nodes. Measurement approval and node registration are administrative operations.

For the production deployment, the registry will be a public Ethereum contract with multi-sig for adding/revoking measurements.
There will be separate backends for reading trusted measurements from Ethereum contract over RPC, and the HTTP registry used for development/testing.

## Data Model

### Measurement Response Schema

```json
{
  "namespace": "string",
  "platform": "tdx | sev_snp",
  "data": { /* platform-specific, see below */ },
  "revoked_at": null | 1234567890 // UNIX timestamp
}
```

The `measurement_hash` (used as the lookup key) is computed as:
- **TDX**: `SHA-256(RTMR0 || RTMR1 || RTMR2 || RTMR3)` - identifies software stack
- **SEV-SNP**: `SHA-256(measurement || host_data)` or `SHA-256(measurement)` if no host_data


### TDX Measurement

```json
{
  "namespace": "guardian",
  "platform": "tdx",
  "data": {
    "rtmr0": "hex_96_chars",
    "rtmr1": "hex_96_chars",
    "rtmr2": "hex_96_chars",
    "rtmr3": "hex_96_chars",
    "debug_allowed": false,
    "minimum_tcb": {
      "seam_svn": 3,
      "tee_tcb_svn": "hex_32_chars"
    },
    "platform_constraints": {
      "allowed_vendor_ids": ["hex_32_chars"],
      "mr_seam": "hex_96_chars",
      "mrsigner_seam": "hex_96_chars",
      "allowed_cpu_models": [
        {"family": 6, "model": 143, "stepping": null}
      ]
    }
  }
}
```

| Field | Description |
|-------|-------------|
| `rtmr0-3` | 48-byte Runtime Measurement Registers extended via `SHA384(RTMR[i] \|\| data)` |
| `debug_allowed` | Allow debug TDs (default: `false`) |
| `minimum_tcb.seam_svn` | **[Optional]** Minimum TDX module SVN |
| `minimum_tcb.tee_tcb_svn` | **[Optional]** Minimum 16-byte TEE TCB SVN array |
| `platform_constraints.allowed_vendor_ids` | **[Optional]** Restrict to specific vendor GUIDs |
| `platform_constraints.mr_seam` | **[Optional]** Require specific TDX module measurement |
| `platform_constraints.mrsigner_seam` | **[Optional]** Require specific TDX module signer (Intel) |
| `platform_constraints.allowed_cpu_models` | **[Optional]** Restrict to specific CPU family/model/stepping |

### SEV-SNP Measurement

```json
{
  "sev_snp": {
    "measurement": "hex_96_chars",
    "host_data": "hex_64_chars",
    "debug_allowed": false,
    "minimum_tcb": {
      "bootloader_svn": 3,
      "tee_svn": 0,
      "snp_svn": 11,
      "microcode_svn": 209
    },
    "platform_constraints": {
      "allowed_chip_ids": ["hex_128_chars"],
      "allowed_cpu_models": [
        {"family": 25, "model": 1, "stepping": null}
      ],
      "require_smt_disabled": true,
      "required_vmpl": 0,
      "id_key_digest": "hex_96_chars",
      "author_key_digest": "hex_96_chars"
    }
  }
}
```

| Field | Description |
|-------|-------------|
| `measurement` | 48-byte SHA-384 launch digest computed during SNP_LAUNCH_UPDATE |
| `host_data` | 32-byte hypervisor-provided data (e.g., container image digest, policy hash) |
| `debug_allowed` | **[Optional]** Allow debug guests (default: `false`) |
| `minimum_tcb.*_svn` | **[Optional]** Minimum SVN for bootloader, TEE, SNP firmware, microcode |
| `platform_constraints.allowed_chip_ids` | **[Optional]** Restrict to specific physical CPUs (64-byte fused IDs) |
| `platform_constraints.allowed_cpu_models` | **[Optional]** Restrict to specific AMD CPU family/model/stepping |
| `platform_constraints.require_smt_disabled` | **[Optional]** Require hyperthreading off (side-channel mitigation) |
| `platform_constraints.required_vmpl` | **[Optional]** Required VM privilege level (0=most privileged) |
| `platform_constraints.id_key_digest` | **[Optional]** Required ID signing key digest (SHA-384) |
| `platform_constraints.author_key_digest` | **[Optional]** Required author signing key digest (SHA-384) |

### Common Types

**CpuModel** - Used by both platforms:
```json
{"family": 6, "model": 143, "stepping": null}
```
- `stepping: null` matches any stepping

**Optional fields**: All `minimum_tcb` and `platform_constraints` fields are optional. Omitted fields are not enforced.

## HTTP API

### Get Measurement

```
GET /measurements/{namespace}/{measurement_hash}

Response (200): Measurement object (see schema above)
Response (404): {"error": "Measurement not found"}
```

### Get Bootstrap Nodes

```
GET /nodes?namespace={namespace}

Response (200):
{
  "nodes": [
    {"ip_address": "192.168.100.10", "raft_port": 8444, "api_port": 8443}
  ]
}
```

## Verification Logic

Verification checks are performed in order. All checks must pass.

### Common Checks (Both Platforms)

1. **Measurement match**: RTMRs (TDX) or launch measurement (SEV-SNP) must match exactly
2. **Debug check**: If attestation has debug enabled and `debug_allowed: false`, reject
3. **TCB check**: Each specified `minimum_tcb` component must be `<=` the attestation's value
4. **CPU model check**: If `allowed_cpu_models` specified, attestation must match one entry

### Platform-Specific Checks

**TDX only:**
- `allowed_vendor_ids`: Attestation vendor_id must be in list
- `mr_seam` / `mrsigner_seam`: Must match exactly if specified

**SEV-SNP only:**
- `allowed_chip_ids`: Attestation chip_id must be in list (skipped if chip_id is masked/zeros)
- `require_smt_disabled`: Platform must have SMT off
- `required_vmpl`: Attestation VMPL must match
- `id_key_digest` / `author_key_digest`: Must match exactly if specified
- `host_data`: Must match if specified in measurement

### Verification Errors

| Error | Cause |
|-------|-------|
| `MeasurementMismatch` | RTMRs or launch measurement don't match |
| `DebugModeNotAllowed` | Debug enabled but `debug_allowed: false` |
| `TcbTooOld` | TCB component below minimum (includes component name) |
| `CpuModelNotAllowed` | CPU family/model/stepping not in allowed list |
| `VendorNotAllowed` | Vendor ID not in allowed list (TDX) |
| `SmtMustBeDisabled` | SMT enabled but `require_smt_disabled: true` (SEV-SNP) |
| `VmplMismatch` | VMPL doesn't match required value (SEV-SNP) |

## Administrative Operations

### Register Measurement

```bash
curl -X POST https://registry/measurements/{namespace} \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{ "platform": "tdx", "data": { "tdx": { ... } } }'
```

### Revoke Measurement

```bash
curl -X DELETE https://registry/measurements/{namespace}/{measurement_hash} \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

Nodes query the registry every 5 minutes. Revoked measurements are detected within 5 minutes.

### Add Bootstrap Node

```bash
curl -X POST https://registry/nodes \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{"namespace": "guardian", "ip_address": "192.168.100.10", "raft_port": 8444, "api_port": 8443}'
```

## Security Model

**Measurement Verification**: Nodes must have their measurement hash in the registry to join. Registry is controlled by admin multi-sig.

**Node Discovery**: Bootstrap IPs are unverified initial contacts. Nodes verify each other via attestation during RAFT join. Malicious bootstrap nodes can only DoS discovery, not compromise the cluster.

**Error Handling**:
- Registry unavailable for measurement verification: Node cannot start (fail closed)
- Registry unavailable for node discovery: Use cached IPs + PEX (degrade gracefully)

