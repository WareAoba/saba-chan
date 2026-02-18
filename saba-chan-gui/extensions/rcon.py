"""
saba-chan RCON Extension
==========================
Unified Source RCON protocol implementation used by all game modules.

Protocol Reference: https://developer.valvesoftware.com/wiki/Source_RCON_Protocol

Usage (simple one-shot):
    from extensions.rcon import rcon_command
    response = rcon_command("127.0.0.1", 25575, "password", "list")

Usage (session-based):
    from extensions.rcon import RconClient
    client = RconClient("127.0.0.1", 25575, "password")
    if client.connect():
        response = client.command("list")
        client.disconnect()
"""

import socket
import struct
import random
import sys


# ─── Packet Type Constants ────────────────────────────────────
SERVERDATA_AUTH = 3
SERVERDATA_AUTH_RESPONSE = 2
SERVERDATA_EXECCOMMAND = 2
SERVERDATA_RESPONSE_VALUE = 0


class RconClient:
    """
    Source RCON protocol client.

    Supports connect → authenticate → send commands → disconnect lifecycle.
    Thread-safe for single-connection usage.
    """

    def __init__(self, host="127.0.0.1", port=25575, password="", timeout=5):
        self.host = host
        self.port = int(port)
        self.password = password
        self.timeout = timeout
        self.socket = None
        self.authenticated = False

    def connect(self):
        """Connect and authenticate with the RCON server.

        Returns:
            bool: True if connected and authenticated successfully.
        """
        try:
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.settimeout(self.timeout)
            self.socket.connect((self.host, self.port))

            if self.password:
                if not self._authenticate():
                    return False
                self.authenticated = True

            return True
        except Exception as e:
            print(f"[RCON] Connection failed: {e}", file=sys.stderr)
            self._cleanup()
            return False

    def disconnect(self):
        """Gracefully close the connection."""
        self._cleanup()
        self.authenticated = False

    def command(self, cmd):
        """Send a command and return the response string.

        Args:
            cmd: The command string to execute.

        Returns:
            str or None: Response text, or None on failure.
        """
        if not self.socket:
            if not self.connect():
                return None

        try:
            req_id = self._next_id()
            self._send_packet(req_id, SERVERDATA_EXECCOMMAND, cmd.encode("utf-8"))
            packet = self._read_packet()
            if packet is None:
                return None

            _pid, _ptype, payload = packet
            return payload.decode("utf-8", errors="replace")
        except Exception as e:
            print(f"[RCON] Command failed: {e}", file=sys.stderr)
            return None

    # ── Internal helpers ─────────────────────────────────────

    def _authenticate(self):
        req_id = self._next_id()
        self._send_packet(req_id, SERVERDATA_AUTH, self.password.encode("utf-8"))
        packet = self._read_packet()
        if packet is None:
            return False
        pid, _ptype, _payload = packet
        if pid == -1:
            print("[RCON] Authentication failed (invalid password)", file=sys.stderr)
            return False
        return True

    def _send_packet(self, req_id, pkt_type, payload):
        """Build and send an RCON packet."""
        body = struct.pack("<ii", req_id, pkt_type) + payload + b"\x00\x00"
        packet = struct.pack("<i", len(body)) + body
        self.socket.sendall(packet)

    def _read_packet(self):
        """Read a single RCON response packet.

        Returns:
            tuple(int, int, bytes) or None: (request_id, packet_type, payload)
        """
        try:
            raw_size = self._recv_exact(4)
            if not raw_size or len(raw_size) < 4:
                return None

            size = struct.unpack("<i", raw_size)[0]
            data = self._recv_exact(size)
            if not data or len(data) < 8:
                return None

            req_id = struct.unpack("<i", data[:4])[0]
            pkt_type = struct.unpack("<i", data[4:8])[0]
            payload = data[8:-2] if len(data) > 10 else b""
            return (req_id, pkt_type, payload)
        except socket.timeout:
            print("[RCON] Response timeout", file=sys.stderr)
            return None
        except Exception as e:
            print(f"[RCON] Read error: {e}", file=sys.stderr)
            return None

    def _recv_exact(self, length):
        """Receive exactly `length` bytes from the socket."""
        buf = b""
        while len(buf) < length:
            chunk = self.socket.recv(length - len(buf))
            if not chunk:
                return None
            buf += chunk
        return buf

    def _next_id(self):
        return random.randint(1, 2147483647)

    def _cleanup(self):
        if self.socket:
            try:
                self.socket.close()
            except Exception:
                pass
            self.socket = None

    def __enter__(self):
        self.connect()
        return self

    def __exit__(self, *args):
        self.disconnect()


# ─── Convenience function ─────────────────────────────────────

def rcon_command(host, port, password, command, timeout=5):
    """One-shot RCON command: connect, authenticate, execute, disconnect.

    Args:
        host: RCON server host.
        port: RCON server port.
        password: RCON password.
        command: Command string to execute.
        timeout: Socket timeout in seconds.

    Returns:
        str: Response text.

    Raises:
        ConnectionError: If authentication fails.
        Exception: On connection/protocol errors.
    """
    client = RconClient(host, port, password, timeout)
    try:
        if not client.connect():
            raise ConnectionError("RCON connection/authentication failed")
        response = client.command(command)
        if response is None:
            raise ConnectionError("RCON command returned no response")
        return response
    finally:
        client.disconnect()
