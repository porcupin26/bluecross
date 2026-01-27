#!/usr/bin/env python3
"""BlueCross control utility - start, stop, restart server and client."""

import argparse
import os
import subprocess
import sys
from pathlib import Path

from .logging_config import (
    get_log_dir,
    get_pid_file,
    is_running,
    read_pid_file,
    stop_daemon,
)


def get_default_config() -> Path:
    """Get the default config file path."""
    # Check current directory first
    cwd_config = Path.cwd() / "bluecross.json"
    if cwd_config.exists():
        return cwd_config
    
    # Check XDG_CONFIG_HOME
    xdg_config = os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config"))
    config_dir = Path(xdg_config) / "bluecross"
    config_file = config_dir / "bluecross.json"
    if config_file.exists():
        return config_file
    
    # Default to current directory
    return cwd_config


def cmd_start(args: argparse.Namespace) -> int:
    """Start a component."""
    component = args.component
    config = args.config or get_default_config()
    
    if is_running(component):
        print(f"BlueCross {component} is already running (PID: {read_pid_file(component)})")
        return 1
    
    # Build command
    cmd = [
        sys.executable, "-m",
        f"bluecross.{component}",
        "-c", str(config),
    ]
    
    if args.foreground:
        cmd.append("-f")
    if args.debug:
        cmd.append("-d")
    
    if args.foreground:
        # Run in foreground - replace current process
        os.execv(sys.executable, cmd)
    else:
        # Start in background
        subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        
        # Wait a moment and check if it started
        import time
        time.sleep(0.5)
        
        if is_running(component):
            pid = read_pid_file(component)
            print(f"BlueCross {component} started (PID: {pid})")
            return 0
        else:
            print(f"Failed to start BlueCross {component}")
            print(f"Check logs at: {get_log_dir() / f'{component}.log'}")
            return 1
    
    return 0


def cmd_stop(args: argparse.Namespace) -> int:
    """Stop a component."""
    component = args.component
    
    if not is_running(component):
        print(f"BlueCross {component} is not running")
        return 1
    
    pid = read_pid_file(component)
    print(f"Stopping BlueCross {component} (PID: {pid})...")
    
    if stop_daemon(component):
        print(f"BlueCross {component} stopped")
        return 0
    else:
        print(f"Failed to stop BlueCross {component}")
        return 1


def cmd_restart(args: argparse.Namespace) -> int:
    """Restart a component."""
    component = args.component
    
    if is_running(component):
        cmd_stop(args)
    
    return cmd_start(args)


def cmd_status(args: argparse.Namespace) -> int:
    """Show status of components."""
    components = ["server", "client"] if args.component == "all" else [args.component]
    
    for component in components:
        pid = read_pid_file(component)
        if pid:
            print(f"BlueCross {component}: running (PID: {pid})")
        else:
            print(f"BlueCross {component}: stopped")
    
    return 0


def cmd_logs(args: argparse.Namespace) -> int:
    """Show or follow logs."""
    component = args.component
    log_dir = get_log_dir()
    
    if args.error:
        log_file = log_dir / f"{component}.error.log"
    else:
        log_file = log_dir / f"{component}.log"
    
    if not log_file.exists():
        print(f"No logs found at {log_file}")
        return 1
    
    if args.follow:
        # Use tail -f
        os.execvp("tail", ["tail", "-f", str(log_file)])
    else:
        # Show last N lines
        lines = args.lines or 50
        os.execvp("tail", ["tail", "-n", str(lines), str(log_file)])
    
    return 0


def main() -> int:
    """Main entry point."""
    parser = argparse.ArgumentParser(
        prog="bluecrossctl",
        description="BlueCross control utility",
    )
    subparsers = parser.add_subparsers(dest="command", help="Command to run")
    
    # Start command
    start_parser = subparsers.add_parser("start", help="Start server or client")
    start_parser.add_argument(
        "component",
        choices=["server", "client"],
        help="Component to start",
    )
    start_parser.add_argument(
        "-c", "--config",
        type=Path,
        help="Path to config file",
    )
    start_parser.add_argument(
        "-f", "--foreground",
        action="store_true",
        help="Run in foreground",
    )
    start_parser.add_argument(
        "-d", "--debug",
        action="store_true",
        help="Enable debug logging",
    )
    start_parser.set_defaults(func=cmd_start)
    
    # Stop command
    stop_parser = subparsers.add_parser("stop", help="Stop server or client")
    stop_parser.add_argument(
        "component",
        choices=["server", "client"],
        help="Component to stop",
    )
    stop_parser.set_defaults(func=cmd_stop)
    
    # Restart command
    restart_parser = subparsers.add_parser("restart", help="Restart server or client")
    restart_parser.add_argument(
        "component",
        choices=["server", "client"],
        help="Component to restart",
    )
    restart_parser.add_argument(
        "-c", "--config",
        type=Path,
        help="Path to config file",
    )
    restart_parser.add_argument(
        "-f", "--foreground",
        action="store_true",
        help="Run in foreground after restart",
    )
    restart_parser.add_argument(
        "-d", "--debug",
        action="store_true",
        help="Enable debug logging",
    )
    restart_parser.set_defaults(func=cmd_restart)
    
    # Status command
    status_parser = subparsers.add_parser("status", help="Show status")
    status_parser.add_argument(
        "component",
        nargs="?",
        choices=["server", "client", "all"],
        default="all",
        help="Component to check (default: all)",
    )
    status_parser.set_defaults(func=cmd_status)
    
    # Logs command
    logs_parser = subparsers.add_parser("logs", help="Show logs")
    logs_parser.add_argument(
        "component",
        choices=["server", "client"],
        help="Component logs to show",
    )
    logs_parser.add_argument(
        "-f", "--follow",
        action="store_true",
        help="Follow log output",
    )
    logs_parser.add_argument(
        "-e", "--error",
        action="store_true",
        help="Show error log instead of main log",
    )
    logs_parser.add_argument(
        "-n", "--lines",
        type=int,
        help="Number of lines to show (default: 50)",
    )
    logs_parser.set_defaults(func=cmd_logs)
    
    args = parser.parse_args()
    
    if not args.command:
        parser.print_help()
        return 1
    
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
