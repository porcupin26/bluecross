"""BlueCross client - receives input from server and injects locally."""

import argparse
import asyncio
import logging
import signal
import sys
import time
from pathlib import Path
from typing import Optional

from .clipboard import ClipboardManager
from .config import ClientConfig, ScreenPosition, load_client_config
from .input_inject import InputInjector
from .logging_config import (
    daemonize,
    is_running,
    setup_logging,
    write_pid_file,
)
from .protocol import (
    Message,
    MessageType,
    ProtocolHandler,
    create_clipboard_data,
    create_handshake,
    create_switch_to_server,
)

log: logging.Logger = logging.getLogger("bluecross.client")


class Client:
    """BlueCross client."""
    
    def __init__(self, config: ClientConfig):
        self.config = config
        self.injector = InputInjector()
        self.clipboard = ClipboardManager()
        self.position: Optional[ScreenPosition] = None
        self.server_width = 1920
        self.server_height = 1080
        self._running = False
        self._active = False
        self._mouse_x = 0
        self._mouse_y = 0
        self._handler: Optional[ProtocolHandler] = None
        self._switch_time = 0.0  # Time when control was received
        self._edge_push_start = 0.0  # Time when started pushing against exit edge
        self._edge_push_count = 0  # Number of consecutive edge pushes
    
    def _check_exit_edge(self, x: int, y: int, dx: int, dy: int) -> bool:
        """Check if mouse is at the very edge and pushing to return to server.
        
        Only triggers when mouse is at the absolute edge AND sustained pushing
        in the direction that would cross back to the server for 300ms.
        """
        if not self.position:
            return False
        
        # Grace period after switching - ignore exit edge for 200ms
        if time.monotonic() - self._switch_time < 0.2:
            return False
        
        w = self.config.screen_width
        h = self.config.screen_height
        
        # Check if at the edge
        at_edge = False
        pushing_exit = False
        moving_away = False
        
        if self.position == ScreenPosition.LEFT:
            at_edge = x >= w - 1
            pushing_exit = dx > 0
            moving_away = dx < 0
        elif self.position == ScreenPosition.RIGHT:
            at_edge = x <= 0
            pushing_exit = dx < 0
            moving_away = dx > 0
        elif self.position == ScreenPosition.TOP:
            at_edge = y >= h - 1
            pushing_exit = dy > 0
            moving_away = dy < 0
        elif self.position == ScreenPosition.BOTTOM:
            at_edge = y <= 0
            pushing_exit = dy < 0
            moving_away = dy > 0
        
        now = time.monotonic()
        
        if at_edge and pushing_exit:
            # At edge and pushing towards exit
            if self._edge_push_start == 0:
                self._edge_push_start = now
                log.debug(f"Edge timer started at x={x}")
            elapsed = now - self._edge_push_start
            if elapsed >= 0.3:
                # Pushed for 300ms, trigger exit
                log.debug(f"Edge timer completed: {elapsed:.3f}s")
                self._edge_push_start = 0
                return True
        elif moving_away or not at_edge:
            # Reset if actively moving away from edge or left the edge
            if self._edge_push_start != 0:
                log.debug(f"Edge timer reset at x={x}, moving_away={moving_away}, at_edge={at_edge}")
            self._edge_push_start = 0
        # If at edge with dx=0 (stationary), keep the timer running
        
        return False
    
    def _process_message(self, msg: Message) -> tuple[int, int]:
        """Process a single message. Returns (dx, dy) for mouse moves."""
        if msg.msg_type == MessageType.SWITCH_TO_CLIENT:
            self._active = True
            self._switch_time = time.monotonic()
            self._edge_push_start = 0
            self._mouse_x = msg.payload.get("entry_x", self.config.screen_width // 2)
            self._mouse_y = msg.payload.get("entry_y", self.config.screen_height // 2)
            log.info(f"Control received (entry point: {self._mouse_x}, {self._mouse_y})")
            return (0, 0)
        
        # Clipboard should always be processed regardless of active state
        if msg.msg_type == MessageType.CLIPBOARD_DATA:
            if self.config.clipboard_sharing:
                content = msg.payload.get("content", "")
                preview = content[:50] + "..." if len(content) > 50 else content
                preview = preview.replace("\n", "\\n")
                log.debug(f"Received clipboard: {preview}")
                self.clipboard.set_last_content(content)
                self.clipboard.set_clipboard(content)
            return (0, 0)
        
        if not self._active:
            return (0, 0)
        
        if msg.msg_type == MessageType.KEY_EVENT:
            self.injector.inject_key(
                msg.payload["code"],
                msg.payload["value"],
            )
        elif msg.msg_type == MessageType.MOUSE_MOVE:
            return (msg.payload["dx"], msg.payload["dy"])
        elif msg.msg_type == MessageType.MOUSE_BUTTON:
            self.injector.inject_mouse_button(
                msg.payload["button"],
                msg.payload["value"],
            )
        elif msg.msg_type == MessageType.MOUSE_SCROLL:
            self.injector.inject_mouse_scroll(
                msg.payload.get("dx", 0),
                msg.payload.get("dy", 0),
            )
        return (0, 0)

    async def _handle_messages(self, handler: ProtocolHandler) -> None:
        """Handle messages from server."""
        while self._running:
            msg = await handler.receive()
            if msg is None:
                log.warning("Connection lost")
                break
            
            # Process the first message and accumulate mouse delta
            total_dx, total_dy = self._process_message(msg)
            
            # Process all buffered messages to catch up
            for buffered_msg in handler.receive_all_buffered():
                dx, dy = self._process_message(buffered_msg)
                total_dx += dx
                total_dy += dy
            
            # Apply accumulated mouse movement
            if total_dx != 0 or total_dy != 0:
                self._mouse_x = max(0, min(
                    self._mouse_x + total_dx,
                    self.config.screen_width - 1,
                ))
                self._mouse_y = max(0, min(
                    self._mouse_y + total_dy,
                    self.config.screen_height - 1,
                ))
                
                # Check exit using original movement direction (not clamped position change)
                if self._check_exit_edge(self._mouse_x, self._mouse_y, total_dx, total_dy):
                    self._active = False
                    log.info(f"Returning control to server (pos={self._mouse_x},{self._mouse_y})")
                    await handler.send(create_switch_to_server())
                else:
                    self.injector.inject_mouse_move(total_dx, total_dy)
    
    async def _monitor_clipboard(self) -> None:
        """Monitor clipboard and send changes to server."""
        def on_clipboard_change(content: str) -> None:
            if self._handler:
                asyncio.create_task(self._send_clipboard(content))
        
        await self.clipboard.monitor(on_clipboard_change)
    
    async def _send_clipboard(self, content: str) -> None:
        """Send clipboard content to server."""
        if self._handler:
            try:
                await self._handler.send(create_clipboard_data(content))
            except Exception:
                pass

    async def run(self) -> None:
        """Run the client."""
        self._running = True
        
        # Set up virtual devices
        self.injector.setup()
        
        log.info(f"Connecting to server at {self.config.server_host}:{self.config.server_port}")
        
        try:
            reader, writer = await asyncio.open_connection(
                self.config.server_host,
                self.config.server_port,
            )
        except ConnectionRefusedError:
            log.error("Connection refused. Is the server running?")
            return
        except Exception as e:
            log.error(f"Connection failed: {e}")
            return
        
        handler = ProtocolHandler(reader, writer)
        self._handler = handler
        
        # Send handshake with screen size
        await handler.send(create_handshake(
            self.config.name,
            self.config.screen_width,
            self.config.screen_height,
        ))
        
        # Wait for ack
        msg = await handler.receive()
        if msg is None or msg.msg_type != MessageType.HANDSHAKE_ACK:
            log.error("Handshake failed")
            handler.close()
            return
        
        self.position = ScreenPosition(msg.payload["position"])
        self.server_width = msg.payload.get("server_width", 1920)
        self.server_height = msg.payload.get("server_height", 1080)
        
        log.info(f"Connected as '{self.config.name}'")
        log.info(f"Position: {self.position.value} of server screen")
        log.info(f"Server screen: {self.server_width}x{self.server_height}")
        log.info(f"Client screen: {self.config.screen_width}x{self.config.screen_height}")
        log.info(f"Clipboard sharing: {'enabled' if self.config.clipboard_sharing else 'disabled'}")
        log.info("Waiting for input...")
        
        # Start clipboard monitoring if enabled
        clipboard_task = None
        if self.config.clipboard_sharing:
            clipboard_task = asyncio.create_task(self._monitor_clipboard())
        
        try:
            await self._handle_messages(handler)
        finally:
            if clipboard_task:
                clipboard_task.cancel()
            self.clipboard.stop()
            handler.close()
            self.injector.close()
    
    def stop(self) -> None:
        """Stop the client."""
        self._running = False
        # Cancel all running tasks to unblock async operations
        for task in asyncio.all_tasks():
            if task is not asyncio.current_task():
                task.cancel()


async def main() -> None:
    """Main entry point."""
    global log
    
    parser = argparse.ArgumentParser(description="BlueCross Client")
    parser.add_argument(
        "-c", "--config",
        type=Path,
        default=Path("bluecross.json"),
        help="Path to config file (default: bluecross.json)",
    )
    parser.add_argument(
        "-f", "--foreground",
        action="store_true",
        help="Run in foreground (don't daemonize)",
    )
    parser.add_argument(
        "-d", "--debug",
        action="store_true",
        help="Enable debug logging",
    )
    args = parser.parse_args()
    
    # Check if already running
    if is_running("client"):
        print("BlueCross client is already running")
        sys.exit(1)
    
    # Daemonize if not running in foreground
    if not args.foreground:
        daemonize()
    
    # Set up logging
    log = setup_logging("client", debug=args.debug, foreground=args.foreground)
    
    # Write PID file
    write_pid_file("client")
    
    config = load_client_config(args.config)
    client = Client(config)
    
    # Handle signals
    loop = asyncio.get_event_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, client.stop)
    
    if args.foreground:
        print("BlueCross Client")
        print("================")
        print("Press Ctrl+C to stop")
        print()
    
    log.info("BlueCross client starting")
    
    try:
        await client.run()
    except (KeyboardInterrupt, asyncio.CancelledError):
        pass
    finally:
        client.stop()
        log.info("BlueCross client stopped")


def run() -> None:
    """Synchronous entry point for console script."""
    asyncio.run(main())


if __name__ == "__main__":
    run()
