#!/usr/bin/env python3
"""Debug tool to show screen info and mouse coordinates on click."""

import subprocess
import sys

def get_screen_info():
    """Get all screen detection info."""
    print("=" * 60)
    print("SCREEN DETECTION INFO")
    print("=" * 60)
    
    # xrandr
    print("\n--- xrandr ---")
    try:
        result = subprocess.run(["xrandr", "--current"], capture_output=True, text=True, timeout=2)
        for line in result.stdout.split('\n'):
            if 'connected' in line or 'current' in line.lower():
                print(line)
    except Exception as e:
        print(f"Error: {e}")
    
    # GNOME Mutter
    print("\n--- GNOME Mutter D-Bus ---")
    try:
        result = subprocess.run(
            ["gdbus", "call", "--session",
             "--dest", "org.gnome.Mutter.DisplayConfig",
             "--object-path", "/org/gnome/Mutter/DisplayConfig",
             "--method", "org.gnome.Mutter.DisplayConfig.GetCurrentState"],
            capture_output=True, text=True, timeout=2
        )
        if result.returncode == 0:
            import re
            # Find current resolution
            current = re.search(r"\('(\d+)x(\d+)@[\d.]+',\s*\d+,\s*\d+,\s*[\d.]+,\s*[\d.]+,\s*\[[^\]]+\],\s*\{[^}]*'is-current':\s*<true>", result.stdout)
            if current:
                print(f"Physical resolution: {current.group(1)}x{current.group(2)}")
            
            # Find scale
            scale_match = re.search(r"\[\((\d+),\s*(\d+),\s*([\d.]+),\s*uint32\s*\d+,\s*true,", result.stdout)
            if scale_match:
                print(f"Scale: {scale_match.group(3)}")
                w, h = int(current.group(1)), int(current.group(2))
                s = float(scale_match.group(3))
                print(f"Logical resolution: {int(w/s)}x{int(h/s)}")
        else:
            print("Not available")
    except Exception as e:
        print(f"Error: {e}")
    
    # BlueCross detection
    print("\n--- BlueCross detect_screen_size() ---")
    try:
        from bluecross.screen import detect_screen_size
        w, h = detect_screen_size()
        print(f"Detected: {w}x{h}")
    except Exception as e:
        print(f"Error: {e}")
    
    print("\n" + "=" * 60)
    print("MOUSE POSITION (click anywhere, Ctrl+C to exit)")
    print("=" * 60)


def track_mouse():
    """Track mouse position using xdotool."""
    print("\nUsing xdotool to get mouse position...")
    print("Press 'c' to capture current mouse position. Press Ctrl+C to exit.\n")
    
    try:
        # Check if xdotool is available
        subprocess.run(["which", "xdotool"], capture_output=True, check=True)
    except:
        print("xdotool not found. Trying alternative method...")
        track_mouse_evdev()
        return
    
    import sys
    import tty
    import termios
    
    # Set terminal to raw mode to capture single keypresses
    fd = sys.stdin.fileno()
    old_settings = termios.tcgetattr(fd)
    
    try:
        tty.setraw(fd)
        while True:
            ch = sys.stdin.read(1)
            if ch == 'c' or ch == 'C':
                result = subprocess.run(
                    ["xdotool", "getmouselocation"],
                    capture_output=True, text=True, timeout=1
                )
                if result.returncode == 0:
                    # Parse: x:123 y:456 screen:0 window:12345
                    parts = result.stdout.strip().split()
                    x = int(parts[0].split(':')[1])
                    y = int(parts[1].split(':')[1])
                    print(f"\rMouse position: x={x}, y={y}                    \n")
            elif ch == '\x03':  # Ctrl+C
                break
    except Exception as e:
        print(f"\rError: {e}")
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)
        print("\nExiting.")


def track_mouse_evdev():
    """Track mouse using evdev (fallback)."""
    print("\nUsing evdev to track mouse position...")
    print("Press 'c' key to capture position. Press Ctrl+C to exit.\n")
    
    try:
        import evdev
        from evdev import InputDevice, ecodes
        import selectors
        
        # Find mouse and keyboard devices
        devices = [evdev.InputDevice(path) for path in evdev.list_devices()]
        mouse = None
        keyboard = None
        
        for dev in devices:
            caps = dev.capabilities()
            if ecodes.EV_REL in caps and mouse is None:
                mouse = dev
                print(f"Mouse: {dev.name} ({dev.path})")
            if ecodes.EV_KEY in caps:
                key_caps = caps[ecodes.EV_KEY]
                if ecodes.KEY_C in key_caps and keyboard is None:
                    keyboard = dev
                    print(f"Keyboard: {dev.name} ({dev.path})")
        
        if not mouse:
            print("No mouse device found")
            return
        
        if not keyboard:
            print("No keyboard device found")
            return
        
        x, y = 0, 0
        print("\nTracking... Press 'c' to print position, Ctrl+C to exit:")
        
        # Use selector to monitor both devices
        sel = selectors.DefaultSelector()
        sel.register(mouse, selectors.EVENT_READ)
        sel.register(keyboard, selectors.EVENT_READ)
        
        while True:
            for key, mask in sel.select():
                device = key.fileobj
                for event in device.read():
                    if event.type == ecodes.EV_REL:
                        if event.code == ecodes.REL_X:
                            x += event.value
                        elif event.code == ecodes.REL_Y:
                            y += event.value
                    elif event.type == ecodes.EV_KEY and event.value == 1:  # Key press
                        if event.code == ecodes.KEY_C:
                            print(f"Position: x={x}, y={y}")
                
    except ImportError:
        print("evdev not available")
    except KeyboardInterrupt:
        print("\nExiting.")
    except Exception as e:
        print(f"Error: {e}")


def main():
    get_screen_info()
    track_mouse()


if __name__ == "__main__":
    main()
