"""
Cluster log streaming and pattern matching for integration tests.

Uses Polars for high-performance vectorized regex matching.
"""

from __future__ import annotations

import json
import threading
import time
from datetime import datetime, timezone

import polars as pl

from tests.utils.openobserve import OpenObserveClient


class LogWaitTimeoutError(TimeoutError):
    """
    Detailed timeout error for log pattern waiting.

    Provides rich diagnostics showing which filters matched, which didn't,
    and recent logs from unmatched sources.
    """

    def __init__(
        self,
        pattern: str,
        timeout: float,
        match_all: list[dict[str, str]] | None,
        matched: list[tuple[dict[str, str], dict]],
        unmatched: list[dict[str, str]],
        recent_logs_by_filter: dict[str, dict | None],
        error_logs: pl.DataFrame,
        total_logs: int,
    ):
        self.pattern = pattern
        self.timeout = timeout
        self.match_all = match_all
        self.matched = matched
        self.unmatched = unmatched
        self.recent_logs_by_filter = recent_logs_by_filter
        self.error_logs = error_logs
        self.total_logs = total_logs
        super().__init__(self._format_message())

    def _format_message(self) -> str:
        lines = [f"Timed out after {self.timeout:.1f}s waiting for pattern '{self.pattern}'"]

        if self.match_all:
            lines.append(f"\n  match_all: {len(self.match_all)} required, {len(self.matched)} matched\n")

            for filter_dict, entry in self.matched:
                filter_str = ", ".join(f"{k}: {v}" for k, v in filter_dict.items())
                ts = datetime.fromtimestamp(entry["timestamp_us"] / 1_000_000, tz=timezone.utc)
                lines.append(f"  ✓ {{{filter_str}}} @ {ts.strftime('%H:%M:%S.%f')[:-3]}")
                msg = entry["message"][:80] + "..." if len(entry["message"]) > 80 else entry["message"]
                lines.append(f'      "{msg}"')

            for filter_dict in self.unmatched:
                filter_str = ", ".join(f"{k}: {v}" for k, v in filter_dict.items())
                lines.append(f"  ✗ {{{filter_str}}} - NO MATCH")

                key = _filter_key(filter_dict)
                if last := self.recent_logs_by_filter.get(key):
                    ts = datetime.fromtimestamp(last["timestamp_us"] / 1_000_000, tz=timezone.utc)
                    msg = last["message"][:60] + "..." if len(last["message"]) > 60 else last["message"]
                    lines.append(f'      last: "{msg}" @ {ts.strftime("%H:%M:%S.%f")[:-3]}')
                else:
                    lines.append("      (no logs from this source)")

        if not self.error_logs.is_empty():
            lines.append(f"\n  Recent ERRORs ({len(self.error_logs)}):")
            for row in self.error_logs.head(5).iter_rows(named=True):
                ts = datetime.fromtimestamp(row["timestamp_us"] / 1_000_000, tz=timezone.utc)
                msg = row["message"][:70] + "..." if len(row["message"]) > 70 else row["message"]
                lines.append(f'    [{row["host_id"]}] {ts.strftime("%H:%M:%S.%f")[:-3]} "{msg}"')
            if len(self.error_logs) > 5:
                lines.append(f"    ... +{len(self.error_logs) - 5} more")

        lines.append(f"\n  Total logs: {self.total_logs}")
        return "\n".join(lines)


def _filter_key(filter_dict: dict[str, str]) -> str:
    """Create a hashable key from filter dict."""
    return "|".join(f"{k}={v}" for k, v in sorted(filter_dict.items()))


def _build_filter_expr(filter_dict: dict[str, str]) -> pl.Expr:
    """Build a Polars filter expression from a filter dict."""
    exprs = []
    for field, value in filter_dict.items():
        if field == "host_id":
            exprs.append(pl.col("host_id") == value)
        else:
            # Search in extra_fields JSON
            exprs.append(pl.col("extra_fields").str.contains(f'"{field}": "{value}"', literal=True))
    return exprs[0] if len(exprs) == 1 else pl.all_horizontal(exprs)


