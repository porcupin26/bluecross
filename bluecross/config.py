"""Configuration management for BlueCross."""

import json
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Optional

from .screen import detect_screen_size


class ScreenPosition(Enum):
    """Position of client screen relative to server screen."""
    LEFT = "left"
    RIGHT = "right"
    TOP = "top"
    BOTTOM = "bottom"


@dataclass
class ServerConfig:
    """Server configuration."""
    host: str = "0.0.0.0"
    port: int = 12345
    screen_width: int = 0  # 0 means auto-detect
    screen_height: int = 0  # 0 means auto-detect
    edge_threshold: int = 5  # pixels from edge to trigger switch
    clipboard_sharing: bool = True
    clients: dict[str, ScreenPosition] = field(default_factory=dict)


@dataclass
class ClientConfig:
    """Client configuration."""
    server_host: str = "127.0.0.1"
    server_port: int = 12345
    screen_width: int = 0  # 0 means auto-detect
    screen_height: int = 0  # 0 means auto-detect
    name: str = "client1"
    clipboard_sharing: bool = True


def load_server_config(path: Optional[Path] = None) -> ServerConfig:
    """Load server configuration from file."""
    if path is None:
        path = Path("bluecross.json")
    
    if not path.exists():
        return ServerConfig()
    
    with open(path) as f:
        data = json.load(f)
    
    server_data = data.get("server", {})
    clients = {}
    for name, pos in server_data.get("clients", {}).items():
        clients[name] = ScreenPosition(pos)
    
    # Auto-detect screen size if not specified
    screen_width = server_data.get("screen_width", 0)
    screen_height = server_data.get("screen_height", 0)
    if screen_width == 0 or screen_height == 0:
        detected_width, detected_height = detect_screen_size()
        if screen_width == 0:
            screen_width = detected_width
        if screen_height == 0:
            screen_height = detected_height
    
    return ServerConfig(
        host=server_data.get("host", "0.0.0.0"),
        port=server_data.get("port", 12345),
        screen_width=screen_width,
        screen_height=screen_height,
        edge_threshold=server_data.get("edge_threshold", 5),
        clipboard_sharing=server_data.get("clipboard_sharing", True),
        clients=clients,
    )


def load_client_config(path: Optional[Path] = None) -> ClientConfig:
    """Load client configuration from file."""
    if path is None:
        path = Path("bluecross.json")
    
    if not path.exists():
        return ClientConfig()
    
    with open(path) as f:
        data = json.load(f)
    
    client_data = data.get("client", {})
    
    # Auto-detect screen size if not specified
    screen_width = client_data.get("screen_width", 0)
    screen_height = client_data.get("screen_height", 0)
    if screen_width == 0 or screen_height == 0:
        detected_width, detected_height = detect_screen_size()
        if screen_width == 0:
            screen_width = detected_width
        if screen_height == 0:
            screen_height = detected_height
    
    return ClientConfig(
        server_host=client_data.get("server_host", "127.0.0.1"),
        server_port=client_data.get("server_port", 12345),
        screen_width=screen_width,
        screen_height=screen_height,
        name=client_data.get("name", "client1"),
        clipboard_sharing=client_data.get("clipboard_sharing", True),
    )
