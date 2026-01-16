#!/usr/bin/env python3
"""
Simple RCON connection test for debugging
"""

import socket
import struct

class TestRconClient:
    def __init__(self, host='127.0.0.1', port=25575, password=''):
        self.host = host
        self.port = int(port)
        self.password = password
        self.socket = None
        self.request_id = 0
    
    def connect(self):
        """Connect and authenticate"""
        try:
            print(f"[TEST] Connecting to {self.host}:{self.port}...")
            self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.socket.settimeout(5)
            self.socket.connect((self.host, self.port))
            print("[TEST] ✅ Connected!")
            
            # Authenticate if password provided
            if self.password:
                print(f"[TEST] Authenticating with password...")
                response = self._send_command(3, self.password)
                if response is not None:
                    print(f"[TEST] ✅ Authenticated! Response: {response[:50]}")
                else:
                    print("[TEST] ❌ Authentication response empty")
            
            return True
        except Exception as e:
            print(f"[TEST] ❌ Connection failed: {e}")
            return False
    
    def test_command(self, command):
        """Test sending a command"""
        try:
            print(f"\n[TEST] Sending command: {command}")
            response = self._send_command(2, command)
            if response:
                print(f"[TEST] ✅ Response: {response[:200]}")
                return True
            else:
                print("[TEST] ⚠️  No response (might be normal for some commands)")
                return True
        except Exception as e:
            print(f"[TEST] ❌ Command failed: {e}")
            return False
    
    def _send_command(self, cmd_type, command):
        """Internal RCON packet handler"""
        self.request_id += 1
        
        # Build packet
        body = command.encode('utf-8')
        packet = (
            self.request_id.to_bytes(4, byteorder='little') +
            cmd_type.to_bytes(4, byteorder='little') +
            body + b'\x00\x00'
        )
        
        # Send with length prefix
        length = len(packet).to_bytes(4, byteorder='little')
        self.socket.sendall(length + packet)
        
        # Receive response
        try:
            response_length_data = self.socket.recv(4)
            if not response_length_data:
                return None
            
            response_length = int.from_bytes(response_length_data, byteorder='little')
            response_data = self.socket.recv(response_length)
            
            if len(response_data) >= 12:
                response_body = response_data[8:-2].decode('utf-8', errors='ignore')
                return response_body
        except socket.timeout:
            print("[TEST] ⚠️  Response timeout")
            return None
        
        return None
    
    def disconnect(self):
        """Cleanup"""
        if self.socket:
            self.socket.close()


if __name__ == '__main__':
    print("=" * 60)
    print("RCON CONNECTION TEST")
    print("=" * 60)
    
    # Test with default Palworld settings
    client = TestRconClient(host='127.0.0.1', port=25575, password='')
    
    if client.connect():
        # Test basic commands
        client.test_command('Info')
        client.test_command('Save')
        client.test_command('say Hello from test')
        
        client.disconnect()
        print("\n[TEST] ✅ All tests completed")
    else:
        print("\n[TEST] ❌ Connection failed - RCON server not accessible")
        print("       Check: Is Palworld server running?")
        print("       Check: RCON port 25575 open?")
        print("       Check: Any firewall blocking?")
