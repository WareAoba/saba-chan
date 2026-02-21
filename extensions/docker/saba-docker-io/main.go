// saba-docker-io — Bidirectional stdin/stdout bridge for Docker containers
//
// Usage:
//
//	saba-docker-io <container_name> [docker_path]
//
// This small Linux binary acts as a bridge between saba-chan's ManagedProcess
// and a Docker container. It lives at /opt/saba-chan/docker/saba-docker-io
// and is invoked from Windows via:
//
//	wsl -u root -- /opt/saba-chan/docker/saba-docker-io <container>
//
// Flow:
//  1. Fetch recent log history via `docker logs --tail 200` → print to stdout
//  2. exec() into `docker attach --sig-proxy=false` → bidirectional IO
//     stdin  flows from saba-chan daemon → container
//     stdout flows from container → saba-chan daemon
package main

import (
	"fmt"
	"os"
	"os/exec"
	"syscall"
)

const (
	defaultDocker  = "/opt/saba-chan/docker/docker"
	initialLogTail = "200"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "Usage: saba-docker-io <container_name> [docker_path]")
		os.Exit(1)
	}

	container := os.Args[1]
	docker := defaultDocker
	if len(os.Args) >= 3 {
		docker = os.Args[2]
	}

	// ── Phase 1: Fetch recent log history ───────────────────
	// Non-fatal: container may have no logs yet.
	logs := exec.Command(docker, "logs", "--tail", initialLogTail, "--timestamps", container)
	logs.Stdout = os.Stdout
	logs.Stderr = os.Stdout // merge container stderr into our stdout
	_ = logs.Run()

	// ── Phase 2: exec into docker attach (replaces this process) ──
	// Using syscall.Exec so this process becomes docker attach directly.
	// stdin/stdout/stderr are inherited automatically — zero-copy IO.
	dockerPath, err := exec.LookPath(docker)
	if err != nil {
		fmt.Fprintf(os.Stderr, "[saba-docker-io] docker not found at %s: %v\n", docker, err)
		os.Exit(2)
	}

	argv := []string{docker, "attach", "--sig-proxy=false", container}
	if err := syscall.Exec(dockerPath, argv, os.Environ()); err != nil {
		fmt.Fprintf(os.Stderr, "[saba-docker-io] exec failed: %v\n", err)
		os.Exit(3)
	}
}