class ClusterLogs:
    """
    Log streaming and pattern matching for cluster integration tests.

    Streams logs from OpenObserve, stores in Polars DataFrame for fast
    vectorized regex matching across multiple nodes.

    Usage:
        with ClusterLogs(test_id="test_leader_election", cluster_ips=ips) as logs:
            df = logs.wait_for_pattern(
                r"Cluster bootstrap successful",
                match_all=[{"host_id": ip} for ip in ips]
            )
    """

    # DataFrame schema
    SCHEMA = {
        "timestamp_us": pl.Int64,
        "level": pl.Categorical,
        "message": pl.String,
        "host_id": pl.Categorical,
        "extra_fields": pl.String,
    }

    def __init__(
        self,
        test_id: str,
        cluster_ips: list[str],
        client: OpenObserveClient | None = None,
        poll_interval: float = 1.0,
    ):
        self.test_id = test_id
        self.cluster_ips = cluster_ips
        self._client = client or OpenObserveClient()
        self._poll_interval = poll_interval
        self._start_time = datetime.now(timezone.utc)

        self._df = pl.DataFrame(schema=self.SCHEMA)
        self._seen: set[str] = set()

        self._lock = threading.Lock()
        self._condition = threading.Condition(self._lock)
        self._running = False
        self._thread: threading.Thread | None = None

    def start(self) -> ClusterLogs:
        """Start background log polling."""
        if self._running:
            return self

        if not self._client.health_check():
            raise ConnectionError(
                f"OpenObserve unavailable at {self._client.endpoint}. "
                "Cannot run integration tests without log aggregation."
            )

        self._running = True
        self._thread = threading.Thread(target=self._poll_loop, daemon=True, name=f"ClusterLogs-{self.test_id}")
        self._thread.start()
        return self

    def stop(self) -> None:
        """Stop background log polling."""
        self._running = False
        if self._thread:
            self._thread.join(timeout=self._poll_interval + 1)
            self._thread = None

    def wait_for_pattern(
        self,
        pattern: str,
        *,
        timeout: float = 30.0,
        level: str | None = None,
        match_all: list[dict[str, str]] | None = None,
        **filters: str,
    ) -> pl.DataFrame:
        """
        Wait for log entries matching pattern and filters.

        Args:
            pattern: Regex pattern to match against message
            timeout: Maximum seconds to wait
            level: Required log level (INFO, WARN, ERROR, DEBUG)
            match_all: List of filter dicts - must match pattern for EACH dict
            **filters: Simple field=value filters (convenience for single match)

        Returns:
            DataFrame with matching log entries

        Raises:
            LogWaitTimeoutError: With detailed diagnostics
            ValueError: If both match_all and **filters specified
        """
        if match_all and filters:
            raise ValueError("Cannot specify both match_all and **filters")

        if filters:
            match_all = [filters]

        deadline = time.time() + timeout

        with self._condition:
            while True:
                if match_all:
                    matched, unmatched = self._check_match_all(pattern, level, match_all)
                    if not unmatched:
                        return pl.DataFrame([m[1] for m in matched], schema=self.SCHEMA)
                else:
                    df = self._filter(pattern, level)
                    if not df.is_empty():
                        return df.head(1)
                    matched, unmatched = [], []

                remaining = deadline - time.time()
                if remaining <= 0:
                    raise LogWaitTimeoutError(
                        pattern=pattern,
                        timeout=timeout,
                        match_all=match_all,
                        matched=matched,
                        unmatched=unmatched,
                        recent_logs_by_filter={_filter_key(f): self._most_recent_for_filter(f) for f in unmatched},
                        error_logs=self._df.filter(pl.col("level").str.to_uppercase() == "ERROR"),
                        total_logs=len(self._df),
                    )

                self._condition.wait(timeout=min(remaining, self._poll_interval))

    def get_logs(
        self,
        pattern: str | None = None,
        level: str | None = None,
        **filters: str,
    ) -> pl.DataFrame:
        """Get captured logs matching filters (non-blocking)."""
        with self._lock:
            return self._filter(pattern, level, filters if filters else None)

    def clear(self) -> None:
        """Clear captured logs."""
        with self._lock:
            self._df = pl.DataFrame(schema=self.SCHEMA)
            self._seen.clear()

    def _filter(
        self,
        pattern: str | None = None,
        level: str | None = None,
        filters: dict[str, str] | None = None,
    ) -> pl.DataFrame:
        """Apply filters to internal DataFrame."""
        if self._df.is_empty():
            return self._df

        expr: pl.Expr | None = None

        if pattern:
            expr = pl.col("message").str.contains(pattern, literal=False)
        if level:
            level_expr = pl.col("level").str.to_uppercase() == level.upper()
            expr = level_expr if expr is None else expr & level_expr
        if filters:
            filter_expr = _build_filter_expr(filters)
            expr = filter_expr if expr is None else expr & filter_expr

        return self._df.filter(expr) if expr else self._df

    def _check_match_all(
        self,
        pattern: str,
        level: str | None,
        match_all: list[dict[str, str]],
    ) -> tuple[list[tuple[dict[str, str], dict]], list[dict[str, str]]]:
        """Check if pattern matches for all filter dicts."""
        if self._df.is_empty():
            return [], list(match_all)

        # Pre-filter by pattern (expensive, do once)
        base = self._filter(pattern, level)
        if base.is_empty():
            return [], list(match_all)

        matched: list[tuple[dict[str, str], dict]] = []
        unmatched: list[dict[str, str]] = []

        for filt in match_all:
            filtered = base.filter(_build_filter_expr(filt))
            if not filtered.is_empty():
                matched.append((filt, filtered.row(0, named=True)))
            else:
                unmatched.append(filt)

        return matched, unmatched

    def _most_recent_for_filter(self, filter_dict: dict[str, str]) -> dict | None:
        """Find most recent log matching filter (ignoring pattern)."""
        if self._df.is_empty():
            return None
        filtered = self._df.filter(_build_filter_expr(filter_dict)).sort("timestamp_us", descending=True)
        return filtered.row(0, named=True) if not filtered.is_empty() else None

    def _poll_loop(self) -> None:
        """Background polling loop."""
        while self._running:
            try:
                self._poll_once()
            except Exception as e:
                print(f"[ClusterLogs] Poll error: {e}")
            time.sleep(self._poll_interval)

    def _poll_once(self) -> None:
        """Execute one poll cycle."""
        try:
            raw_logs = self._client.query(start_time=self._start_time, test_id=self.test_id, limit=1000)
        except Exception:
            return

        if not raw_logs:
            return

        new_rows = []
        for raw in raw_logs:
            ts_us = raw.get("_timestamp", 0)
            message = raw.get("message", "")
            host_id = raw.get("host-id", "")

            key = f"{ts_us}|{host_id}|{hash(message)}"
            if key in self._seen:
                continue
            self._seen.add(key)

            level = raw.get("level", "UNKNOWN")
            extra = {k: str(v) for k, v in raw.items() if k not in {"_timestamp", "level", "message", "host-id"}}

            new_rows.append(
                {
                    "timestamp_us": ts_us,
                    "level": level,
                    "message": message,
                    "host_id": host_id,
                    "extra_fields": json.dumps(extra) if extra else "{}",
                }
            )

        if new_rows:
            with self._condition:
                self._df = pl.concat([self._df, pl.DataFrame(new_rows, schema=self.SCHEMA)])
                self._condition.notify_all()

    def __enter__(self) -> ClusterLogs:
        return self.start()

    def __exit__(self, *_: object) -> None:
        self.stop()
