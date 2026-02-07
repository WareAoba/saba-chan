#!/usr/bin/env python3
"""
Minecraft server lifecycle management module.
Always outputs JSON to stdout, logs to stderr.
"""

import sys
import json
import subprocess
import os
import psutil
import urllib.request
import urllib.error
import urllib.parse
from i18n import I18n

# Initialize i18n
MODULE_DIR = os.path.dirname(os.path.abspath(__file__))
i18n = I18n(MODULE_DIR)

# Daemon API endpoint (localhost by default)
DAEMON_API_URL = os.environ.get('DAEMON_API_URL', 'http://127.0.0.1:57474')

class MinecraftProcessPipe:
    """Helper class to communicate with Minecraft server process via STDIN/STDOUT"""
    def __init__(self, pid):
        self.pid = pid
        self.process = None
    
    def connect(self):
        """Get reference to running process"""
        try:
            self.process = psutil.Process(self.pid)
            # Verify process is running
            if self.process.status() == psutil.STATUS_RUNNING:
                return True
            return False
        except psutil.NoSuchProcess:
            return False
    
    def send_command(self, command):
        """Send command to server process via STDIN"""
        try:
            if not self.process or self.process.status() != psutil.STATUS_RUNNING:
                return False
            
            # Try to write to STDIN (may not work on Windows for all processes)
            # This is a simplified approach - actual Minecraft server integration may require:
            # - Direct socket connection to query port
            # - Reading from server log files
            # - Using rcon4j or similar library
            
            print(f"[Minecraft] {i18n.t('messages.would_send_command', pid=self.pid, command=command)}", file=sys.stderr)
            return True
        except Exception as e:
            print(f"[Minecraft] {i18n.t('messages.failed_send_command', error=str(e))}", file=sys.stderr)
            return False

def start(config):
    """Start Minecraft server"""
    try:
        java_path = config.get("java_path", "java")
        server_jar = config.get("server_jar")
        
        if not server_jar:
            return {
                "success": False,
                "message": i18n.t('errors.server_jar_not_specified')
            }
        
        # Check if jar exists
        if not os.path.exists(server_jar):
            return {
                "success": False,
                "message": i18n.t('errors.server_jar_not_found', path=server_jar)
            }
        
        ram = config.get("ram", "8G")
        working_dir = config.get("working_dir")
        
        # Use working directory if specified, otherwise use jar's directory
        if not working_dir:
            working_dir = os.path.dirname(server_jar)
        
        # Construct command
        cmd = [
            java_path,
            f"-Xmx{ram}",
            f"-Xms{ram}",
            "-jar", server_jar,
            "nogui"
        ]
        
        # Log for debugging
        print(i18n.t('messages.starting_server', command=' '.join(cmd)), file=sys.stderr)
        print(i18n.t('messages.working_directory', path=working_dir), file=sys.stderr)
        
        # Start process (detached, cross-platform)
        if sys.platform == 'win32':
            creationflags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS
            proc = subprocess.Popen(
                cmd,
                cwd=working_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                creationflags=creationflags
            )
        else:
            # Unix/Linux/macOS: Use start_new_session for detached process
            proc = subprocess.Popen(
                cmd,
                cwd=working_dir,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True
            )
        
        return {
            "success": True,
            "pid": proc.pid,
            "message": i18n.t('messages.server_starting', pid=proc.pid)
        }
    except Exception as e:
        import traceback
        error_details = traceback.format_exc()
        print(f"Error details: {error_details}", file=sys.stderr)
        return {
            "success": False,
            "message": i18n.t('errors.failed_to_start', error=str(e))
        }

