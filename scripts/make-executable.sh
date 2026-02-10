#!/bin/bash
# This file contains instructions for making scripts executable on Unix-like systems
# Run these commands to enable the shell scripts:

# Make startup scripts executable:
chmod +x start-all.sh
chmod +x run-daemon.sh
chmod +x discord_bot/run-bot.sh
chmod +x saba-chan-gui/run-gui.sh

echo "✓ All scripts are now executable"
echo ""
echo "You can now run:"
echo "  ./start-all.sh              # Start all services"
echo "  python start_all.py         # Cross-platform starter"
echo ""
