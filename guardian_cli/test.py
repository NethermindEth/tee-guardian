"""Test commands for Guardian CLI."""

import click
import sys


@click.group()
def test():
    """Run Guardian tests."""
    pass


@test.command()
@click.option("--no-tui", is_flag=True, help="Run without TUI")
def unit(no_tui):
    """Run Rust unit tests in dev VM."""
    from guardian_cli.utils import load_config, ensure_nebula, ensure_host_ssh

    config = load_config()
    ensure_host_ssh(config, "tdx")
    ensure_nebula("tdx")

    if no_tui:
        from guardian_cli.commands.run_unit_tests import main

        main(launch_tui=False)
    else:
        from .tui import launch_zellij

        launch_zellij(tab_name="unit-tests", python_module="guardian_cli.commands.run_unit_tests")


@test.command()
@click.option("--no-tui", is_flag=True, help="Run without TUI")
@click.option("--test", "-t", "test_name", default=None, help="Run specific test")
@click.option("-n", "concurrency", default=1, type=int, help="Number of clusters")
@click.option("--tdx", is_flag=True, help="Run on TDX host")
@click.option("--sev", is_flag=True, help="Run on SEV-SNP host")
def integration(no_tui, test_name, concurrency, tdx, sev):
    """Run integration tests.

    Must specify at least one host: --tdx and/or --sev
    """
    if not tdx and not sev:
        raise click.ClickException("Specify host: --tdx and/or --sev")

    from guardian_cli.utils import load_config, ensure_nebula, ensure_host_ssh
    from guardian_cli.utils.cluster import ClusterAllocator

    config = load_config()
    host_types = []

    if tdx:
        ensure_host_ssh(config, "tdx")
        ensure_nebula("tdx")
        host_types.append("tdx")
    if sev:
        ensure_host_ssh(config, "sev")
        ensure_nebula("sev")
        host_types.append("sev")

    # Acquire clusters (locks are on TDX host, shared across all)
    allocator = ClusterAllocator(config)
    clusters = []
    try:
        for _ in range(concurrency):
            clusters.append(allocator.acquire(purpose="integration-tests"))

        cluster_ids = [c.cluster_id for c in clusters]
        cluster_ids_str = ",".join(str(cid) for cid in cluster_ids)
        host_types_str = ",".join(host_types)

        if no_tui:
            from guardian_cli.commands.run_integration_tests import main

            exit_code = main(launch_tui=False, test_name=test_name, cluster_ids=cluster_ids, host_types=host_types)
            sys.exit(exit_code)
        else:
            from .tui import launch_zellij

            module_args = ["--cluster-ids", cluster_ids_str, "--host-types", host_types_str]
            if test_name:
                module_args.extend(["--test", test_name])
            launch_zellij(
                tab_name="integration-tests",
                python_module="guardian_cli.commands.run_integration_tests",
                module_args=module_args,
            )
    finally:
        for cluster in clusters:
            allocator.release(cluster)


@test.command()
@click.option("--no-tui", is_flag=True, help="Run without TUI")
@click.option("--test", "-t", "test_name", default=None, help="Run specific test")
@click.option("-n", "concurrency", default=1, type=int, help="Number of clusters")
@click.option("--tdx", is_flag=True, help="Run on TDX host")
@click.option("--sev", is_flag=True, help="Run on SEV-SNP host")
def bootstrap(no_tui, test_name, concurrency, tdx, sev):
    """Run bootstrap tests.

    Must specify at least one host: --tdx and/or --sev
    """
    if not tdx and not sev:
        raise click.ClickException("Specify host: --tdx and/or --sev")

    from guardian_cli.utils import load_config, ensure_nebula, ensure_host_ssh
    from guardian_cli.utils.cluster import ClusterAllocator

    config = load_config()
    host_types = []

    if tdx:
        ensure_host_ssh(config, "tdx")
        ensure_nebula("tdx")
        host_types.append("tdx")
    if sev:
        ensure_host_ssh(config, "sev")
        ensure_nebula("sev")
        host_types.append("sev")

    allocator = ClusterAllocator(config)
    clusters = []
    try:
        for _ in range(concurrency):
            clusters.append(allocator.acquire(purpose="bootstrap-tests"))

        cluster_ids = [c.cluster_id for c in clusters]
        cluster_ids_str = ",".join(str(cid) for cid in cluster_ids)
        host_types_str = ",".join(host_types)

        if no_tui:
            from guardian_cli.commands.run_bootstrap_tests import main

            exit_code = main(launch_tui=False, test_name=test_name, cluster_ids=cluster_ids, host_types=host_types)
            sys.exit(exit_code)
        else:
            from .tui import launch_zellij

            module_args = ["--cluster-ids", cluster_ids_str, "--host-types", host_types_str]
            if test_name:
                module_args.extend(["--test", test_name])
            launch_zellij(
                tab_name="bootstrap-tests",
                python_module="guardian_cli.commands.run_bootstrap_tests",
                module_args=module_args,
            )
    finally:
        for cluster in clusters:
            allocator.release(cluster)
