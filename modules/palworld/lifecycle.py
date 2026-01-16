#!/usr/bin/env python3
"""
Palworld server lifecycle management module.
Always outputs JSON to stdout, logs to stderr.
"""

import sys
import json
import subprocess
import os
import socket
import struct
import time
import random
import base64
import urllib.request
import urllib.error

class PalworldRconClient:
    """Palworld RCON client implementing the correct protocol"""
    
    # Packet type constants
    AUTH = 3
    EXEC_COMMAND = 2
    COMMAND_RESPONSE = 0
    
    def __init__(self, host='127.0.0.1', port=25575, password=''):
        self.host = host
        self.port = int(port)
        self.password = password
        self.socket = None
        self.request_id = 0
        self.authenticated = False
    
    def connect(self):
        """Connect to RCON server"""
        try:
            print(f"[RCON] Connecting to {self.host}:{self.port}...", file=sys.stderr)
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.settimeout(5)
            self.socket.connect((self.host, self.port))
            print("[RCON] ✅ Connected!", file=sys.stderr)
            
            # Authenticate
            if self.password:
                print(f"[RCON] Authenticating...", file=sys.stderr)
                if not self._authenticate(self.password):
                    print("[RCON] ❌ Authentication failed", file=sys.stderr)
                    return False
                print("[RCON] ✅ Authenticated!", file=sys.stderr)
            
            return True
        except Exception as e:
            print(f"[RCON] ❌ Connection failed: {e}", file=sys.stderr)
            return False
    
    def disconnect(self):
        """Disconnect from RCON server"""
        if self.socket:
            try:
                self.socket.close()
            except:
                pass
    
    def send_command(self, command):
        """Send a command to the server"""
        try:
            if not self.socket:
                if not self.connect():
                    return None
            
            print(f"[RCON] Sending command: {command}", file=sys.stderr)
            response = self._send_rcon_command(command)
            print(f"[RCON] Response received: {response[:100] if response else 'None'}", file=sys.stderr)
            return response
        except Exception as e:
            print(f"[RCON] ❌ Command failed: {e}", file=sys.stderr)
            return None
    
    def _authenticate(self, password):
        """Authenticate with RCON server"""
        try:
            packet = self._make_packet(self.AUTH, password.encode('utf-8'))
            self.socket.sendall(packet)
            
            # Read response
            response_packet = self._read_packet()
            if response_packet is None:
                return False
            
            packet_id, packet_type, payload = response_packet
            if packet_id == -1:
                print("[RCON] ❌ Invalid password", file=sys.stderr)
                return False
            
            self.authenticated = True
            return True
        except Exception as e:
            print(f"[RCON] ❌ Auth error: {e}", file=sys.stderr)
            return False
    
    def _send_rcon_command(self, command):
        """Send RCON command and receive response"""
        try:
            if not self.authenticated:
                return None
            
            packet = self._make_packet(self.EXEC_COMMAND, command.encode('utf-8'))
            self.socket.sendall(packet)
            
            # Read response
            response_packet = self._read_packet()
            if response_packet is None:
                return None
            
            packet_id, packet_type, payload = response_packet
            if packet_type != self.COMMAND_RESPONSE:
                print(f"[RCON] ❌ Unexpected response type: {packet_type}", file=sys.stderr)
                return None
            
            return payload.decode('utf-8', errors='ignore')
        except Exception as e:
            print(f"[RCON] ❌ Command error: {e}", file=sys.stderr)
            return None
    
    def _make_packet(self, packet_type, payload):
        """Create RCON packet"""
        self.request_id = random.randint(0, 2147483647)
        
        # Build packet: [id (4B)] [type (4B)] [payload] [terminator (2B)]
        packet_data = struct.pack('<i', self.request_id)
        packet_data += struct.pack('<i', packet_type)
        packet_data += payload
        packet_data += b'\x00\x00'
        
        # Add size prefix
        size = len(packet_data)
        full_packet = struct.pack('<i', size) + packet_data
        
        return full_packet
    
    def _read_packet(self):
        """Read RCON packet response"""
        try:
            # Read size (4 bytes)
            size_data = self.socket.recv(4)
            if not size_data:
                return None
            
            size = struct.unpack('<i', size_data)[0]
            
            # Read packet data
            packet_data = b''
            while len(packet_data) < size:
                chunk = self.socket.recv(size - len(packet_data))
                if not chunk:
                    break
                packet_data += chunk
            
            if len(packet_data) < 8:
                return None
            
            # Parse packet
            packet_id = struct.unpack('<i', packet_data[:4])[0]
            packet_type = struct.unpack('<i', packet_data[4:8])[0]
            payload = packet_data[8:-2]  # Remove terminator
            
            return (packet_id, packet_type, payload)
        except socket.timeout:
            print("[RCON] ⚠️ Response timeout", file=sys.stderr)
            return None
        except Exception as e:
            print(f"[RCON] ❌ Read error: {e}", file=sys.stderr)
            return None


