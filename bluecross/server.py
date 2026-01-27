"""BlueCross server - captures input and sends to clients."""

import argparse
import asyncio
import logging
import signal
import sys
import time
from pathlib import Path
from typing import Optional

from .clipboard import ClipboardManager
from .config import ScreenPosition, ServerConfig, load_server_config
from .input_capture import EventType, InputCapturer, InputEvent
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
    create_handshake_ack,
    create_key_event,
    create_mouse_button,
    create_mouse_move,
    create_mouse_scroll,
    create_switch_to_client,
)

log: logging.Logger = logging.getLogger("bluecross.server")


class ConnectedClient:
    """Represents a connected client."""
    
    def __init__(
        self,
        name: str,
        position: ScreenPosition,
        handler: ProtocolHandler,
        screen_width: int,
        screen_height: int,
    ):
        self.name = name
        self.position = position
        self.handler = handler
        self.screen_width = screen_width
        self.screen_height = screen_height


class Server:
    """BlueCross server."""
    
    def __init__(self, config: ServerConfig):
        self.config = config
        self.capturer = InputCapturer()
        self.injector = InputInjector()
        self.clipboard = ClipboardManager()
        self.clients: dict[str, ConnectedClient] = {}
        self.active_client: Optional[ConnectedClient] = None
        self._running = False
        self._server: Optional[asyncio.Server] = None
        self._pending_dx = 0
        self._pending_dy = 0
        self._last_flush = 0.0
    
    def _check_edge(self, x: int, y: int) -> Optional[tuple[ConnectedClient, int, int]]:
        """Check if mouse is at an edge that should trigger a switch."""
        threshold = self.config.edge_threshold
        server_w = self.config.screen_width
        server_h = self.config.screen_height
        # Entry offset to prevent immediate bounce-back (enter well inside the screen)
        # Must be larger than the exit threshold to avoid immediate exit triggers
        entry_offset = 100
        
        for client in self.clients.values():
            pos = client.position
            client_w = client.screen_width
            client_h = client.screen_height
            
            if pos == ScreenPosition.LEFT and x <= threshold:
                # Entry point: right side of client screen, but inside the edge
                # Scale Y position proportionally to client screen
                entry_x = client_w - entry_offset
                entry_y = int(y * client_h / server_h)
                return client, entry_x, entry_y
            
            elif pos == ScreenPosition.RIGHT and x >= server_w - threshold - 1:
                # Scale Y position proportionally to client screen
                entry_x = entry_offset
                entry_y = int(y * client_h / server_h)
                return client, entry_x, entry_y
            
            elif pos == ScreenPosition.TOP and y <= threshold:
                # Scale X position proportionally to client screen
                entry_x = int(x * client_w / server_w)
                entry_y = client_h - entry_offset
                return client, entry_x, entry_y
            
            elif pos == ScreenPosition.BOTTOM and y >= server_h - threshold - 1:
                # Scale X position proportionally to client screen
                entry_x = int(x * client_w / server_w)
                entry_y = entry_offset
                return client, entry_x, entry_y
        
        return None
    
    async def _handle_client(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        """Handle a client connection."""
        addr = writer.get_extra_info("peername")
        log.info(f"Client connecting from {addr}")
        
        handler = ProtocolHandler(reader, writer)
        client_name = "unknown"
        
        try:
            # Wait for handshake
            msg = await handler.receive()
            if msg is None or msg.msg_type != MessageType.HANDSHAKE:
                log.warning(f"Invalid handshake from {addr}")
                handler.close()
                return
            
            client_name = msg.payload["name"]
            client_screen_width = msg.payload.get("screen_width", 1920)
            client_screen_height = msg.payload.get("screen_height", 1080)
            
            # Check if client is configured
            if client_name not in self.config.clients:
                log.warning(f"Unknown client '{client_name}' from {addr}")
                handler.close()
                return
            
            position = self.config.clients[client_name]
            
            # Send ack with position info
            await handler.send(create_handshake_ack(
                position.value,
                self.config.screen_width,
                self.config.screen_height,
            ))
            
            client = ConnectedClient(
                client_name, position, handler,
                client_screen_width, client_screen_height,
            )
            self.clients[client_name] = client
            log.info(f"Client '{client_name}' connected from {addr} (position: {position.value}, screen: {client_screen_width}x{client_screen_height})")
            
            # Listen for messages from client
            while self._running:
                msg = await handler.receive()
                if msg is None:
                    break
                
                if msg.msg_type == MessageType.SWITCH_TO_SERVER:
                    # Client wants to return control to server
                    if self.active_client == client:
                        self.active_client = None
                        self.capturer.ungrab_devices()
                        self.clipboard.resume()
                        log.info(f"Control returned from client '{client_name}'")
                
                elif msg.msg_type == MessageType.CLIPBOARD_DATA:
                    if self.config.clipboard_sharing:
                        content = msg.payload.get("content", "")
                        preview = content[:50] + "..." if len(content) > 50 else content
                        preview = preview.replace("\n", "\\n")
                        log.debug(f"Received clipboard from {client_name}: {preview}")
                        self.clipboard.set_last_content(content)
                        self.clipboard.set_clipboard(content)
                        # Broadcast to other clients
                        for other_client in self.clients.values():
                            if other_client != client:
                                try:
                                    await other_client.handler.send(msg)
                                except Exception:
                                    pass
                
        except Exception as e:
            log.error(f"Error with client {addr}: {e}")
        finally:
            if client_name in self.clients:
                del self.clients[client_name]
            if self.active_client and self.active_client.name == client_name:
                self.active_client = None
                self.capturer.ungrab_devices()
                self.clipboard.resume()
            handler.close()
            log.info(f"Client '{client_name}' disconnected")
    
    async def _process_events(self) -> None:
        """Process input events and forward to active client."""
        self.capturer.set_screen_size(
            self.config.screen_width,
            self.config.screen_height,
        )
        self.capturer.discover_devices()
        
        async for event in self.capturer.read_events():
            if not self._running:
                break
            
            if self.active_client:
                # Forward events to active client
                try:
                    if event.event_type == EventType.KEY:
                        # Flush any pending mouse movement first
                        if self._pending_dx != 0 or self._pending_dy != 0:
                            await self.active_client.handler.send(
                                create_mouse_move(event.x, event.y, self._pending_dx, self._pending_dy)
                            )
                            self._pending_dx = 0
                            self._pending_dy = 0
                        await self.active_client.handler.send(
                            create_key_event(event.code, event.value)
                        )
                        await self.active_client.handler.flush()
                    elif event.event_type == EventType.MOUSE_MOVE:
                        # Accumulate mouse movements
                        self._pending_dx += event.dx
                        self._pending_dy += event.dy
                        
                        # Send batched movement periodically (every ~2ms)
                        now = time.monotonic()
                        if now - self._last_flush >= 0.002:
                            await self.active_client.handler.send(
                                create_mouse_move(event.x, event.y, self._pending_dx, self._pending_dy)
                            )
                            self._pending_dx = 0
                            self._pending_dy = 0
                            self._last_flush = now
                    elif event.event_type == EventType.MOUSE_BUTTON:
                        # Flush pending movement before button event
                        if self._pending_dx != 0 or self._pending_dy != 0:
                            await self.active_client.handler.send(
                                create_mouse_move(event.x, event.y, self._pending_dx, self._pending_dy)
                            )
                            self._pending_dx = 0
                            self._pending_dy = 0
                        await self.active_client.handler.send(
                            create_mouse_button(event.code, event.value)
                        )
                        await self.active_client.handler.flush()
                    elif event.event_type == EventType.MOUSE_SCROLL:
                        await self.active_client.handler.send(
                            create_mouse_scroll(event.dx, event.dy)
                        )
                except Exception as e:
                    log.error(f"Error sending to client: {e}")
                    self.active_client = None
                    self.capturer.ungrab_devices()
                    self.clipboard.resume()
            else:
                # Check for edge transition
                if event.event_type == EventType.MOUSE_MOVE:
                    result = self._check_edge(event.x, event.y)
                    if result:
                        client, entry_x, entry_y = result
                        self.active_client = client
                        self.capturer.grab_devices()
                        self.clipboard.pause()
                        log.info(f"Switching to client '{client.name}'")
                        
                        try:
                            await client.handler.send(
                                create_switch_to_client(entry_x, entry_y)
                            )
                        except Exception as e:
                            log.error(f"Error switching to client: {e}")
                            self.active_client = None
                            self.capturer.ungrab_devices()
                            self.clipboard.resume()
    
    async def _monitor_clipboard(self) -> None:
        """Monitor clipboard and broadcast changes to all clients."""
        def on_clipboard_change(content: str) -> None:
            # Schedule broadcast as a task since we're in a sync callback
            asyncio.create_task(self._broadcast_clipboard(content))
        
        await self.clipboard.monitor(on_clipboard_change)
    
    async def _broadcast_clipboard(self, content: str) -> None:
        """Broadcast clipboard content to all connected clients."""
        if not self.clients:
            return
        msg = create_clipboard_data(content)
        log.debug(f"Broadcasting clipboard to {len(self.clients)} client(s)")
        for client in self.clients.values():
            try:
                await client.handler.send(msg)
                await client.handler.flush()
            except Exception as e:
                log.error(f"Error sending clipboard to {client.name}: {e}")

    async def run(self) -> None:
        """Run the server."""
        self._running = True
        
        # Start TCP server
        self._server = await asyncio.start_server(
            self._handle_client,
            self.config.host,
            self.config.port,
        )
        
        addr = self._server.sockets[0].getsockname()
        log.info(f"Server listening on {addr[0]}:{addr[1]}")
        log.info(f"Screen size: {self.config.screen_width}x{self.config.screen_height}")
        log.info(f"Edge threshold: {self.config.edge_threshold}px")
        log.info(f"Configured clients: {list(self.config.clients.keys())}")
        log.info(f"Clipboard sharing: {'enabled' if self.config.clipboard_sharing else 'disabled'}")
        
        # Start clipboard monitoring if enabled
        clipboard_task = None
        if self.config.clipboard_sharing:
            clipboard_task = asyncio.create_task(self._monitor_clipboard())
        
        # Run event processing
        try:
            await self._process_events()
        finally:
            if clipboard_task:
                clipboard_task.cancel()
            self.clipboard.stop()
            self._running = False
            if self._server:
                self._server.close()
                await self._server.wait_closed()
            self.capturer.ungrab_devices()
    
    def stop(self) -> None:
        """Stop the server."""
        self._running = False
        # Cancel all running tasks to unblock async operations
        for task in asyncio.all_tasks():
            if task is not asyncio.current_task():
                task.cancel()


async def main() -> None:
    """Main entry point."""
    global log
    
    parser = argparse.ArgumentParser(description="BlueCross Server")
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
    if is_running("server"):
        print("BlueCross server is already running")
        sys.exit(1)
    
    # Daemonize if not running in foreground
    if not args.foreground:
        daemonize()
    
    # Set up logging
    log = setup_logging("server", debug=args.debug, foreground=args.foreground)
    
    # Write PID file
    write_pid_file("server")
    
    config = load_server_config(args.config)
    server = Server(config)
    
    # Handle signals
    loop = asyncio.get_event_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, server.stop)
    
    if args.foreground:
        print("BlueCross Server")
        print("================")
        print("Press Ctrl+C to stop")
        print()
    
    log.info("BlueCross server starting")
    
    try:
        await server.run()
    except (KeyboardInterrupt, asyncio.CancelledError):
        pass
    finally:
        server.stop()
        log.info("BlueCross server stopped")


def run() -> None:
    """Synchronous entry point for console script."""
    asyncio.run(main())


if __name__ == "__main__":
    run()
