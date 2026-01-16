# RCON Implementation Test

## RCON System Status: ✅ IMPLEMENTED

### What Changed
1. **Palworld (lifecycle.py)**
   - ✅ Added `RconClient` class with socket-based RCON protocol
   - ✅ Implements Minecraft RCON protocol (compatible with Palworld)
   - ✅ Authentication with RCON password
   - ✅ Command transmission with response parsing
   - ✅ Connection error handling

2. **Minecraft (lifecycle.py)**
   - ✅ Added `MinecraftProcessPipe` helper class
   - ✅ PID-based process communication setup
   - ✅ Command formatting for process pipe

3. **Backend (src/supervisor/mod.rs)**
   - ✅ Added RCON settings to command config:
     - `rcon_host` (127.0.0.1)
     - `rcon_port` (from instance.rcon_port)
     - `rcon_password` (from instance.rcon_password)
     - `pid` (from ProcessTracker)

### Testing RCON Connectivity

**Prerequisites:**
- Palworld server running and listening on RCON port (default: 25575)
- Palworld RCON password configured in instance settings
- Backend daemon (core_daemon) running on port 57474

**Test 1: Say Command**
```bash
curl -X POST http://127.0.0.1:57474/api/instance/{instance-id}/command \
  -H "Content-Type: application/json" \
  -d '{
    "command": "say",
    "args": {"message": "Hello from API!"}
  }'
```

Expected Response:
```json
{
  "success": true,
  "message": "Command sent: say Hello from API!"
}
```

**Test 2: Raw RCON Command**
```bash
curl -X POST http://127.0.0.1:57474/api/instance/{instance-id}/command \
  -H "Content-Type: application/json" \
  -d '{
    "command": "/adminpassword 8434"
  }'
```

**Test 3: Broadcast Command**
```bash
curl -X POST http://127.0.0.1:57474/api/instance/{instance-id}/command \
  -H "Content-Type: application/json" \
  -d '{
    "command": "broadcast",
    "args": {"message": "Server message"}
  }'
```

### GUI Testing

1. Start a Palworld server (should be running)
2. Open GUI at http://localhost:3000
3. Navigate to server card
4. Click "💻 Command" button
5. Enter command in CLI input (e.g., `/say Test message`)
6. Press Enter or click Execute
7. Check Palworld server console for command output

### Expected Behavior

**Success Flow:**
1. GUI input → API request
2. API → supervisor.execute_command()
3. Supervisor → Python module via plugin runner
4. Python module → RconClient socket connection
5. RCON → Game server
6. Response back to GUI

**Error Scenarios:**
- Connection refused: RCON port not accessible
- Authentication failed: Wrong RCON password
- Command transmission failed: Socket error

### Daemon Console Output Example

When command succeeds:
```
[RCON] Sending: say Test message
[RCON] Response: §cYour message
[Palworld] Executed command: say Test message with args: {"message": "Test message"}
```

When connection fails:
```
[RCON] Connection failed: Connection refused (os error 111)
```

### Known Limitations

1. **Minecraft STDIN**: Currently set to log-only mode due to process pipe complexity
   - Can be enhanced with direct process STDIN injection
   - Requires elevated permissions on some systems

2. **Command Output Display**: Currently shows brief response in modal
   - Full server console streaming not yet implemented
   - Can be added in future phase

3. **Connection Pooling**: New socket created per command
   - Could be optimized with persistent connections
   - Current approach is simpler and works for low command frequency

### Architecture Summary

```
GUI (Terminal Input)
  ↓
API POST /api/instance/{id}/command
  ↓
IPC Handler (execute_command)
  ↓
Supervisor::execute_command()
  ├─ Get instance (RCON settings)
  ├─ Get ProcessTracker PID
  └─ Call Python module with RCON config
    ↓
Python lifecycle.py::command()
  ├─ Create RconClient
  ├─ Connect to RCON (rcon_port, rcon_password)
  ├─ Send command via RCON protocol
  └─ Return response
    ↓
Response Modal (SuccessModal/FailureModal)
```

### Build Status: ✅ SUCCESS

- Backend compiled: `cargo build --release`
- No compilation errors
- Only deprecation warnings (non-critical)
- Ready for testing
