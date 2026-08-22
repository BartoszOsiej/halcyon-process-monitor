package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/gorilla/websocket"
)

// API response types
type ApiResponse struct {
	OK    bool            `json:"ok"`
	Data  json.RawMessage `json:"data"`
	Error string          `json:"error"`
}

type Stats struct {
	TotalEvents uint64 `json:"total_events"`
	TotalLost   uint64 `json:"total_lost"`
	UptimeSecs  uint64 `json:"uptime_secs"`
	ActivePIDs  int    `json:"active_pids"`
	Threshold   uint64 `json:"threshold"`
}

type ProcessInfo struct {
	PID        uint32            `json:"pid"`
	PPID       uint32            `json:"ppid"`
	Comm       string            `json:"comm"`
	TotalOpens uint64            `json:"total_opens"`
	TotalExecs uint64            `json:"total_execs"`
	Alerts     uint64            `json:"alerts"`
	Extensions map[string]uint64 `json:"extensions"`
}

type FileRank struct {
	Path      string  `json:"path"`
	Count     uint64  `json:"count"`
	Extension string  `json:"extension"`
	Entropy   float64 `json:"entropy"`
}

type WsEvent struct {
	Type      string `json:"type"`
	Ts        string `json:"ts"`
	Kind      string `json:"kind"`
	PID       uint32 `json:"pid"`
	UID       uint32 `json:"uid"`
	Comm      string `json:"comm"`
	File      string `json:"file"`
	Opens     uint64 `json:"opens_in_1s"`
}

var (
	baseURL   string
	jsonOutput bool
	watchMode  bool
)

func init() {
	flag.StringVar(&baseURL, "url", "http://localhost:8080", "Halcyon daemon URL")
	flag.BoolVar(&jsonOutput, "json", false, "JSON output")
	flag.BoolVar(&watchMode, "watch", false, "Watch mode (WebSocket)")
}

func main() {
	flag.Parse()

	if flag.NArg() == 0 {
		fmt.Fprintf(os.Stderr, "Usage: halcyon-agent <command> [options]\n\nCommands:\n  stats       Show global statistics\n  processes   List tracked processes\n  files       Show top opened files\n  extensions  Show file extension frequency\n  watch       Watch events in real-time (WebSocket)\n  tree        Show process tree\n  version     Show version\n\nOptions:\n")
		flag.PrintDefaults()
		os.Exit(1)
	}

	cmd := flag.Arg(0)
	switch cmd {
	case "stats":
		cmdStats()
	case "processes", "ps":
		cmdProcesses()
	case "files":
		cmdFiles()
	case "extensions", "ext":
		cmdExtensions()
	case "watch":
		cmdWatch()
	case "tree":
		cmdTree()
	case "version":
		fmt.Println("halcyon-agent v0.3.0")
	default:
		fmt.Fprintf(os.Stderr, "Unknown command: %s\n", cmd)
		os.Exit(1)
	}
}

