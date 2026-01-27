"""Input injection using uinput for virtual devices."""

import os
from typing import Optional

import evdev
from evdev import UInput, ecodes


class InputInjector:
    """Injects input events using uinput virtual devices."""
    
    def __init__(self):
        self._keyboard: Optional[UInput] = None
        self._mouse: Optional[UInput] = None
    
    def setup(self) -> None:
        """Set up virtual input devices."""
        # Create virtual keyboard with all standard keys
        key_caps = [
            code for code in range(ecodes.KEY_MAX)
            if code not in (ecodes.BTN_LEFT, ecodes.BTN_RIGHT, ecodes.BTN_MIDDLE,
                           ecodes.BTN_SIDE, ecodes.BTN_EXTRA)
        ]
        
        self._keyboard = UInput(
            {
                ecodes.EV_KEY: key_caps,
            },
            name="BlueCross Virtual Keyboard",
        )
        
        # Create virtual mouse
        self._mouse = UInput(
            {
                ecodes.EV_KEY: [
                    ecodes.BTN_LEFT,
                    ecodes.BTN_RIGHT,
                    ecodes.BTN_MIDDLE,
                    ecodes.BTN_SIDE,
                    ecodes.BTN_EXTRA,
                ],
                ecodes.EV_REL: [
                    ecodes.REL_X,
                    ecodes.REL_Y,
                    ecodes.REL_WHEEL,
                    ecodes.REL_HWHEEL,
                ],
            },
            name="BlueCross Virtual Mouse",
        )
        
        print("Virtual devices created:")
        print(f"  Keyboard: {self._keyboard.device.path}")
        print(f"  Mouse: {self._mouse.device.path}")
    
    def close(self) -> None:
        """Close virtual devices."""
        if self._keyboard:
            self._keyboard.close()
            self._keyboard = None
        if self._mouse:
            self._mouse.close()
            self._mouse = None
    
    def inject_key(self, code: int, value: int) -> None:
        """Inject a key event."""
        if not self._keyboard:
            return
        self._keyboard.write(ecodes.EV_KEY, code, value)
        self._keyboard.syn()
    
    def inject_mouse_move(self, dx: int, dy: int) -> None:
        """Inject relative mouse movement."""
        if not self._mouse:
            return
        if dx != 0:
            self._mouse.write(ecodes.EV_REL, ecodes.REL_X, dx)
        if dy != 0:
            self._mouse.write(ecodes.EV_REL, ecodes.REL_Y, dy)
        if dx != 0 or dy != 0:
            self._mouse.syn()
    
    def inject_mouse_button(self, button: int, value: int) -> None:
        """Inject a mouse button event."""
        if not self._mouse:
            return
        self._mouse.write(ecodes.EV_KEY, button, value)
        self._mouse.syn()
    
    def inject_mouse_scroll(self, dx: int, dy: int) -> None:
        """Inject mouse scroll events."""
        if not self._mouse:
            return
        if dy != 0:
            self._mouse.write(ecodes.EV_REL, ecodes.REL_WHEEL, dy)
        if dx != 0:
            self._mouse.write(ecodes.EV_REL, ecodes.REL_HWHEEL, dx)
        if dx != 0 or dy != 0:
            self._mouse.syn()
