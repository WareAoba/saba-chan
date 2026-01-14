require('dotenv').config();
const { Client, GatewayIntentBits, Collection } = require('discord.js');
const axios = require('axios');

const client = new Client({ intents: [GatewayIntentBits.Guilds, GatewayIntentBits.MessageContent] });
const IPC_BASE = process.env.IPC_BASE || 'http://localhost:57474';

client.commands = new Collection();

// Command: /server list
client.on('interactionCreate', async (interaction) => {
    if (!interaction.isChatInputCommand()) return;

    try {
        if (interaction.commandName === 'server') {
            const subcommand = interaction.options.getSubcommand();
            const response = await axios.get(`${IPC_BASE}/api/server/${subcommand}`);
            await interaction.reply({ content: JSON.stringify(response.data, null, 2), ephemeral: true });
        }
    } catch (error) {
        await interaction.reply({ content: `Error: ${error.message}`, ephemeral: true });
    }
});

// Register commands
client.once('ready', () => {
    console.log(`Discord Bot logged in as ${client.user.tag}`);
    
    // Register slash commands (stub)
    const commands = [
        {
            name: 'server',
            description: 'Manage game servers',
            options: [
                { name: 'list', description: 'List all servers', type: 1 },
                { name: 'start', description: 'Start a server', type: 1 },
                { name: 'stop', description: 'Stop a server', type: 1 },
                { name: 'status', description: 'Get server status', type: 1 }
            ]
        }
    ];
    
    console.log('Discord Bot ready');
});

client.login(process.env.DISCORD_TOKEN);