class PalworldRestClient:
    """Minimal REST client for Palworld built-in REST API."""

    def __init__(self, host='127.0.0.1', port=8212, username='', password='', timeout=5):
        self.base_url = f"http://{host}:{port}/v1/api"
        self.auth_header = self._build_auth(username, password) if username or password else None
        self.timeout = timeout

    def _build_auth(self, username, password):
        token = base64.b64encode(f"{username}:{password}".encode('utf-8')).decode('utf-8')
        return f"Basic {token}"

    def _request(self, method, path, payload=None):
        url = f"{self.base_url}{path}"
        headers = {"Accept": "application/json"}
        if self.auth_header:
            headers["Authorization"] = self.auth_header

        data = None
        if payload is not None:
            data = json.dumps(payload).encode('utf-8')
            headers["Content-Type"] = "application/json"

        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        print(f"[REST] {method} {url} payload={payload}", file=sys.stderr)

        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                body = resp.read().decode('utf-8', errors='ignore')
                if not body:
                    return None
                try:
                    return json.loads(body)
                except json.JSONDecodeError:
                    return body
        except urllib.error.HTTPError as e:
            error_body = e.read().decode('utf-8', errors='ignore') if hasattr(e, 'read') else ''
            raise RuntimeError(f"HTTP {e.code} {e.reason}: {error_body}")
        except Exception as e:
            raise RuntimeError(str(e))

    def announce(self, message):
        return self._request("POST", "/announce", {"message": message})

    def info(self):
        return self._request("GET", "/info")

    def players(self):
        return self._request("GET", "/players")

    def metrics(self):
        return self._request("GET", "/metrics")

    def kick(self, userid, message=None):
        payload = {"userid": userid}
        if message:
            payload["message"] = message
        return self._request("POST", "/kick", payload)

    def ban(self, userid, message=None):
        payload = {"userid": userid}
        if message:
            payload["message"] = message
        return self._request("POST", "/ban", payload)

    def unban(self, userid):
        return self._request("POST", "/unban", {"userid": userid})


def start(config):
    """Start Palworld server"""
    try:
        executable = config.get("server_executable")
        if not executable:
            return {
                "success": False,
                "message": "server_executable not specified in instance configuration. Please add the path to PalServer.exe"
            }
        
        # Check if executable exists
        if not os.path.exists(executable):
            return {
                "success": False,
                "message": f"Executable not found: {executable}. Please check the path in instance settings."
            }
        
        port = config.get("port", 8211)
        working_dir = config.get("working_dir")
        
        # Use working directory if specified, otherwise use executable's directory
        if not working_dir:
            working_dir = os.path.dirname(executable)
        
        # Construct command
        cmd = [
            executable,
            f"--port={port}"
        ]
        
        # Log for debugging (to stderr)
        print(f"Starting server: {' '.join(cmd)}", file=sys.stderr)
        print(f"Working directory: {working_dir}", file=sys.stderr)
        
        # Start process (detached, Windows-compatible)
        # CREATE_NEW_PROCESS_GROUP = 0x00000200
        # DETACHED_PROCESS = 0x00000008
        creationflags = 0
        if sys.platform == 'win32':
            creationflags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS
        
        proc = subprocess.Popen(
            cmd,
            cwd=working_dir,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            creationflags=creationflags
        )
        
        return {
            "success": True,
            "pid": proc.pid,
            "message": f"Palworld server starting with PID {proc.pid}"
        }
    except Exception as e:
        import traceback
        error_details = traceback.format_exc()
        print(f"Error details: {error_details}", file=sys.stderr)
        return {
            "success": False,
            "message": f"Failed to start: {str(e)}"
        }

