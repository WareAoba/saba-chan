// E2E용 가짜 마인크래프트 서버. 바닐라와 같은 로그 형식·EULA·server.properties·포트 바인드·콘솔 명령을 흉내 낸다.
// 실제 서버를 받을 수 없는 환경(클라우드)에서 데몬·러너·모듈의 실행 경로를 끝까지 돌리기 위한 것이다.
// 시험용 명령: e2e-crash(크래시 리포트 남기고 종료 1), e2e-alloc <MB>(메모리 점유), e2e-hang(stop 무시),
//              e2e-spam <N>(콘솔 N줄), e2e-exit <code>(그 코드로 즉시 종료).
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.time.LocalTime;
import java.time.format.DateTimeFormatter;
import java.util.*;

public class FakeMinecraft {
    static final DateTimeFormatter TS = DateTimeFormatter.ofPattern("HH:mm:ss");
    static final List<byte[]> HOLD = new ArrayList<>();
    static volatile boolean hang = false;
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);

    static void log(String thread, String level, String msg) {
        OUT.println("[" + LocalTime.now().format(TS) + "] [" + thread + "/" + level + "]: " + msg);
    }

    public static void main(String[] args) throws Exception {
        Path dir = Paths.get("").toAbsolutePath();
        String version = System.getProperty("fake.version", readVersion());
        log("ServerMain", "INFO", "Environment: Environment[sessionHost=https://sessionserver.mojang.com]");
        Path eula = dir.resolve("eula.txt");
        if (!Files.exists(eula) || !Files.readString(eula).replace(" ", "").contains("eula=true")) {
            if (!Files.exists(eula)) {
                Files.writeString(eula, "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\neula=false\n");
            }
            log("main", "WARN", "Failed to load eula.txt");
            log("main", "INFO", "You need to agree to the EULA in order to run the server. Go to eula.txt for more info.");
            System.exit(0);
        }
        Properties props = new Properties();
        Path propsPath = dir.resolve("server.properties");
        if (Files.exists(propsPath)) {
            try (Reader r = Files.newBufferedReader(propsPath)) { props.load(r); }
        } else {
            props.setProperty("server-port", "25565");
            props.setProperty("max-players", "20");
            props.setProperty("motd", "A Minecraft Server");
            try (Writer w = Files.newBufferedWriter(propsPath)) { props.store(w, "Minecraft server properties"); }
        }
        int port = Integer.parseInt(props.getProperty("server-port", "25565").trim());
        int maxPlayers = Integer.parseInt(props.getProperty("max-players", "20").trim());
        log("Server thread", "INFO", "Starting minecraft server version " + version);
        log("Server thread", "INFO", "Loading properties");
        log("Server thread", "INFO", "Default game type: SURVIVAL");
        log("Server thread", "INFO", "Starting Minecraft server on *:" + port);
        ServerSocket socket;
        try {
            socket = new ServerSocket();
            socket.bind(new InetSocketAddress(port));
        } catch (IOException e) {
            log("Server thread", "WARN", "**** FAILED TO BIND TO PORT!");
            log("Server thread", "WARN", "The exception was: " + e);
            System.exit(1);
            return;
        }
        Path world = dir.resolve(props.getProperty("level-name", "world"));
        Files.createDirectories(world);
        Path level = world.resolve("level.dat");
        if (!Files.exists(level)) Files.writeString(level, "fake-level " + UUID.randomUUID() + "\n");
        log("Server thread", "INFO", "Preparing level \"" + world.getFileName() + "\"");
        log("Worker-Main-1", "INFO", "Preparing spawn area: 100%");
        log("Server thread", "INFO", "Done (0." + (100 + new Random().nextInt(800)) + "s)! For help, type \"help\"");
        Thread acceptor = new Thread(() -> {
            while (true) {
                try { socket.accept().close(); } catch (IOException e) { return; }
            }
        });
        acceptor.setDaemon(true);
        acceptor.start();

        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;
        while ((line = in.readLine()) != null) {
            String cmd = line.trim();
            if (cmd.startsWith("/")) cmd = cmd.substring(1);
            if (cmd.isEmpty()) continue;
            String[] parts = cmd.split("\\s+", 2);
            String arg = parts.length > 1 ? parts[1] : "";
            switch (parts[0]) {
                case "stop":
                    if (hang) { log("Server thread", "INFO", "(e2e-hang) ignoring stop"); break; }
                    log("Server thread", "INFO", "Stopping the server");
                    log("Server thread", "INFO", "Stopping server");
                    log("Server thread", "INFO", "Saving players");
                    log("Server thread", "INFO", "Saving worlds");
                    Files.writeString(level, "fake-level saved " + System.currentTimeMillis() + "\n");
                    log("Server thread", "INFO", "ThreadedAnvilChunkStorage: All dimensions are saved");
                    socket.close();
                    System.exit(0);
                    break;
                case "list":
                    log("Server thread", "INFO", "There are 0 of a max of " + maxPlayers + " players online: ");
                    break;
                case "say":
                    log("Server thread", "INFO", "[Server] " + arg);
                    break;
                case "save-all":
                    Files.writeString(level, "fake-level saved " + System.currentTimeMillis() + "\n");
                    log("Server thread", "INFO", "Saving the game (this may take a moment!)");
                    log("Server thread", "INFO", "Saved the game");
                    break;
                case "save-off":
                    log("Server thread", "INFO", "Automatic saving is now disabled");
                    break;
                case "save-on":
                    log("Server thread", "INFO", "Automatic saving is now enabled");
                    break;
                case "help":
                    log("Server thread", "INFO", "/list /say /stop /save-all");
                    break;
                case "e2e-crash": {
                    Path reports = dir.resolve("crash-reports");
                    Files.createDirectories(reports);
                    Files.writeString(reports.resolve("crash-" + System.currentTimeMillis() + "-server.txt"),
                            "---- Minecraft Crash Report ----\nDescription: Exception in server tick loop\n\njava.lang.IllegalStateException: e2e crash\n");
                    log("Server thread", "ERROR", "Encountered an unexpected exception");
                    OUT.println("java.lang.IllegalStateException: e2e crash");
                    OUT.println("\tat net.minecraft.server.MinecraftServer.runServer(MinecraftServer.java:1)");
                    Runtime.getRuntime().halt(1);
                    break;
                }
                case "e2e-alloc": {
                    // 1 MiB 배열은 작은 G1 힙에서 humongous로 리전 두 개를 먹어 힙의 절반만 써도 OOM이 난다. 256 KiB씩 잡는다
                    int mb = Integer.parseInt(arg.trim());
                    for (int i = 0; i < mb * 4; i++) {
                        byte[] b = new byte[256 * 1024];
                        for (int j = 0; j < b.length; j += 4096) b[j] = 1;
                        HOLD.add(b);
                    }
                    log("Server thread", "INFO", "e2e-alloc holding " + HOLD.size() / 4 + " MiB");
                    break;
                }
                case "e2e-hang":
                    hang = true;
                    log("Server thread", "INFO", "e2e-hang on");
                    break;
                case "e2e-spam": {
                    int n = Integer.parseInt(arg.trim());
                    for (int i = 0; i < n; i++) log("Server thread", "INFO", "e2e-spam " + i);
                    break;
                }
                case "e2e-exit":
                    Runtime.getRuntime().halt(Integer.parseInt(arg.trim()));
                    break;
                default:
                    log("Server thread", "INFO", "Unknown or incomplete command, see below for error");
                    log("Server thread", "INFO", cmd + "<--[HERE]");
            }
        }
        // stdin이 닫혀도 실제 서버처럼 계속 돈다
        while (true) Thread.sleep(60_000);
    }

    static String readVersion() {
        try {
            return Files.readString(Paths.get("fake-version.txt")).trim();
        } catch (IOException e) {
            return "1.21.1";
        }
    }
}
