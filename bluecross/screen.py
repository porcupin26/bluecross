"""Screen detection for BlueCross."""

import os
import re
import subprocess
from typing import Optional


def detect_screen_size() -> tuple[int, int]:
    """Detect the primary screen physical resolution.
    
    Returns:
        Tuple of (width, height) in physical pixels (before scaling).
        Mouse coordinates from evdev/xdotool are in physical pixels.
    """
    # Try xrandr first - reports physical resolution directly
    result = _detect_xrandr()
    if result:
        return result
    
    # Try GNOME/Mutter - get physical resolution (not logical)
    result = _detect_gnome_mutter_physical()
    if result:
        return result
    
    # Try wlr-randr physical
    result = _detect_wlr_randr_physical()
    if result:
        return result
    
    # Try KDE Plasma Wayland physical
    result = _detect_kde_wayland_physical()
    if result:
        return result
    
    # Default fallback
    print("Warning: Could not detect screen size, using default 1920x1080")
    return (1920, 1080)


def _detect_gnome_mutter_physical() -> Optional[tuple[int, int]]:
    """Detect physical screen size using GNOME Mutter D-Bus interface."""
    try:
        result = subprocess.run(
            [
                "gdbus", "call", "--session",
                "--dest", "org.gnome.Mutter.DisplayConfig",
                "--object-path", "/org/gnome/Mutter/DisplayConfig",
                "--method", "org.gnome.Mutter.DisplayConfig.GetCurrentState"
            ],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Find resolution with 'is-current': <true> - this is the physical resolution
        current_match = re.search(
            r"\('(\d+)x(\d+)@[\d.]+',\s*\d+,\s*\d+,\s*[\d.]+,\s*[\d.]+,\s*\[[^\]]+\],\s*\{[^}]*'is-current':\s*<true>",
            output
        )
        
        if current_match:
            width = int(current_match.group(1))
            height = int(current_match.group(2))
            return (width, height)
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_wlr_randr_physical() -> Optional[tuple[int, int]]:
    """Detect physical screen size using wlr-randr."""
    try:
        result = subprocess.run(
            ["wlr-randr"],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Parse wlr-randr output - get physical resolution (before scale)
        for line in output.split('\n'):
            if '(current)' in line:
                res_match = re.search(r'(\d+)x(\d+)\s+px', line)
                if res_match:
                    width = int(res_match.group(1))
                    height = int(res_match.group(2))
                    return (width, height)
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_kde_wayland_physical() -> Optional[tuple[int, int]]:
    """Detect physical screen size on KDE Plasma Wayland."""
    try:
        result = subprocess.run(
            ["kscreen-doctor", "--outputs"],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Get physical geometry before scaling
        for line in output.split('\n'):
            if 'Geometry:' in line:
                geo_match = re.search(r'Geometry:\s*\d+,\d+\s+(\d+)x(\d+)', line)
                if geo_match:
                    width = int(geo_match.group(1))
                    height = int(geo_match.group(2))
                    return (width, height)
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_gnome_mutter() -> Optional[tuple[int, int]]:
    """Detect screen size using GNOME Mutter D-Bus interface."""
    try:
        result = subprocess.run(
            [
                "gdbus", "call", "--session",
                "--dest", "org.gnome.Mutter.DisplayConfig",
                "--object-path", "/org/gnome/Mutter/DisplayConfig",
                "--method", "org.gnome.Mutter.DisplayConfig.GetCurrentState"
            ],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Parse the logical monitor configuration
        # Format includes: (x, y, scale, transform, primary, monitors, properties)
        # We need to find the primary monitor (primary=true) and get its resolution/scale
        
        # Find the monitors section with is-current
        # Look for resolution with 'is-current': <true>
        current_match = re.search(
            r"\('(\d+)x(\d+)@[\d.]+',\s*\d+,\s*\d+,\s*[\d.]+,\s*[\d.]+,\s*\[[^\]]+\],\s*\{[^}]*'is-current':\s*<true>",
            output
        )
        
        if current_match:
            width = int(current_match.group(1))
            height = int(current_match.group(2))
        else:
            # Fallback: try to find any resolution
            res_match = re.search(r"\('(\d+)x(\d+)@", output)
            if not res_match:
                return None
            width = int(res_match.group(1))
            height = int(res_match.group(2))
        
        # Find the scale from logical monitor config
        # Format: (x, y, scale, transform, primary, ...)
        # Look for the primary monitor (last bool is true)
        scale_match = re.search(
            r"\[\((\d+),\s*(\d+),\s*([\d.]+),\s*uint32\s*\d+,\s*true,",
            output
        )
        
        scale = 1.0
        if scale_match:
            scale = float(scale_match.group(3))
        
        # Return logical size (physical / scale)
        logical_width = int(width / scale)
        logical_height = int(height / scale)
        
        return (logical_width, logical_height)
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_kde_wayland() -> Optional[tuple[int, int]]:
    """Detect screen size on KDE Plasma Wayland."""
    try:
        result = subprocess.run(
            ["kscreen-doctor", "--outputs"],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Parse kscreen-doctor output
        # Look for enabled output with resolution and scale
        # Format: Output: 1 ... Geometry: 0,0 1920x1080 ... Scale: 1.5
        
        width = None
        height = None
        scale = 1.0
        
        for line in output.split('\n'):
            if 'Geometry:' in line:
                geo_match = re.search(r'Geometry:\s*\d+,\d+\s+(\d+)x(\d+)', line)
                if geo_match:
                    width = int(geo_match.group(1))
                    height = int(geo_match.group(2))
            if 'Scale:' in line:
                scale_match = re.search(r'Scale:\s*([\d.]+)', line)
                if scale_match:
                    scale = float(scale_match.group(1))
        
        if width and height:
            return (int(width / scale), int(height / scale))
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_wlr_randr() -> Optional[tuple[int, int]]:
    """Detect screen size using wlr-randr (wlroots compositors)."""
    try:
        result = subprocess.run(
            ["wlr-randr"],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Parse wlr-randr output
        # Format:
        # OutputName
        #   ...
        #   1920x1080 px, 60.000000 Hz (current)
        #   Scale: 1.500000
        
        width = None
        height = None
        scale = 1.0
        
        for line in output.split('\n'):
            if '(current)' in line:
                res_match = re.search(r'(\d+)x(\d+)\s+px', line)
                if res_match:
                    width = int(res_match.group(1))
                    height = int(res_match.group(2))
            if 'Scale:' in line:
                scale_match = re.search(r'Scale:\s*([\d.]+)', line)
                if scale_match:
                    scale = float(scale_match.group(1))
        
        if width and height:
            return (int(width / scale), int(height / scale))
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _detect_xrandr() -> Optional[tuple[int, int]]:
    """Detect screen size using xrandr (X11)."""
    try:
        result = subprocess.run(
            ["xrandr", "--current"],
            capture_output=True,
            text=True,
            timeout=2,
        )
        if result.returncode != 0:
            return None
        
        output = result.stdout
        
        # Parse xrandr output
        # Look for: Screen 0: ... current 1920 x 1080 ...
        screen_match = re.search(r'current\s+(\d+)\s*x\s*(\d+)', output)
        if screen_match:
            width = int(screen_match.group(1))
            height = int(screen_match.group(2))
            return (width, height)
        
        # Alternative: look for connected primary output
        # Format: HDMI-1 connected primary 1920x1080+0+0
        primary_match = re.search(
            r'connected\s+primary\s+(\d+)x(\d+)',
            output
        )
        if primary_match:
            width = int(primary_match.group(1))
            height = int(primary_match.group(2))
            return (width, height)
        
        # Fallback: first connected output
        connected_match = re.search(
            r'connected\s+(\d+)x(\d+)',
            output
        )
        if connected_match:
            width = int(connected_match.group(1))
            height = int(connected_match.group(2))
            return (width, height)
        
        return None
        
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None
