"""OpenObserve client for log queries."""

from __future__ import annotations

from datetime import datetime, timezone

import requests


class OpenObserveClient:
    """Low-level client for OpenObserve API."""

    def __init__(
        self,
        endpoint: str = "http://localhost:5080",
        org: str = "default",
        stream: str = "guardian_logs",
        user: str = "root@example.com",
        password: str = "Complexpass#123",
        timeout: int = 10,
    ):
        self.endpoint = endpoint.rstrip("/")
        self.org = org
        self.stream = stream
        self.auth = (user, password)
        self.timeout = timeout

    def query(
        self,
        start_time: datetime,
        end_time: datetime | None = None,
        test_id: str | None = None,
        limit: int = 500,
    ) -> list[dict]:
        """
        Query logs from OpenObserve.

        Args:
            start_time: Start of time range (required)
            end_time: End of time range (default: now)
            test_id: Filter by test-id field
            limit: Maximum results

        Returns:
            List of raw log dicts from OpenObserve

        Raises:
            ConnectionError: If query fails
        """
        end_time = end_time or datetime.now(timezone.utc)

        # Build WHERE clause
        conditions = []
        if test_id:
            conditions.append(f"\"test-id\" = '{test_id}'")

        where = f" WHERE {' AND '.join(conditions)}" if conditions else ""
        sql = f"SELECT * FROM {self.stream}{where} ORDER BY _timestamp ASC"

        # Convert to microseconds for OpenObserve API
        start_us = int(start_time.timestamp() * 1_000_000)
        end_us = int(end_time.timestamp() * 1_000_000)

        try:
            response = requests.post(
                f"{self.endpoint}/api/{self.org}/_search",
                auth=self.auth,
                json={
                    "query": {
                        "sql": sql,
                        "start_time": start_us,
                        "end_time": end_us,
                        "from": 0,
                        "size": limit,
                    }
                },
                timeout=self.timeout,
            )
            response.raise_for_status()
            return response.json().get("hits", [])
        except requests.RequestException as e:
            raise ConnectionError(f"OpenObserve query failed: {e}") from e

    def health_check(self) -> bool:
        """Check if OpenObserve is reachable."""
        try:
            r = requests.get(f"{self.endpoint}/healthz", timeout=5)
            return r.status_code == 200
        except requests.RequestException:
            return False
