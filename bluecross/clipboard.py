"""Clipboard handling for BlueCross."""

import asyncio
import subprocess
from typing import Callable, Optional


class ClipboardManager:
    """Manages clipboard reading and monitoring."""
    
    def __init__(self):
        self._last_content: str = ""
        self._running = False
        self._paused = False
    
    def get_clipboard(self) -> str:
        """Get current clipboard content."""
        try:
            # Try wl-paste for Wayland
            result = subprocess.run(
                ["wl-paste", "--no-newline"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=1,
            )
            if result.returncode == 0:
                return result.stdout
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
        
        try:
            # Fall back to xclip for X11
            result = subprocess.run(
                ["xclip", "-selection", "clipboard", "-o"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=1,
            )
            if result.returncode == 0:
                return result.stdout
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
        
        try:
            # Try xsel as another fallback
            result = subprocess.run(
                ["xsel", "--clipboard", "--output"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
                timeout=1,
            )
            if result.returncode == 0:
                return result.stdout
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
        
        return ""
    
    def set_clipboard(self, content: str) -> None:
        """Set clipboard content."""
        if not content:
            return
            
        try:
            # Try wl-copy for Wayland
            # Let it fork to background (default behavior) to serve paste requests
            subprocess.Popen(
                ["wl-copy"],
                stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            ).communicate(input=content.encode(), timeout=1)
            return
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
        
        try:
            # Fall back to xclip for X11
            result = subprocess.run(
                ["xclip", "-selection", "clipboard"],
                input=content,
                text=True,
                timeout=1,
            )
            if result.returncode == 0:
                return
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
        
        try:
            # Try xsel as another fallback
            subprocess.run(
                ["xsel", "--clipboard", "--input"],
                input=content,
                text=True,
                timeout=1,
            )
        except (FileNotFoundError, subprocess.TimeoutExpired, OSError):
            pass
    
    async def monitor(self, on_change: Callable[[str], None]) -> None:
        """Monitor clipboard for changes and call callback when changed."""
        self._running = True
        self._last_content = self.get_clipboard()
        
        # Try to use wl-paste --watch for efficient Wayland monitoring
        try:
            await self._monitor_wayland(on_change)
            return
        except (FileNotFoundError, OSError):
            pass
        
        # Fall back to polling for X11
        await self._monitor_polling(on_change)
    
    async def _monitor_wayland(self, on_change: Callable[[str], None]) -> None:
        """Monitor clipboard using wl-paste --watch (efficient, no polling)."""
        process = await asyncio.create_subprocess_exec(
            "wl-paste", "--watch", "cat",
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
            stdin=asyncio.subprocess.DEVNULL,
        )
        
        try:
            while self._running and process.stdout:
                if self._paused:
                    await asyncio.sleep(0.1)
                    continue
                
                try:
                    # Read until we get data or timeout
                    line = await asyncio.wait_for(
                        process.stdout.readline(),
                        timeout=0.5
                    )
                    if not line:
                        break
                    
                    # wl-paste --watch cat outputs the full clipboard each time
                    # Read remaining content
                    content = line.decode('utf-8', errors='replace')
                    while True:
                        try:
                            more = await asyncio.wait_for(
                                process.stdout.readline(),
                                timeout=0.05
                            )
                            if not more:
                                break
                            content += more.decode('utf-8', errors='replace')
                        except asyncio.TimeoutError:
                            break
                    
                    content = content.rstrip('\n')
                    if content and content != self._last_content:
                        self._last_content = content
                        preview = content[:50] + "..." if len(content) > 50 else content
                        preview = preview.replace("\n", "\\n")
                        print(f"Clipboard changed: {preview}")
                        on_change(content)
                        
                except asyncio.TimeoutError:
                    continue
        finally:
            process.terminate()
            try:
                await asyncio.wait_for(process.wait(), timeout=1.0)
            except asyncio.TimeoutError:
                process.kill()
    
    async def _monitor_polling(self, on_change: Callable[[str], None]) -> None:
        """Monitor clipboard by polling (fallback for X11)."""
        while self._running:
            await asyncio.sleep(1.0)
            if self._paused:
                continue
            content = self.get_clipboard()
            if content and content != self._last_content:
                self._last_content = content
                preview = content[:50] + "..." if len(content) > 50 else content
                preview = preview.replace("\n", "\\n")
                print(f"Clipboard changed: {preview}")
                on_change(content)
    
    def pause(self) -> None:
        """Pause clipboard monitoring."""
        self._paused = True
        print("Clipboard monitoring paused")
    
    def resume(self) -> None:
        """Resume clipboard monitoring."""
        self._paused = False
        print("Clipboard monitoring resumed")
    
    def set_last_content(self, content: str) -> None:
        """Update last known content to avoid echoing back received clipboard."""
        self._last_content = content
    
    def stop(self) -> None:
        """Stop monitoring."""
        self._running = False