def stop(config):
    """Stop Palworld server"""
    try:
        executable = config.get("server_executable")
        if not executable:
            return {"success": False, "message": "server_executable not specified"}
        
        # Extract executable name from path (e.g., "D:\\path\\PalServer.exe" -> "PalServer.exe")
        exe_name = os.path.basename(executable)
        force = config.get("force", False)
        
        if sys.platform == 'win32':
            # Windows: Always use /F /T for forceful termination with tree kill
            # /F = force kill, /T = terminate child processes
            try:
                result = subprocess.run(
                    ['taskkill', '/F', '/T', '/IM', exe_name],
                    capture_output=True,
                    text=True,
                    check=False
                )
                
                # Log the output for debugging
                if result.stdout:
                    print(f"taskkill stdout: {result.stdout}", file=sys.stderr)
                if result.stderr:
                    print(f"taskkill stderr: {result.stderr}", file=sys.stderr)
                print(f"taskkill return code: {result.returncode}", file=sys.stderr)
                
                # taskkill returns 0 on success, but can also return 128 if process not found
                # Return success in either case since the goal is to stop the server
                return {
                    "success": True,
                    "message": f"Terminated {exe_name}"
                }
            except Exception as e:
                error_msg = f"Failed to stop process: {str(e)}"
                print(error_msg, file=sys.stderr)
                return {
                    "success": False,
                    "message": error_msg
                }
        else:
            # Unix-like: Use pkill with force
            try:
                # Use SIGKILL (9) for immediate termination
                result = subprocess.run(
                    ['pkill', '-9', os.path.splitext(exe_name)[0]],
                    capture_output=True,
                    text=True,
                    check=False
                )
                
                if result.stdout:
                    print(f"pkill stdout: {result.stdout}", file=sys.stderr)
                if result.stderr:
                    print(f"pkill stderr: {result.stderr}", file=sys.stderr)
                print(f"pkill return code: {result.returncode}", file=sys.stderr)
                
                return {
                    "success": True,
                    "message": f"Terminated {exe_name}"
                }
            except Exception as e:
                error_msg = f"Failed to stop process: {str(e)}"
                print(error_msg, file=sys.stderr)
                return {
                    "success": False,
                    "message": error_msg
                }
    except Exception as e:
        return {
            "success": False,
            "message": f"Failed to stop: {str(e)}"
        }

def status(config):
    """Get server status"""
    try:
        executable = config.get("server_executable")
        if not executable:
            return {"success": True, "status": "stopped", "message": "No executable specified"}
        
        # Extract executable name from path (e.g., "D:\\path\\PalServer.exe" -> "PalServer.exe")
        exe_name = os.path.basename(executable)
        
        # Check if process is running by name
        if sys.platform == 'win32':
            try:
                # Use tasklist to check if process is running
                result = subprocess.run(
                    ['tasklist', '/FI', f'IMAGENAME eq {exe_name}'],
                    capture_output=True,
                    text=True,
                    check=False
                )
                if exe_name in result.stdout:
                    return {
                        "success": True,
                        "status": "running",
                        "message": f"{exe_name} is running"
                    }
                else:
                    return {
                        "success": True,
                        "status": "stopped",
                        "message": f"{exe_name} is not running"
                    }
            except Exception as e:
                return {
                    "success": True,
                    "status": "stopped",
                    "message": f"Could not determine status: {str(e)}"
                }
        else:
            # Unix-like: Use pgrep
            try:
                result = subprocess.run(
                    ['pgrep', '-f', os.path.splitext(exe_name)[0]],
                    capture_output=True,
                    check=False
                )
                if result.returncode == 0:
                    pid = result.stdout.decode().strip().split('\n')[0]
                    return {
                        "success": True,
                        "status": "running",
                        "pid": int(pid) if pid else None,
                        "message": f"{exe_name} is running"
                    }
                else:
                    return {
                        "success": True,
                        "status": "stopped",
                        "message": f"{exe_name} is not running"
                    }
            except Exception as e:
                return {
                    "success": True,
                    "status": "stopped",
                    "message": f"Could not determine status: {str(e)}"
                }
    except Exception as e:
        return {
            "success": False,
            "message": f"Failed to get status: {str(e)}"
        }