def stop(config):
    """Stop Minecraft server"""
    try:
        pid = config.get("pid")
        if not pid:
            return {"success": False, "message": i18n.t('errors.no_pid_provided')}
        
        force = config.get("force", False)
        
        if sys.platform == 'win32':
            # Windows: Use taskkill
            try:
                if not force:
                    subprocess.run(['taskkill', '/PID', str(pid)], check=True)
                    return {
                        "success": True,
                        "message": i18n.t('messages.shutdown_signal_sent', pid=pid)
                    }
                else:
                    subprocess.run(['taskkill', '/F', '/PID', str(pid)], check=True)
                    return {
                        "success": True,
                        "message": i18n.t('messages.force_killed', pid=pid)
                    }
            except subprocess.CalledProcessError as e:
                return {
                    "success": False,
                    "message": i18n.t('errors.failed_to_kill', error=str(e))
                }
        else:
            # Unix-like: Use os.kill
            signal_num = 9 if force else 15
            os.kill(pid, signal_num)
            return {
                "success": True,
                "message": i18n.t('messages.signal_sent', signal=signal_num, pid=pid)
            }
    except Exception as e:
        return {
            "success": False,
            "message": i18n.t('errors.failed_to_stop', error=str(e))
        }

def status(config):
    """Get server status"""
    try:
        pid = config.get("pid")
        if not pid:
            return {"success": True, "status": "stopped", "message": i18n.t('messages.no_process_running')}
        
        # Check if process exists
        try:
            os.kill(pid, 0)  # Signal 0: check existence
            return {
                "success": True,
                "status": "running",
                "pid": pid,
                "message": "Server is running"
            }
        except:
            return {
                "success": True,
                "status": "stopped",
                "message": "Process not found"
            }
    except Exception as e:
        return {
            "success": False,
            "message": f"Failed to get status: {str(e)}"
        }

def command(config):
    """Execute server command via daemon RCON API"""
    try:
        command_text = config.get("command")
        args = config.get("args", {})
        instance_id = config.get("instance_id")
        
        if not command_text:
            return {
                "success": False,
                "message": "No command specified"
            }
        
        if not instance_id:
            return {
                "success": False,
                "message": "No instance_id specified"
            }
        
        print(f"[Minecraft] {i18n.t('messages.executing_command', command=command_text, args=str(args))}", file=sys.stderr)
        
        # Format command text
        formatted_command = command_text
        
        # Special handlers for known commands with parameters
        if command_text == "say":
            message = args.get("message", "")
            if not message:
                return {"success": False, "message": "Message parameter required"}
            formatted_command = f"say {message}"
        
        elif command_text == "give":
            player = args.get("player", "")
            item = args.get("item", "")
            amount = int(args.get("amount", 1))
            if not player or not item:
                return {"success": False, "message": "Player and item parameters required"}
            formatted_command = f"give {player} {item} {amount}"
        
        elif command_text == "save-all":
            formatted_command = "save-all"
        
        elif command_text == "list":
            formatted_command = "list"
        
        elif command_text == "weather":
            weather_type = args.get("type", "clear")
            duration = int(args.get("duration", 1000))
            formatted_command = f"weather {weather_type} {duration}"
        
        elif command_text == "difficulty":
            level = args.get("level", "normal")
            formatted_command = f"difficulty {level}"
        
        # Call daemon RCON API
        api_url = f"{DAEMON_API_URL}/api/instance/{instance_id}/rcon"
        payload = json.dumps({
            "command": formatted_command
        }).encode('utf-8')
        
        try:
            req = urllib.request.Request(
                api_url,
                data=payload,
                headers={'Content-Type': 'application/json'},
                method='POST'
            )
            
            with urllib.request.urlopen(req, timeout=5) as response:
                result = json.loads(response.read().decode('utf-8'))
                print(f"[Minecraft] {i18n.t('messages.daemon_rcon_response', result=str(result))}", file=sys.stderr)
                
                return {
                    "success": result.get("success", True),
                    "message": f"RCON command executed: {formatted_command}"
                }
        
        except urllib.error.URLError as e:
            print(f"[Minecraft] {i18n.t('errors.daemon_connection_error', error=str(e))}", file=sys.stderr)
            return {
                "success": False,
                "message": i18n.t('errors.daemon_connection_error', error=str(e))
            }
        except json.JSONDecodeError as e:
            print(f"[Minecraft] {i18n.t('errors.invalid_json_response', error=str(e))}", file=sys.stderr)
            return {
                "success": False,
                "message": i18n.t('errors.invalid_json_response', error=str(e))
            }
        except Exception as e:
            print(f"[Minecraft] {i18n.t('errors.daemon_error', error=str(e))}", file=sys.stderr)
            return {
                "success": False,
                "message": i18n.t('errors.daemon_error', error=str(e))
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
