# Halcyon Process Monitor — C + Go rewrite
#
# Components:
#   c-ebpf/       eBPF kernel programs (libbpf)
#   c-monitor/    Core monitor library (libbpf userspace)
#   c-tui/        ncurses TUI frontend
#   go-web/       Web dashboard (Go)

CC      ?= gcc
CFLAGS   = -Wall -Wextra -O2 -std=c11 -D_GNU_SOURCE
LDFLAGS  = -lpthread -lncurses -lelf -lz
BPF_CC   = clang
BPF_CFLAGS = -O2 -g -Wall -target bpf -D__TARGET_ARCH_x86

PREFIX   ?= /usr/local
BINDIR   = $(PREFIX)/bin
LIBDIR   = $(PREFIX)/lib/halcyon

.PHONY: all clean install uninstall ebpf monitor tui web help

all: ebpf monitor tui

help:
	@echo "Halcyon Process Monitor (C + Go)"
	@echo ""
	@echo "Targets:"
	@echo "  all       Build everything (ebpf + monitor + tui)"
	@echo "  ebpf      Compile eBPF kernel programs"
	@echo "  monitor   Build core monitor library"
	@echo "  tui       Build ncurses TUI"
	@echo "  web       Build Go web dashboard"
	@echo "  install   Install to $(PREFIX)"
	@echo "  clean     Remove build artifacts"
	@echo ""
	@echo "Dependencies:"
	@echo "  C: libbpf, libelf, zlib, ncurses, clang (for eBPF)"
	@echo "  Go: gorilla/websocket (for web dashboard)"

# ── eBPF ──────────────────────────────────────────────────────────────────

ebpf: c-ebpf/process_monitor.bpf.o

c-ebpf/process_monitor.bpf.o: c-ebpf/process_monitor.bpf.c
	$(BPF_CC) $(BPF_CFLAGS) -c $< -o $@

# ── Monitor library ───────────────────────────────────────────────────────

MONITOR_SRC = c-monitor/src/monitor.c
MONITOR_OBJ = $(MONITOR_SRC:.c=.o)

monitor: $(MONITOR_OBJ)

$(MONITOR_OBJ): c-monitor/src/monitor.c c-monitor/include/halcyon.h
	$(CC) $(CFLAGS) -Ic-monitor/include -c $< -o $@

# ── TUI ───────────────────────────────────────────────────────────────────

TUI_SRC = c-tui/src/tui.c
TUI_OBJ = $(TUI_SRC:.c=.o)
TUI_BIN = halcyon-tui

tui: $(TUI_BIN)

$(TUI_BIN): $(TUI_OBJ) $(MONITOR_OBJ)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS) -lbpf -lelf -lz

$(TUI_OBJ): c-tui/src/tui.c c-monitor/include/halcyon.h
	$(CC) $(CFLAGS) -Ic-monitor/include -c $< -o $@

# ── Web dashboard (Go) ────────────────────────────────────────────────────

web:
	cd go-web && go build -o halcyon-web .

# ── Install ───────────────────────────────────────────────────────────────

install: all
	install -d $(DESTDIR)$(BINDIR)
	install -d $(DESTDIR)$(LIBDIR)
	install -m 755 $(TUI_BIN) $(DESTDIR)$(BINDIR)/halcyon
	install -m 644 c-ebpf/process_monitor.bpf.o $(DESTDIR)$(LIBDIR)/
	install -m 644 c-monitor/include/halcyon.h $(DESTDIR)$(LIBDIR)/
	@if [ -f go-web/halcyon-web ]; then \
		install -m 755 go-web/halcyon-web $(DESTDIR)$(BINDIR)/halcyon-web; \
	fi
	@echo ""
	@echo "Installed to $(DESTDIR)$(PREFIX)"
	@echo "  $(BINDIR)/halcyon       — TUI monitor"
	@echo "  $(LIBDIR)/process_monitor.bpf.o — eBPF program"
	@echo "  $(LIBDIR)/halcyon.h     — C API header"

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/halcyon
	rm -f $(DESTDIR)$(BINDIR)/halcyon-web
	rm -rf $(DESTDIR)$(LIBDIR)

clean:
	rm -f c-ebpf/*.o c-monitor/src/*.o c-tui/src/*.o $(TUI_BIN)
	rm -f go-web/halcyon-web
