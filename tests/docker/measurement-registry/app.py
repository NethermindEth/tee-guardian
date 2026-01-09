"""
Guardian Measurement Registry - HTTP Implementation

Provides two core functions:
1. Measurement Verification - validate TDX measurements against trusted list
2. Node Discovery - provide bootstrap IPs for cluster formation

All configuration is done dynamically via the /config API endpoint.
No file-based configuration is used.

Spec: docs/measurement_registry.md
"""

import logging
import os
import threading
from contextlib import asynccontextmanager
from hashlib import sha256

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)


# --- Data Models ---


class TrustedMeasurement(BaseModel):
    """Trusted measurement entry"""

    namespace: str
    rtmr0: str  # 48 bytes hex (96 chars)
    rtmr1: str
    rtmr2: str
    rtmr3: str
    revoked: bool


class BootstrapNode(BaseModel):
    """Bootstrap node endpoint"""

    ip_address: str
    raft_port: int
    api_port: int


class NodesResponse(BaseModel):
    """Response for GET /nodes"""

    nodes: list[BootstrapNode]


class MeasurementEntry(BaseModel):
    """Measurement entry for POST /config"""

    rtmr0: str
    rtmr1: str
    rtmr2: str
    rtmr3: str
    revoked: bool = False
    description: str = ""


class NamespaceConfig(BaseModel):
    """Configuration for a namespace via POST /config"""

    namespace: str
    measurements: list[MeasurementEntry]
    nodes: list[BootstrapNode]


# --- In-Memory Storage ---

# Structure: {namespace: {measurement_hash: TrustedMeasurement}}
measurements: dict[str, dict[str, TrustedMeasurement]] = {}

# Structure: {namespace: [BootstrapNode]}
bootstrap_nodes: dict[str, list[BootstrapNode]] = {}

# Thread lock for safe concurrent access
registry_lock = threading.Lock()


# --- Helper Functions ---


def compute_measurement_hash(rtmr0: str, rtmr1: str, rtmr2: str, rtmr3: str) -> str:
    """
    Compute measurement hash from RTMRs.

    measurement_hash = SHA-256(RTMR0 || RTMR1 || RTMR2 || RTMR3)

    Returns: 32 byte hash as hex string (64 chars)
    """
    hasher = sha256()
    hasher.update(bytes.fromhex(rtmr0))
    hasher.update(bytes.fromhex(rtmr1))
    hasher.update(bytes.fromhex(rtmr2))
    hasher.update(bytes.fromhex(rtmr3))
    return hasher.hexdigest()


