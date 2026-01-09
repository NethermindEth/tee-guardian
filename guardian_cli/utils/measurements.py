"""TEE measurement extraction utilities."""

import logging

from guardian_cli.utils import remote_execute

logger = logging.getLogger("guardian.measurements")


def extract_measurements(
    config: dict,
    workspace: str,
    ip_octet: int,
    host_type: str = "tdx",
    progress=None,
):
    """
    Extract TEE measurements by running the extract script on the specified host.

    Measurements are cached globally at /tmp/guardian-rtmr-cache/{hash}/measurements.txt.
    The script handles caching - if measurements exist for this boot.img hash, it returns immediately.

    Args:
        config: Guardian config dict
        workspace: Remote workspace path
        ip_octet: IP octet for the extraction VM (e.g., 20 for 10.42.X.20)
        host_type: Host type to extract from ("tdx" or "sev")
        progress: Optional rich Progress instance for UI feedback
    """
    task_id = None
    tee_type = "TDX" if host_type == "tdx" else "SEV-SNP"
    if progress:
        task_id = progress.add_task(f"Extract {tee_type} measurements", total=1)

    extract_script = f"{workspace}/test-vm-result/extract-{host_type}-measurements"
    remote_execute(config, f"{extract_script} --ip-octet {ip_octet}", streaming=True, host_type=host_type)

    if progress and task_id is not None:
        progress.update(task_id, completed=1)

    logger.info(f"{tee_type} measurements extracted and cached")