def command(config):
    """Execute server command via RCON or stdin"""
    try:
        command_text = config.get("command")
        args = config.get("args", {})
        pid = config.get("pid")
        
        if not command_text:
            return {
                "success": False,
                "message": "No command specified"
            }
        
        print(f"[Palworld] Executing command: {command_text} with args: {args}", file=sys.stderr)
        
        # Process special command formats (used for RCON fallback)
        formatted_command = command_text
        
        # Normalize once for branching
        command_lower = command_text.lower()
        
        # Special handlers for known commands with parameters (RCON formatting)
        if command_lower == "say":
            message = args.get("message", "")
            if not message:
                return {"success": False, "message": "Message parameter required"}
            formatted_command = f"say {message}"
        
        elif command_lower == "broadcast":
            message = args.get("message", "")
            if not message:
                return {"success": False, "message": "Message parameter required"}
            formatted_command = f"Broadcast {message.replace(' ', '_')}"
        
        elif command_lower == "shutdown":
            seconds = int(args.get("seconds", 10))
            formatted_command = f"Shutdown {seconds} Server_shutting_down"
        
        # REST API configuration
        rest_host = config.get("rest_host", "127.0.0.1")
        rest_port = config.get("rest_port", 8212)
        rest_username = config.get("rest_username", "")
        rest_password = config.get("rest_password", "")
        rest_client = PalworldRestClient(host=rest_host, port=rest_port, username=rest_username, password=rest_password)
        
        def execute_via_rest():
            try:
                if command_lower in ("broadcast", "announce", "say"):
                    message = args.get("message", "")
                    if not message:
                        return {"success": False, "message": "Message parameter required"}
                    rest_client.announce(message)
                    return {"success": True, "message": f"✅ Announced via REST: {message}"}
                
                if command_lower == "info":
                    data = rest_client.info()
                    return {"success": True, "message": "✅ Info fetched via REST", "data": data}
                
                if command_lower == "players":
                    data = rest_client.players()
                    return {"success": True, "message": "✅ Players fetched via REST", "data": data}
                
                if command_lower == "metrics":
                    data = rest_client.metrics()
                    return {"success": True, "message": "✅ Metrics fetched via REST", "data": data}
                
                if command_lower == "kick":
                    userid = args.get("userid") or args.get("player_id") or args.get("steam_id")
                    if not userid:
                        return {"success": False, "message": "userid (player id) is required"}
                    message = args.get("message")
                    rest_client.kick(userid, message)
                    return {"success": True, "message": f"✅ Kicked {userid} via REST"}
                
                if command_lower == "ban":
                    userid = args.get("userid") or args.get("player_id") or args.get("steam_id")
                    if not userid:
                        return {"success": False, "message": "userid (player id) is required"}
                    message = args.get("message")
                    rest_client.ban(userid, message)
                    return {"success": True, "message": f"✅ Banned {userid} via REST"}
                
                if command_lower == "unban":
                    userid = args.get("userid") or args.get("player_id") or args.get("steam_id")
                    if not userid:
                        return {"success": False, "message": "userid (player id) is required"}
                    rest_client.unban(userid)
                    return {"success": True, "message": f"✅ Unbanned {userid} via REST"}
                
                return None  # Not a REST-supported command
            except Exception as e:
                print(f"[REST] ❌ {e}", file=sys.stderr)
                return {"success": False, "message": f"REST error: {e}"}
        
        rest_result = execute_via_rest()
        if rest_result and rest_result.get("success"):
            return rest_result
        elif rest_result:
            print(f"[REST] Falling back to RCON after error: {rest_result.get('message')}", file=sys.stderr)
        
        # RCON fallback
        rcon_host = config.get("rcon_host", "127.0.0.1")
        rcon_port = config.get("rcon_port", 25575)
        rcon_password = config.get("rcon_password", "")
        
        print(f"[Palworld] Attempting RCON connection to {rcon_host}:{rcon_port}", file=sys.stderr)
        rcon = PalworldRconClient(host=rcon_host, port=rcon_port, password=rcon_password)
        
        if rcon.connect():
            try:
                print(f"[RCON] Sending: {formatted_command}", file=sys.stderr)
                response = rcon.send_command(formatted_command)
                rcon.disconnect()
                
                if response is not None:
                    print(f"[RCON] Response: {response}", file=sys.stderr)
                    return {
                        "success": True,
                        "message": f"✅ Command executed via RCON: {formatted_command}"
                    }
            except Exception as e:
                print(f"[RCON] RCON failed: {e}", file=sys.stderr)
                rcon.disconnect()
        else:
            print(f"[Palworld] RCON connection failed, command will be logged but not executed", file=sys.stderr)
        
        # If we reach here, log for reference
        print(f"[Palworld] Command logged: {formatted_command}", file=sys.stderr)
        return {
            "success": True,
            "message": f"Command acknowledged: {formatted_command} (Note: RCON not responding, command not transmitted)"
        }
    
    except Exception as e:
        import traceback
        error_details = traceback.format_exc()
        print(f"Error details: {error_details}", file=sys.stderr)
        return {
            "success": False,
            "message": f"Failed to execute command: {str(e)}"
        }


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(json.dumps({"success": False, "message": "Usage: lifecycle.py <function> <config_json>"}))
        sys.exit(1)
    
    function_name = sys.argv[1]
    config_json = sys.argv[2]
    
    try:
        config = json.loads(config_json)
    except:
        print(json.dumps({"success": False, "message": "Invalid JSON config"}))
        sys.exit(1)
    
    # Call function
    if function_name == "start":
        result = start(config)
    elif function_name == "stop":
        result = stop(config)
    elif function_name == "status":
        result = status(config)
    elif function_name == "command":
        result = command(config)
    else:
        result = {"success": False, "message": f"Unknown function: {function_name}"}
    
    # Output JSON only
    print(json.dumps(result))
