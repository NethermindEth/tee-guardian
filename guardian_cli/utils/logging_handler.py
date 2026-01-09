"""Guardian logging configuration using Rich's RichHandler with dedicated Console.

Configures stdout/stderr buffering and creates an isolated Console instance
for logging to stderr, preventing interference with subprocess streams.
"""

import logging
import sys
from rich.console import Console
from rich.logging import RichHandler


def setup_guardian_logging(level=logging.INFO):
    """
    Setup Guardian logging with RichHandler using dedicated Console.

    Configures stdout/stderr for line buffering to ensure predictable output
    ordering when mixing logs with subprocess streams.

    Creates a dedicated Console instance for logging that writes to stderr,
    completely isolated from Rich's global Console cache and subprocess streams.

    Call this once at application startup to configure the root guardian logger.
    Subloggers (guardian.vm, guardian.unit_tests, etc) will inherit this config.

    Args:
        level: Logging level (default: INFO)
    """
    # Configure buffering FIRST - critical for output ordering
    # Line buffering ensures logs appear immediately and in correct order
    # relative to subprocess output
    sys.stdout.reconfigure(line_buffering=True)
    sys.stderr.reconfigure(line_buffering=True)

    # Create root guardian logger
    logger = logging.getLogger("guardian")
    logger.setLevel(level)

    # Remove existing handlers to avoid duplicates
    logger.handlers.clear()

    # Create dedicated Console for logging
    # This writes to stderr and is completely isolated from:
    # - Rich's global Console cache (get_console())
    # - stdout (used by subprocess streams)
    # - status_renderer's Console (writes to StringIO)
    log_console = Console(
        file=sys.stderr,  # Logs go to stderr (best practice)
        force_terminal=True,  # Enable colors/formatting
        width=None,  # Auto-detect terminal width
        legacy_windows=False,  # Modern ANSI support
        force_interactive=False,  # Don't assume interactive
    )

    # Create RichHandler with our dedicated Console
    handler = RichHandler(
        console=log_console,  # USE OUR CONSOLE, not global cache
        show_time=False,  # No timestamps - keep output clean
        show_path=False,  # No file paths - not useful in production
        show_level=False,  # No level prefixes - keep output clean
        markup=True,  # Enable Rich markup like [green]text[/green]
        rich_tracebacks=True,  # Better error formatting
    )

    # Simple formatter - just the message
    handler.setFormatter(logging.Formatter("%(message)s"))
    logger.addHandler(handler)

    # Don't propagate to root logger (avoid duplicate output)
    logger.propagate = False

    return logger