# --- Application Lifespan ---


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Application lifespan handler."""
    cluster_id = os.environ.get("CLUSTER_ID", "unknown")
    logger.info(f"Measurement Registry starting (cluster_id={cluster_id})")
    yield
    logger.info("Measurement Registry shutting down")


app = FastAPI(
    title="Guardian Measurement Registry",
    description="Trusted TDX measurement verification and node discovery",
    version="2.0.0",
    lifespan=lifespan,
)


# --- API Endpoints ---


@app.get("/health")
async def health():
    """Health check endpoint."""
    with registry_lock:
        return {
            "status": "healthy",
            "cluster_id": os.environ.get("CLUSTER_ID", "unknown"),
            "namespaces": list(measurements.keys()),
            "total_measurements": sum(len(ns) for ns in measurements.values()),
            "total_nodes": sum(len(ns) for ns in bootstrap_nodes.values()),
        }


@app.get("/measurements/{namespace}/{measurement_hash}")
async def get_measurement(namespace: str, measurement_hash: str):
    """
    Get trusted measurement.

    GET /measurements/{namespace}/{measurement_hash}

    Returns:
      200: Measurement found
      404: Measurement not found
    """
    with registry_lock:
        if namespace not in measurements:
            raise HTTPException(status_code=404, detail="Namespace not found")

        measurement = measurements[namespace].get(measurement_hash)
        if not measurement:
            raise HTTPException(status_code=404, detail="Measurement not found")

        return {
            "namespace": measurement.namespace,
            "rtmr0": measurement.rtmr0,
            "rtmr1": measurement.rtmr1,
            "rtmr2": measurement.rtmr2,
            "rtmr3": measurement.rtmr3,
            "revoked": measurement.revoked,
        }


@app.get("/nodes", response_model=NodesResponse)
async def get_nodes(namespace: str | None = "guardian"):
    """
    Get bootstrap nodes for namespace.

    GET /nodes?namespace=guardian

    Returns list of bootstrap IPs for initial cluster discovery.
    """
    with registry_lock:
        if namespace not in bootstrap_nodes:
            logger.warning(f"No bootstrap nodes found for namespace '{namespace}'")
            return NodesResponse(nodes=[])

        return NodesResponse(nodes=bootstrap_nodes[namespace])


# --- Dynamic Configuration API ---


@app.post("/config")
async def configure_namespace(config: NamespaceConfig):
    """
    Configure measurements and nodes for a namespace.

    Called by test automation before launching VMs. Replaces any
    existing configuration for the namespace.

    POST /config
    {
        "namespace": "guardian",
        "measurements": [{"rtmr0": "...", "rtmr1": "...", "rtmr2": "...", "rtmr3": "..."}],
        "nodes": [{"ip_address": "192.168.122.20", "raft_port": 8444, "api_port": 8443}]
    }
    """
    namespace = config.namespace

    with registry_lock:
        # Clear existing namespace data
        measurements.pop(namespace, None)
        bootstrap_nodes.pop(namespace, None)

        # Add measurements
        if config.measurements:
            measurements[namespace] = {}
            for entry in config.measurements:
                measurement_hash = compute_measurement_hash(entry.rtmr0, entry.rtmr1, entry.rtmr2, entry.rtmr3)
                trusted = TrustedMeasurement(
                    namespace=namespace,
                    rtmr0=entry.rtmr0,
                    rtmr1=entry.rtmr1,
                    rtmr2=entry.rtmr2,
                    rtmr3=entry.rtmr3,
                    revoked=entry.revoked,
                )
                measurements[namespace][measurement_hash] = trusted

        # Add bootstrap nodes
        if config.nodes:
            bootstrap_nodes[namespace] = list(config.nodes)

    logger.info(
        f"Configured namespace '{namespace}': {len(config.measurements)} measurements, {len(config.nodes)} nodes"
    )

    return {
        "status": "configured",
        "namespace": namespace,
        "measurements": len(config.measurements),
        "nodes": len(config.nodes),
    }


@app.delete("/config")
async def clear_all_config():
    """
    Clear all measurements and nodes.

    Called by test cleanup to reset registry state.
    """
    with registry_lock:
        measurement_count = sum(len(ns) for ns in measurements.values())
        node_count = sum(len(ns) for ns in bootstrap_nodes.values())

        measurements.clear()
        bootstrap_nodes.clear()

    logger.info(f"Cleared all config: {measurement_count} measurements, {node_count} nodes")

    return {
        "status": "cleared",
        "measurements_removed": measurement_count,
        "nodes_removed": node_count,
    }


@app.delete("/config/{namespace}")
async def clear_namespace(namespace: str):
    """
    Clear measurements and nodes for a specific namespace.
    """
    with registry_lock:
        measurement_count = len(measurements.get(namespace, {}))
        node_count = len(bootstrap_nodes.get(namespace, []))

        measurements.pop(namespace, None)
        bootstrap_nodes.pop(namespace, None)

    logger.info(f"Cleared namespace '{namespace}': {measurement_count} measurements, {node_count} nodes")

    return {
        "status": "cleared",
        "namespace": namespace,
        "measurements_removed": measurement_count,
        "nodes_removed": node_count,
    }


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=9000)
