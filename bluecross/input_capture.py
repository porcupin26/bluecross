"""Input capture for keyboard and mouse events."""

import asyncio
import os
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import AsyncIterator, Callable, Optional

import evdev
from evdev import InputDevice, UInput, ecodes


class EventType(Enum):
    """Types of input events."""
    KEY = "key"
    MOUSE_MOVE = "mouse_move"
    MOUSE_BUTTON = "mouse_button"
    MOUSE_SCROLL = "mouse_scroll"


@dataclass
class InputEvent:
    """An input event."""
    event_type: EventType
    code: int = 0
    value: int = 0
    x: int = 0
    y: int = 0
    dx: int = 0
    dy: int = 0


class InputCapturer:
    """Captures input events from keyboard and mouse devices."""
    
    def __init__(self):
        self._devices: list[InputDevice] = []
        self._keyboard_devices: list[InputDevice] = []
        self._mouse_devices: list[InputDevice] = []
        self._grabbed = False
        self._mouse_x = 0
        self._mouse_y = 0
        self._screen_width = 1920
        self._screen_height = 1080
    
    def set_screen_size(self, width: int, height: int) -> None:
        """Set the screen size for mouse position tracking."""
        self._screen_width = width
        self._screen_height = height
        self._mouse_x = width // 2
        self._mouse_y = height // 2
    
    def discover_devices(self) -> None:
        """Discover keyboard and mouse input devices."""
        self._devices = []
        self._keyboard_devices = []
        self._mouse_devices = []
        
        for path in Path("/dev/input").glob("event*"):
            try:
                device = InputDevice(str(path))
                caps = device.capabilities()
                
                # Check for keyboard (has KEY events with actual key codes)
                if ecodes.EV_KEY in caps:
                    key_caps = caps[ecodes.EV_KEY]
                    # Keyboard has letter keys
                    if ecodes.KEY_A in key_caps:
                        self._keyboard_devices.append(device)
                        self._devices.append(device)
                        continue
                    # Mouse has buttons
                    if ecodes.BTN_LEFT in key_caps or ecodes.BTN_MOUSE in key_caps:
                        self._mouse_devices.append(device)
                        self._devices.append(device)
                        continue
                
                # Check for relative mouse movement
                if ecodes.EV_REL in caps:
                    rel_caps = caps[ecodes.EV_REL]
                    if ecodes.REL_X in rel_caps or ecodes.REL_Y in rel_caps:
                        if device not in self._devices:
                            self._mouse_devices.append(device)
                            self._devices.append(device)
                            
            except (PermissionError, OSError):
                continue
        
        print(f"Discovered {len(self._keyboard_devices)} keyboard(s) and {len(self._mouse_devices)} mouse/pointer(s)")
        for dev in self._devices:
            print(f"  - {dev.name} ({dev.path})")
    
    def grab_devices(self) -> None:
        """Grab exclusive access to input devices."""
        if self._grabbed:
            return
        for device in self._devices:
            try:
                device.grab()
            except OSError:
                pass
        self._grabbed = True
    
    def ungrab_devices(self) -> None:
        """Release exclusive access to input devices."""
        if not self._grabbed:
            return
        for device in self._devices:
            try:
                device.ungrab()
            except OSError:
                pass
        self._grabbed = False
    
    def get_mouse_position(self) -> tuple[int, int]:
        """Get current mouse position."""
        return self._mouse_x, self._mouse_y
    
    def set_mouse_position(self, x: int, y: int) -> None:
        """Set current mouse position."""
        self._mouse_x = max(0, min(x, self._screen_width - 1))
        self._mouse_y = max(0, min(y, self._screen_height - 1))
    
    async def read_events(self) -> AsyncIterator[InputEvent]:
        """Read input events from all devices."""
        if not self._devices:
            self.discover_devices()
        
        # Single queue that all device readers push to directly
        combined_queue: asyncio.Queue = asyncio.Queue()
        tasks = []
        
        for device in self._devices:
            async def reader(dev=device):
                try:
                    async for ev in dev.async_read_loop():
                        await combined_queue.put((dev, ev))
                except OSError:
                    pass
            
            tasks.append(asyncio.create_task(reader()))
        
        try:
            while True:
                device, event = await combined_queue.get()
                
                # Handle key events
                if event.type == ecodes.EV_KEY:
                    # Mouse buttons
                    if event.code in (ecodes.BTN_LEFT, ecodes.BTN_RIGHT, ecodes.BTN_MIDDLE,
                                     ecodes.BTN_SIDE, ecodes.BTN_EXTRA):
                        yield InputEvent(
                            event_type=EventType.MOUSE_BUTTON,
                            code=event.code,
                            value=event.value,
                        )
                    else:
                        # Keyboard key
                        yield InputEvent(
                            event_type=EventType.KEY,
                            code=event.code,
                            value=event.value,
                        )
                
                # Handle relative mouse movement
                elif event.type == ecodes.EV_REL:
                    if event.code == ecodes.REL_X:
                        dx = event.value
                        self._mouse_x = max(0, min(self._mouse_x + dx, self._screen_width - 1))
                        yield InputEvent(
                            event_type=EventType.MOUSE_MOVE,
                            x=self._mouse_x,
                            y=self._mouse_y,
                            dx=dx,
                            dy=0,
                        )
                    elif event.code == ecodes.REL_Y:
                        dy = event.value
                        self._mouse_y = max(0, min(self._mouse_y + dy, self._screen_height - 1))
                        yield InputEvent(
                            event_type=EventType.MOUSE_MOVE,
                            x=self._mouse_x,
                            y=self._mouse_y,
                            dx=0,
                            dy=dy,
                        )
                    elif event.code == ecodes.REL_WHEEL:
                        yield InputEvent(
                            event_type=EventType.MOUSE_SCROLL,
                            dy=event.value,
                        )
                    elif event.code == ecodes.REL_HWHEEL:
                        yield InputEvent(
                            event_type=EventType.MOUSE_SCROLL,
                            dx=event.value,
                        )
                        
        finally:
            for task in tasks:
                task.cancel()


def get_display_server() -> str:
    """Detect the display server in use."""
    if os.environ.get("WAYLAND_DISPLAY"):
        return "wayland"
    if os.environ.get("DISPLAY"):
        return "x11"
    return "unknown"
