"""Network protocol for BlueCross communication."""

import asyncio
import json
import struct
from dataclasses import dataclass
from enum import IntEnum
from typing import Optional


class MessageType(IntEnum):
    """Types of messages sent over the network."""
    HANDSHAKE = 1
    HANDSHAKE_ACK = 2
    KEY_EVENT = 3
    MOUSE_MOVE = 4
    MOUSE_BUTTON = 5
    MOUSE_SCROLL = 6
    SWITCH_TO_CLIENT = 7
    SWITCH_TO_SERVER = 8
    HEARTBEAT = 9
    CLIPBOARD_DATA = 10


@dataclass
class Message:
    """A network message."""
    msg_type: MessageType
    payload: dict

    def encode(self) -> bytes:
        """Encode message to bytes."""
        payload_bytes = json.dumps(self.payload).encode("utf-8")
        header = struct.pack("!BH", self.msg_type, len(payload_bytes))
        return header + payload_bytes

    @classmethod
    def decode(cls, data: bytes) -> tuple["Message", bytes]:
        """Decode message from bytes, return message and remaining data."""
        if len(data) < 3:
            raise ValueError("Incomplete header")
        
        msg_type, payload_len = struct.unpack("!BH", data[:3])
        
        if len(data) < 3 + payload_len:
            raise ValueError("Incomplete payload")
        
        payload_bytes = data[3:3 + payload_len]
        payload = json.loads(payload_bytes.decode("utf-8"))
        
        return cls(MessageType(msg_type), payload), data[3 + payload_len:]


class ProtocolHandler:
    """Handles protocol communication over a stream."""
    
    def __init__(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ):
        self.reader = reader
        self.writer = writer
        self._buffer = b""
    
    async def send(self, msg: Message) -> None:
        """Send a message."""
        self.writer.write(msg.encode())
        # Don't drain on every send - let TCP buffer handle it
    
    async def flush(self) -> None:
        """Flush pending writes."""
        await self.writer.drain()
    
    async def receive(self) -> Optional[Message]:
        """Receive a message."""
        while True:
            try:
                msg, self._buffer = Message.decode(self._buffer)
                return msg
            except ValueError:
                chunk = await self.reader.read(65536)
                if not chunk:
                    return None
                self._buffer += chunk
    
    def receive_all_buffered(self) -> list[Message]:
        """Receive all messages currently in the buffer without waiting."""
        messages = []
        while True:
            try:
                msg, self._buffer = Message.decode(self._buffer)
                messages.append(msg)
            except ValueError:
                break
        return messages
    
    def close(self) -> None:
        """Close the connection."""
        self.writer.close()


def create_handshake(client_name: str, screen_width: int, screen_height: int) -> Message:
    """Create a handshake message."""
    return Message(MessageType.HANDSHAKE, {
        "name": client_name,
        "screen_width": screen_width,
        "screen_height": screen_height,
    })


def create_handshake_ack(position: str, server_width: int, server_height: int) -> Message:
    """Create a handshake acknowledgment."""
    return Message(MessageType.HANDSHAKE_ACK, {
        "position": position,
        "server_width": server_width,
        "server_height": server_height,
    })


def create_key_event(code: int, value: int) -> Message:
    """Create a key event message."""
    return Message(MessageType.KEY_EVENT, {"code": code, "value": value})


def create_mouse_move(x: int, y: int, dx: int, dy: int) -> Message:
    """Create a mouse move message."""
    return Message(MessageType.MOUSE_MOVE, {"x": x, "y": y, "dx": dx, "dy": dy})


def create_mouse_button(button: int, value: int) -> Message:
    """Create a mouse button message."""
    return Message(MessageType.MOUSE_BUTTON, {"button": button, "value": value})


def create_mouse_scroll(dx: int, dy: int) -> Message:
    """Create a mouse scroll message."""
    return Message(MessageType.MOUSE_SCROLL, {"dx": dx, "dy": dy})


def create_switch_to_client(entry_x: int, entry_y: int) -> Message:
    """Create a switch to client message."""
    return Message(MessageType.SWITCH_TO_CLIENT, {"entry_x": entry_x, "entry_y": entry_y})


def create_switch_to_server() -> Message:
    """Create a switch to server message."""
    return Message(MessageType.SWITCH_TO_SERVER, {})


def create_clipboard_data(content: str) -> Message:
    """Create a clipboard data message."""
    return Message(MessageType.CLIPBOARD_DATA, {"content": content})