func getJSON(path string, target interface{}) error {
	resp, err := http.Get(baseURL + path)
	if err != nil {
		return fmt.Errorf("connection failed: %w (is halcyon daemon running?)", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("read response: %w", err)
	}

	var apiResp ApiResponse
	if err := json.Unmarshal(body, &apiResp); err != nil {
		return fmt.Errorf("parse response: %w", err)
	}
	if !apiResp.OK {
		return fmt.Errorf("API error: %s", apiResp.Error)
	}
	if target != nil {
		return json.Unmarshal(apiResp.Data, target)
	}
	return nil
}

func cmdStats() {
	var stats Stats
	if err := getJSON("/api/v1/stats", &stats); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	if jsonOutput {
		json.NewEncoder(os.Stdout).Encode(stats)
	} else {
		fmt.Printf("📊 Halcyon eBPF Monitor\n")
		fmt.Printf("  Events:   %d\n", stats.TotalEvents)
		fmt.Printf("  Lost:     %d\n", stats.TotalLost)
		fmt.Printf("  Uptime:   %ds\n", stats.UptimeSecs)
		fmt.Printf("  PIDs:     %d\n", stats.ActivePIDs)
		fmt.Printf("  Threshold: %d/s\n", stats.Threshold)
	}
}

func cmdProcesses() {
	var procs []ProcessInfo
	if err := getJSON("/api/v1/processes", &procs); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	if jsonOutput {
		json.NewEncoder(os.Stdout).Encode(procs)
	} else {
		fmt.Printf("%-8s %-20s %8s %8s %6s\n", "PID", "COMM", "OPENS", "EXECS", "ALERTS")
		fmt.Println(strings.Repeat("─", 50))
		for _, p := range procs {
			alertStr := ""
			if p.Alerts > 0 {
				alertStr = fmt.Sprintf(" ⚠%d", p.Alerts)
			}
			fmt.Printf("%-8d %-20s %8d %8d %6d%s\n", p.PID, p.Comm, p.TotalOpens, p.TotalExecs, p.Alerts, alertStr)
		}
	}
}

func cmdFiles() {
	var files []FileRank
	if err := getJSON("/api/v1/files", &files); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	if jsonOutput {
		json.NewEncoder(os.Stdout).Encode(files)
	} else {
		fmt.Printf("%-3s %-40s %-5s %6s %6s\n", "#", "FILE", "EXT", "OPENS", "ENTR")
		fmt.Println(strings.Repeat("─", 65))
		for i, f := range files {
			path := f.Path
			if len(path) > 38 {
				path = "…" + path[len(path)-37:]
			}
			entropyColor := ""
			if f.Entropy > 0.7 {
				entropyColor = "\033[31m" // red
			} else if f.Entropy > 0.5 {
				entropyColor = "\033[33m" // yellow
			}
			reset := "\033[0m"
			fmt.Printf("%-3d %-40s .%-5s %6d %s%.2f%s\n", i+1, path, f.Extension, f.Count, entropyColor, f.Entropy, reset)
		}
	}
}

func cmdExtensions() {
	var exts []struct {
		Extension string `json:"extension"`
		Count     uint64 `json:"count"`
	}
	if err := getJSON("/api/v1/extensions", &exts); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	if jsonOutput {
		json.NewEncoder(os.Stdout).Encode(exts)
	} else {
		if len(exts) == 0 {
			fmt.Println("No extensions tracked yet.")
			return
		}
		maxCount := exts[0].Count
		fmt.Printf("%-10s %6s  %s\n", "EXT", "COUNT", "BAR")
		fmt.Println(strings.Repeat("─", 50))
		for _, e := range exts {
			barLen := int(float64(e.Count) / float64(maxCount) * 20)
			bar := strings.Repeat("█", barLen) + strings.Repeat("░", 20-barLen)
			fmt.Printf(".%-9s %6d  %s\n", e.Extension, e.Count, bar)
		}
	}
}

func cmdTree() {
	var tree []struct {
		Depth      int    `json:"depth"`
		PID        uint32 `json:"pid"`
		PPID       uint32 `json:"ppid"`
		Comm       string `json:"comm"`
		Alerts     uint64 `json:"alerts"`
		TotalOpens uint64 `json:"total_opens"`
	}
	if err := getJSON("/api/v1/process-tree", &tree); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	if jsonOutput {
		json.NewEncoder(os.Stdout).Encode(tree)
	} else {
		for _, n := range tree {
			indent := strings.Repeat("  ", n.Depth)
			prefix := "├─ "
			if n.Depth == 0 {
				prefix = ""
			}
			alertBadge := ""
			if n.Alerts > 0 {
				alertBadge = fmt.Sprintf(" ⚠%d", n.Alerts)
			}
			opensBadge := ""
			if n.TotalOpens > 0 {
				opensBadge = fmt.Sprintf(" (%d)", n.TotalOpens)
			}
			fmt.Printf("%s%s%s [%d]%s%s\n", indent, prefix, n.Comm, n.PID, opensBadge, alertBadge)
		}
	}
}

func cmdWatch() {
	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt, syscall.SIGTERM)

	wsURL := strings.Replace(baseURL, "http://", "ws://", 1)
	wsURL = strings.Replace(wsURL, "https://", "wss://", 1)
	wsURL += "/ws"

	fmt.Fprintf(os.Stderr, "Connecting to %s...\n", wsURL)

	conn, _, err := websocket.DefaultDialer.Dial(wsURL, nil)
	if err != nil {
		fmt.Fprintf(os.Stderr, "WebSocket connection failed: %v\n", err)
		fmt.Fprintf(os.Stderr, "Is halcyon daemon running with --web flag?\n")
		os.Exit(1)
	}
	defer conn.Close()

	fmt.Fprintf(os.Stderr, "Connected! Watching events...\n\n")

	done := make(chan struct{})

	go func() {
		defer close(done)
		for {
			_, message, err := conn.ReadMessage()
			if err != nil {
				return
			}
			var event WsEvent
			if err := json.Unmarshal(message, &event); err != nil {
				continue
			}
			printEvent(event)
		}
	}()

	for {
		select {
		case <-done:
			return
		case <-interrupt:
			fmt.Fprintf(os.Stderr, "\nDisconnecting...\n")
			conn.WriteMessage(websocket.CloseMessage,
				websocket.FormatCloseMessage(websocket.CloseNormalClosure, ""))
			time.Sleep(time.Second)
			return
		}
	}
}

func printEvent(ev WsEvent) {
	switch ev.Type {
	case "event":
		tag := "\033[32mEXEC\033[0m" // green
		if ev.Kind == "Open" {
			tag = "\033[36mOPEN\033[0m" // cyan
		}
		file := ""
		if ev.File != "" {
			file = " → " + ev.File
		}
		fmt.Printf("%s %s [%d] %s%s\n", ev.Ts, tag, ev.PID, ev.Comm, file)
	case "alert":
		fmt.Printf("%s \033[31m⚠ ALERT\033[0m [%d] %s — %d opens/s\n", ev.Ts, ev.PID, ev.Comm, ev.Opens)
	}
}
