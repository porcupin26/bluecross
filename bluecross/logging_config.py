"""Logging configuration and daemonization for BlueCross."""

import atexit
import logging
import os
import signal
import sys
from pathlib import Path
from typing import Optional


def get_log_dir() -> Path:
    """Get the log directory, creating it if necessary."""
    # Use XDG_STATE_HOME or fallback to ~/.local/state
    xdg_state = os.environ.get("XDG_STATE_HOME", os.path.expanduser("~/.local/state"))
    log_dir = Path(xdg_state) / "bluecross" / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    return log_dir


def get_run_dir() -> Path:
    """Get the runtime directory for PID files."""
    # Use XDG_RUNTIME_DIR or fallback to /tmp
    xdg_runtime = os.environ.get("XDG_RUNTIME_DIR", "/tmp")
    run_dir = Path(xdg_runtime) / "bluecross"
    run_dir.mkdir(parents=True, exist_ok=True)
    return run_dir


def get_pid_file(name: str) -> Path:
    """Get the PID file path for a component."""
    return get_run_dir() / f"{name}.pid"


def write_pid_file(name: str) -> None:
    """Write the current PID to the pid file."""
    pid_file = get_pid_file(name)
    pid_file.write_text(str(os.getpid()))
    atexit.register(lambda: pid_file.unlink(missing_ok=True))


def read_pid_file(name: str) -> Optional[int]:
    """Read the PID from the pid file, or None if not running."""
    pid_file = get_pid_file(name)
    if not pid_file.exists():
        return None
    try:
        pid = int(pid_file.read_text().strip())
        # Check if process is running
        os.kill(pid, 0)
        return pid
    except (ValueError, OSError):
        # Invalid PID or process not running
        pid_file.unlink(missing_ok=True)
        return None


def is_running(name: str) -> bool:
    """Check if a component is currently running."""
    return read_pid_file(name) is not None


def stop_daemon(name: str) -> bool:
    """Stop a running daemon. Returns True if stopped, False if not running."""
    pid = read_pid_file(name)
    if pid is None:
        return False
    
    try:
        os.kill(pid, signal.SIGTERM)
        # Wait for process to terminate
        import time
        for _ in range(50):  # Wait up to 5 seconds
            time.sleep(0.1)
            try:
                os.kill(pid, 0)
            except OSError:
                # Process has terminated
                pid_file = get_pid_file(name)
                pid_file.unlink(missing_ok=True)
                return True
        # Force kill if still running
        os.kill(pid, signal.SIGKILL)
        pid_file = get_pid_file(name)
        pid_file.unlink(missing_ok=True)
        return True
    except OSError:
        return False


def daemonize() -> None:
    """Daemonize the current process using double-fork."""
    # First fork
    try:
        pid = os.fork()
        if pid > 0:
            sys.exit(0)
    except OSError as e:
        sys.stderr.write(f"First fork failed: {e}\n")
        sys.exit(1)
    
    # Decouple from parent environment
    os.chdir("/")
    os.setsid()
    os.umask(0)
    
    # Second fork
    try:
        pid = os.fork()
        if pid > 0:
            sys.exit(0)
    except OSError as e:
        sys.stderr.write(f"Second fork failed: {e}\n")
        sys.exit(1)
    
    # Redirect standard file descriptors to /dev/null
    sys.stdout.flush()
    sys.stderr.flush()
    
    with open("/dev/null", "r") as devnull:
        os.dup2(devnull.fileno(), sys.stdin.fileno())
    with open("/dev/null", "a+") as devnull:
        os.dup2(devnull.fileno(), sys.stdout.fileno())
        os.dup2(devnull.fileno(), sys.stderr.fileno())


def setup_logging(name: str, debug: bool = False, foreground: bool = False) -> logging.Logger:
    """Set up logging for a BlueCross component.
    
    Args:
        name: Component name ('server' or 'client')
        debug: Enable debug logging
        foreground: If True, also log to console
    
    Returns:
        Configured logger
    """
    log_dir = get_log_dir()
    
    # Create logger
    logger = logging.getLogger(f"bluecross.{name}")
    logger.setLevel(logging.DEBUG if debug else logging.INFO)
    logger.handlers.clear()
    
    # Formatter
    formatter = logging.Formatter(
        '%(asctime)s - %(name)s - %(levelname)s - %(message)s',
        datefmt='%Y-%m-%d %H:%M:%S'
    )
    
    # File handler for all logs
    all_handler = logging.FileHandler(log_dir / f"{name}.log")
    all_handler.setLevel(logging.DEBUG if debug else logging.INFO)
    all_handler.setFormatter(formatter)
    logger.addHandler(all_handler)
    
    # File handler for errors only
    error_handler = logging.FileHandler(log_dir / f"{name}.error.log")
    error_handler.setLevel(logging.ERROR)
    error_handler.setFormatter(formatter)
    logger.addHandler(error_handler)
    
    # Console handler if running in foreground
    if foreground:
        console_handler = logging.StreamHandler(sys.stdout)
        console_handler.setLevel(logging.DEBUG if debug else logging.INFO)
        console_handler.setFormatter(formatter)
        logger.addHandler(console_handler)
    
    return logger
