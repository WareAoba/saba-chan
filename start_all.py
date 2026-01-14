#!/usr/bin/env python3
"""
All-in-one starter script for Game Server Management Platform
Cross-platform: Works on Windows, macOS, and Linux

Usage:
    python start_all.py [mode] [options]

Modes:
    all    - Start all services (default)
    daemon - Start only Core Daemon
    bot    - Start only Discord Bot
    gui    - Start only Electron GUI

Options:
    --log-level LEVEL   - Set log level (debug, info, warn, error)
    --no-wait          - Don't wait for user input to exit
"""

import os
import sys
import subprocess
import time
import platform
import argparse
from pathlib import Path

# Colors
class Colors:
    BLUE = '\033[94m'
    GREEN = '\033[92m'
    YELLOW = '\033[93m'
    RED = '\033[91m'
    ENDC = '\033[0m'
    BOLD = '\033[1m'
    
    @staticmethod
    def disable():
        """Disable colors on Windows cmd.exe"""
        if platform.system() == 'Windows':
            Colors.BLUE = ''
            Colors.GREEN = ''
            Colors.YELLOW = ''
            Colors.RED = ''
            Colors.ENDC = ''
            Colors.BOLD = ''

class ProcessManager:
    def __init__(self, script_dir):
        self.script_dir = Path(script_dir)
        self.processes = []
        self.os_type = platform.system()
        
        # Disable colors on Windows
        if self.os_type == 'Windows':
            Colors.disable()
    
    def print_header(self):
        """Print header"""
        print()
        print(f"{Colors.BLUE}{Colors.BOLD}╔════════════════════════════════════════════════════════════╗{Colors.ENDC}")
        print(f"{Colors.BLUE}{Colors.BOLD}║  Game Server Management Platform - Development Mode       ║{Colors.ENDC}")
        print(f"{Colors.BLUE}{Colors.BOLD}║  모듈형 통합 디스코드 봇 & 게임 서버 관리 플랫폼            ║{Colors.ENDC}")
        print(f"{Colors.BLUE}{Colors.BOLD}╚════════════════════════════════════════════════════════════╝{Colors.ENDC}")
        print()
    
    def start_daemon(self):
        """Start Core Daemon"""
        print(f"{Colors.BLUE}[1/3] Starting Core Daemon (Rust){Colors.ENDC}")
        print("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
        
        # Check Cargo.toml
        if not (self.script_dir / "Cargo.toml").exists():
            print(f"{Colors.RED}✗ Error: Cargo.toml not found{Colors.ENDC}")
            return False
        
        # Create modules directory
        (self.script_dir / "modules").mkdir(exist_ok=True)
        
        # Check cargo
        try:
            subprocess.run(["cargo", "--version"], capture_output=True, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError):
            print(f"{Colors.RED}✗ Rust/cargo not found. Install from https://rustup.rs{Colors.ENDC}")
            return False
        
        try:
            # Build first
            subprocess.run(
                ["cargo", "build", "--quiet"],
                cwd=self.script_dir,
                capture_output=True
            )
            
            # Start daemon
            if self.os_type == 'Windows':
                p = subprocess.Popen(
                    ["cargo", "run"],
                    cwd=self.script_dir,
                    creationflags=subprocess.CREATE_NEW_CONSOLE
                )
            else:
                p = subprocess.Popen(
                    ["cargo", "run"],
                    cwd=self.script_dir
                )
            
            self.processes.append(("Core Daemon", p))
            print(f"{Colors.GREEN}✓ Core Daemon started (PID: {p.pid}){Colors.ENDC}")
            print()
            return True
        except Exception as e:
            print(f"{Colors.RED}✗ Error starting Core Daemon: {e}{Colors.ENDC}")
            return False
    
    def start_bot(self):
        """Start Discord Bot"""
        print(f"{Colors.BLUE}[2/3] Starting Discord Bot (Node.js){Colors.ENDC}")
        print("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
        
        bot_dir = self.script_dir / "discord_bot"
        
        # Check package.json
        if not (bot_dir / "package.json").exists():
            print(f"{Colors.RED}✗ Error: discord_bot/package.json not found{Colors.ENDC}")
            return False
        
        # Check/create .env
        env_file = bot_dir / ".env"
        if not env_file.exists():
            example_file = bot_dir / ".env.example"
            if example_file.exists():
                import shutil
                shutil.copy(example_file, env_file)
                print(f"{Colors.YELLOW}⚠ .env created from .env.example{Colors.ENDC}")
                print(f"{Colors.YELLOW}  ⚠ Please set DISCORD_TOKEN in .env{Colors.ENDC}")
                print(f"{Colors.YELLOW}  Get token from: https://discord.com/developers/applications{Colors.ENDC}")
            else:
                print(f"{Colors.RED}✗ .env file not found{Colors.ENDC}")
                return False
        
        # Check npm
        try:
            subprocess.run(["npm", "--version"], capture_output=True, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError):
            print(f"{Colors.RED}✗ Node.js/npm not found{Colors.ENDC}")
            return False
        
        try:
            # Install dependencies if needed
            node_modules = bot_dir / "node_modules"
            if not node_modules.exists():
                print("Installing npm dependencies...")
                subprocess.run(
                    ["npm", "install", "--silent"],
                    cwd=bot_dir,
                    check=True
                )
            
            # Start bot
            if self.os_type == 'Windows':
                p = subprocess.Popen(
                    ["npm", "start"],
                    cwd=bot_dir,
                    creationflags=subprocess.CREATE_NEW_CONSOLE
                )
            else:
                p = subprocess.Popen(
                    ["npm", "start"],
                    cwd=bot_dir
                )
            
            self.processes.append(("Discord Bot", p))
            print(f"{Colors.GREEN}✓ Discord Bot started (PID: {p.pid}){Colors.ENDC}")
            print()
            return True
        except Exception as e:
            print(f"{Colors.RED}✗ Error starting Discord Bot: {e}{Colors.ENDC}")
            return False
    
    def start_gui(self):
        """Start Electron GUI"""
        print(f"{Colors.BLUE}[3/3] Starting Electron GUI (React){Colors.ENDC}")
        print("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
        
        gui_dir = self.script_dir / "electron_gui"
        
        # Check package.json
        if not (gui_dir / "package.json").exists():
            print(f"{Colors.RED}✗ Error: electron_gui/package.json not found{Colors.ENDC}")
            return False
        
        # Check npm
        try:
            subprocess.run(["npm", "--version"], capture_output=True, check=True)
        except (subprocess.CalledProcessError, FileNotFoundError):
            print(f"{Colors.RED}✗ Node.js/npm not found{Colors.ENDC}")
            return False
        
        try:
            # Install dependencies if needed
            node_modules = gui_dir / "node_modules"
            if not node_modules.exists():
                print("Installing npm dependencies...")
                subprocess.run(
                    ["npm", "install", "--silent"],
                    cwd=gui_dir,
                    check=True
                )
            
            # Start GUI
            if self.os_type == 'Windows':
                p = subprocess.Popen(
                    ["npm", "start"],
                    cwd=gui_dir,
                    creationflags=subprocess.CREATE_NEW_CONSOLE
                )
            else:
                p = subprocess.Popen(
                    ["npm", "start"],
                    cwd=gui_dir
                )
            
            self.processes.append(("Electron GUI", p))
            print(f"{Colors.GREEN}✓ Electron GUI started (PID: {p.pid}){Colors.ENDC}")
            print()
            return True
        except Exception as e:
            print(f"{Colors.RED}✗ Error starting Electron GUI: {e}{Colors.ENDC}")
            return False
    
    def print_summary(self):
        """Print startup summary"""
        print(f"{Colors.GREEN}{Colors.BOLD}╔════════════════════════════════════════════════════════════╗{Colors.ENDC}")
        print(f"{Colors.GREEN}{Colors.BOLD}║           All services are running!                       ║{Colors.ENDC}")
        print(f"{Colors.GREEN}{Colors.BOLD}╠════════════════════════════════════════════════════════════╣{Colors.ENDC}")
        print(f"{Colors.GREEN}║  Core Daemon:   http://localhost:57474                   ║{Colors.ENDC}")
        print(f"{Colors.GREEN}║  React Dev:     http://localhost:3000                    ║{Colors.ENDC}")
        print(f"{Colors.GREEN}║  Discord Bot:   Connected (check terminal)               ║{Colors.ENDC}")
        print(f"{Colors.GREEN}{Colors.BOLD}╠════════════════════════════════════════════════════════════╣{Colors.ENDC}")
        
        if self.os_type == 'Windows':
            print(f"{Colors.GREEN}║  Press Ctrl+C in each terminal to stop services          ║{Colors.ENDC}")
        else:
            print(f"{Colors.GREEN}║  Press Ctrl+C to stop all services                       ║{Colors.ENDC}")
        
        print(f"{Colors.GREEN}{Colors.BOLD}╚════════════════════════════════════════════════════════════╝{Colors.ENDC}")
        print()
    
    def wait_for_processes(self):
        """Wait for all processes"""
        try:
            if self.os_type == 'Windows':
                # On Windows, processes are in new windows, just wait
                while True:
                    time.sleep(1)
            else:
                # On Unix, wait for first process to finish
                for name, p in self.processes:
                    p.wait()
        except KeyboardInterrupt:
            print()
            print(f"{Colors.YELLOW}Shutting down all processes...{Colors.ENDC}")
            self.cleanup()
            print(f"{Colors.GREEN}✓ All processes stopped{Colors.ENDC}")
    
    def cleanup(self):
        """Stop all processes"""
        for name, p in self.processes:
            try:
                if self.os_type == 'Windows':
                    # Windows
                    import subprocess
                    subprocess.run(['taskkill', '/F', '/PID', str(p.pid)], 
                                 capture_output=True)
                else:
                    # Unix
                    p.terminate()
                    try:
                        p.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        p.kill()
            except Exception as e:
                pass

def main():
    parser = argparse.ArgumentParser(
        description='All-in-one starter for Game Server Management Platform'
    )
    parser.add_argument(
        'mode',
        nargs='?',
        default='all',
        choices=['all', 'daemon', 'bot', 'gui'],
        help='Which services to start'
    )
    parser.add_argument(
        '--log-level',
        default='info',
        choices=['debug', 'info', 'warn', 'error'],
        help='Log level for Core Daemon'
    )
    parser.add_argument(
        '--no-wait',
        action='store_true',
        help="Don't wait for processes (useful in CI)"
    )
    
    args = parser.parse_args()
    
    script_dir = Path(__file__).parent
    manager = ProcessManager(script_dir)
    
    manager.print_header()
    print(f"Mode: {args.mode}")
    print()
    
    # Set environment variable for log level
    os.environ['RUST_LOG'] = args.log_level
    
    # Start services based on mode
    success = True
    if args.mode in ['all', 'daemon']:
        if not manager.start_daemon():
            success = False
        else:
            time.sleep(2)  # Wait for daemon to start
    
    if args.mode in ['all', 'bot'] and success:
        if not manager.start_bot():
            success = False
        else:
            time.sleep(1)
    
    if args.mode in ['all', 'gui'] and success:
        if not manager.start_gui():
            success = False
        else:
            time.sleep(1)
    
    if success and len(manager.processes) > 0:
        manager.print_summary()
        
        if not args.no_wait:
            manager.wait_for_processes()
    elif not success:
        sys.exit(1)

if __name__ == '__main__':
    main()
