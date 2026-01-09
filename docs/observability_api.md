# Observability API

Public read-only HTTP endpoints for monitoring peer state and cluster health. Runs on an isolated port (8445) with resource limits to prevent DoS impact on core services.

## Endpoints

### Peers

| Endpoint | Description |
|----------|-------------|
| `GET /peers` | List all known peers |
| `GET /peers/trusted` | List trusted peers only |
| `GET /peers/revoked` | List revoked peers only |
| `GET /peers/stats` | Cache statistics |
| `GET /peers/:measurement_id` | Get peer by measurement ID (hex) |
| `GET /peers/by-ip/:ip` | Get peer by IP address |

### Certificates (Stub)

| Endpoint | Description |
|----------|-------------|
| `GET /certificates` | List all certificates (not yet implemented) |
| `GET /certificates/:namespace` | List namespace certificates (not yet implemented) |

### Health

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Health check |

## Response Format

### Peer Object

```json
{
  "measurement_id": "hex_64_chars",
  "platform_id": "hex_64_chars",
  "ip_address": "192.168.1.100",
  "raft_port": 8444,
  "api_port": 8443,
  "tdx_info": {
    "rtmr0": "hex_96_chars",
    "rtmr1": "hex_96_chars",
    "rtmr2": "hex_96_chars",
    "rtmr3": "hex_96_chars",
    "mr_td": "hex_96_chars",
    "mr_seam": "hex_96_chars",
    "mrsigner_seam": "hex_96_chars",
    "tcb_svn": "hex_32_chars",
    "td_attributes": "hex_16_chars",
    "xfam": "hex_16_chars",
    "vendor_id": "hex_32_chars",
    "quote_version": 4
  },
  "first_seen": 1704067200,
  "last_seen": 1704153600,
  "last_attestation": 1704153600,
  "discovery_source": "DirectConnection",
  "is_trusted": true,
  "revoked_at": null,
  "revocation_reason": null
}
```

### Stats Object

```json
{
  "peers": {
    "total": 5,
    "trusted": 4,
    "revoked": 1
  }
}
```

## Peer Metadata Cache

The peer cache tracks all peers discovered through attestation. Peers are identified by:
- `mr_td` (MRTD): 48-byte SHA-384 of initial TD contents, unique per TD build
- `measurement_id`: `SHA256(RTMR0 || RTMR1 || RTMR2 || RTMR3)`, identifies software stack

**Note**: `measurement_id` is NOT unique per instance. Two TDX VMs with identical images have the same `measurement_id`. Per-instance uniqueness requires REPORTDATA binding with an ephemeral keypair. See [TEE Platform Identification](tee-platform-identification.md).

### Discovery Sources

- `DirectConnection` - Peer discovered via direct attestation
- `Gossip` - Peer discovered via peer gossip protocol
- `Registry` - Peer discovered via measurement registry

### Trust Lifecycle

1. Peer discovered and attestation verified
2. Peer recorded in cache with `is_trusted: true`
3. If attestation fails or peer is revoked:
   - `is_trusted: false`
   - `revoked_at` timestamp set
   - `revocation_reason` populated

### IP Mismatch Detection

If the same `mr_td` (MRTD) appears from different IPs, this may indicate a replay attack. The cache:

1. Logs a warning
2. Blacklists the new IP
3. Revokes trust for the peer

**Note**: Same `measurement_id` from different IPs is expected and legitimate when running multiple instances of the same software. MRTD provides stronger uniqueness but is still deterministic for identical TD builds. True per-instance identity requires REPORTDATA binding.

### Persistence

Cache is persisted to disk hourly at `/var/lib/guardian/peer_cache.bin` and loaded on startup.

## Revalidation Endpoint

The RAFT API exposes a revalidation endpoint for notifying peers when a measurement is revoked:

```
POST /raft/revalidate-peers
Content-Type: application/json

{
  "measurement_ids": ["hex_64_chars", "hex_64_chars"]
}
```

This triggers re-attestation of specified peers. Used by the certificate authority or admin tooling when measurements are revoked in the registry.

## Configuration

| Constant | Value | Description |
|----------|-------|-------------|
| `OBSERVABILITY_API_PORT` | 8445 | API listen port |
| `OBSERVABILITY_API_WORKERS` | 2 | Worker thread limit |
| `PEER_CACHE_PATH` | `/var/lib/guardian/peer_cache.bin` | Cache file path |
| `PEER_CACHE_PERSIST_INTERVAL_SECS` | 3600 | Persistence interval |
