#!/usr/bin/env python3
"""
Minecraft server lifecycle management module.
Always outputs JSON to stdout, logs to stderr.
"""

import sys
import json
import subprocess
import os

def start(config):
    """Start Minecraft server"""
    try:
        java_path = config.get("java_path", "java")
        server_jar = config.get("server_jar", "server.jar")
        ram = config.get("ram", "8G")
        
        # Construct command
        cmd = [
            java_path,
            f"-Xmx{ram}",
            f"-Xms{ram}",
            "-jar", server_jar,
            "nogui"
        ]
        
        # Start process (detached)
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        
        return {
            "success": True,
            "pid": proc.pid,
            "message": f"Minecraft server starting with PID {proc.pid}"
        }
    except Exception as e:
        return {
            "success": False,
            "message": f"Failed to start: {str(e)}"
        }

def stop(config):
    """Stop Minecraft server"""
    try:
        pid = config.get("pid")
        if not pid:
            return {"success": False, "message": "No PID provided"}
        
        # Send SIGTERM
        os.kill(pid, 15)
        return {
            "success": True,
            "message": f"Sent SIGTERM to PID {pid}"
        }
    except Exception as e:
        return {
            "success": False,
            "message": f"Failed to stop: {str(e)}"
        }

def status(config):
    """Get server status"""
    try:
        pid = config.get("pid")
        if not pid:
            return {"success": True, "status": "stopped", "message": "No process running"}
        
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
    else:
        result = {"success": False, "message": f"Unknown function: {function_name}"}
    
    # Output JSON only
    print(json.dumps(result))
