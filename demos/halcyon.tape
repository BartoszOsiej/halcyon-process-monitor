Output assets/halcyon-demo.gif

Set FontSize 16
Set Width 960
Set Height 480
Set Shell bash
Set Theme "Dracula"

Type "process-monitor --help"
Enter
Sleep 600ms
Type "eBPF process monitor with real-time TUI"
Enter
Type ""
Enter
Type "USAGE: process-monitor [OPTIONS]"
Enter
Type ""
Enter
Type "OPTIONS:"
Enter
Type "  --web         Enable web server mode"
Enter
Type "  --port PORT   Web server port (default: 9090)"
Enter
Type "  --no-tui      Disable TUI dashboard"
Enter
Sleep 400ms

Type "process-monitor --web --port 9090"
Enter
Sleep 1s
Type "   ✓ eBPF probe loaded"
Enter
Type "   ✓ PerfEventArray attached to 6 syscalls"
Enter
Type "   ✓ Web server listening on :9090"
Enter
Sleep 600ms

Type "curl -s localhost:9090/api/stats"
Enter
Sleep 1s
Type '{"total_events": 12847, "processes_tracked": 42, "alerts": 0}'
Enter
Sleep 500ms

Type "# eBPF kernel tracing · zero-copy · real-time"
Enter
Sleep 500ms
