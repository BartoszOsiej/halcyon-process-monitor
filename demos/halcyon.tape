Output assets/halcyon-demo.svg

Set FontSize 16
Set Width 960
Set Height 480
Set Shell bash
Set MarginColor "#0d1117"
Set Theme {
  "name": "halcyon",
  "black": "#0d1117",
  "red": "#f85149",
  "green": "#3fb950",
  "yellow": "#e3b341",
  "blue": "#58a6ff",
  "magenta": "#a371f7",
  "cyan": "#39c5cf",
  "white": "#e6edf3",
  "brightBlack": "#8b949e"
}

Type "process-monitor --help"
Enter
Sleep 600ms

Type "process-monitor --web --port 9090"
Enter
Sleep 1s

Type "curl -s localhost:9090/api/events | jq '.[] | {pid, comm, event}'"
Enter
Sleep 1s

Type "curl -s localhost:9090/api/stats | jq '.total_events, .processes_tracked'"
Enter
Sleep 1s

Type "# eBPF kernel tracing · zero-copy · real-time"
Enter
Sleep 500ms
