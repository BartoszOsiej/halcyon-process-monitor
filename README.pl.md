# Halcyon Process Monitor

> **Telemetria procesów, operacji plikowych i sieci w czasie rzeczywistym dla Linuksa, oparta na eBPF.**

Halcyon Process Monitor śledzi syscalle `execve`, `openat`, `connect`, `accept`, `sendto` i `recvfrom` na poziomie jądra przez tracepointy eBPF, strumieniuje zdarzenia do przestrzeni użytkownika przez bufor per-CPU perf, i prezentuje je w zaawansowanym terminalowym TUI na żywo — jednocześnie oceniając tempo otwierania plików przez każdy proces w ruchomym oknie, aby wykrywać masowy dostęp do plików w stylu ransomware.

```
┌─────────────────────────────────────────────────────────────────┐
│                        PRZESTRZEŃ JĄDRA (eBPF)                 │
│                                                                 │
│  sys_enter_execve ──┐                                           │
│  sys_enter_openat  ──┤                                           │
│  sys_enter_connect ──┤   ProcessEvent    PerfEventArray         │
│  sys_enter_accept  ──┼──► (map)    ────► (bufory per-CPU)      │
│  sys_enter_sendto  ──┤                                           │
│  sys_enter_recvfrom ─┘                                           │
└──────────────────────────────────┬──────────────────────────────┘
                                   │
┌──────────────────────────────────▼──────────────────────────────┐
│                  PRZESTRZEŃ UŻYTKOWNIKA (Rust)                  │
│                                                                 │
│  wątek czytający ──► kanał ──► Monitor ──► TUI / JSON / Web    │
│       │                  │         │                            │
│       │                  │    ruchome okno                      │
│       │                  │    + alerty                          │
│       │                  │    + drzewo procesów                 │
│       │                  │    + ranking plików                  │
│       │                  │    + śledzenie sieci                 │
│       │                  │    + heatmapa                        │
│       └──► perf buffer   └──► szukanie/filtrowanie             │
└─────────────────────────────────────────────────────────────────┘
```

## Wersja angielska

**[English README](README.md)** · [Nowe funkcje v0.4](NEW_FEATURES.md) · [Architektura](ARCHITECTURE.md)

## Funkcje

| Możliwość | Opis |
|---|---|
| **Śledzenie na poziomie jądra** | Tracepointy `execve`, `openat`, `connect`, `accept`, `sendto`, `recvfrom` |
| **Kod jądra bezpieczny dla verifiera** | Wskaźniki przestrzeni użytkownika czytane wyłącznie przez `bpf_probe_read_user` |
| **Potok zdarzeń zero-copy** | Rekordy `ProcessEvent` o stałym rozmiarze przez bufory per-CPU `PerfEventArray` |
| **Ultra-zaawansowane TUI** | 7 paneli: Zdarzenia, Procesy, Sieć, Pliki, Rozszerzenia, Alerty, Heatmapa |
| **Szukanie i filtrowanie** | Wyszukiwanie tekstu w zdarzeniach (`/` aby aktywować) |
| **Widok szczegółów procesu** | Pełna hierarchia drzewa procesów ze statystykami (`Enter`) |
| **Nakładka pomocy** | Kompletna lista skrótów klawiszowych (`?` aby pokazać) |
| **Zmiana rozmiaru paneli** | Przesuwanie granic kolumn `[`/`]` i `{`/`}` |
| **Panel sieci** | Podgląd connect/accept/sendto/recvfrom z IP:port |
| **Sparkline'y tempa** | Wizualizacje exec/s, open/s, net/s, alert/s (120s) |
| **Heatmapa** | Wizualizacja częstotliwości syscalli |
| **Śledzenie typów plików** | Częstotliwość otwarć per rozszerzenie z kolorowymi kategoriami |
| **Ranking plików** | Pliki posortowane wg liczby otwarć, z entropią Shannona |
| **Heurystyka ruchomego okna** | 1-sekundowe okno per PID; alert przy przekroczeniu progu |
| **Wiele trybów wyjścia** | TUI, JSON, plain text, autodiagnostyka, dashboard webowy |
| **Dashboard webowy** | REST API + WebSocket + metryki Prometheus (opcjonalny build) |
| **Agent Go** | Lekki CLI łączący się z daemonem przez HTTP/WebSocket |
| **Biblioteka C FFI** | `libhalcyon.so` z C bindings dla integracji międzyjęzykowej |
| **Kubernetes** | Manifesty DaemonSet + Service + ConfigMap + ServiceMonitor |
| **Schemat Protobuf** | Definicja usługi gRPC dla komunikacji między komponentami |
| **Liczenie utraconych zdarzeń** | Przepełnienia bufora perf liczone i raportowane |
| **Pojedynczy statyczny binarny** | Pełne LTO, `panic = "abort"`, usunięte symbole |

## Budowanie

```bash
# TUI-only (domyślne, 1.7MB) — zalecane
./build.sh
# lub
cargo build --release

# Z serwerem webowym (2.5MB) — z REST API, WebSocket, Prometheus
./build.sh --web
# lub
cargo build --release --features web

# Oba warianty
./build.sh --all
```

### Rozmiary binarek (release)

| Wariant | Rozmiar | Zależności |
|---|---|---|
| `process-monitor-tui` | 1.7MB | aya, ratatui, chrono, crossterm |
| `process-monitor-web` | 2.5MB | +axum, tokio, tower-http, prometheus-client |

## Wymagania

| Wymaganie | Uwagi |
|---|---|
| Jądro Linux **5.8+** | wsparcie eBPF + tracepointów |
| **root** (`CAP_BPF` / `CAP_SYS_ADMIN`) | wymagane do załadowania programów eBPF |
| Rust **nightly** + `rust-src` | buduje program eBPF z `-Z build-std` |
| `bpf-linker`, `clang` | toolchain linkowania eBPF |
| BTF (`/sys/kernel/btf/vmlinux`) | zalecane dla zgodności CO-RE |

## Szybki start

```bash
# Instalacja systemowa do /usr/local
./install.sh --system

# Instalacja lokalna do ~/.local
./install.sh

# Budowanie ręczne
./build.sh
sudo target/release/process-monitor-tui
```

## Użycie

```bash
# TUI (domyślnie)
sudo process-monitor

# Podnieś próg alertu
sudo process-monitor --alert-threshold 100

# Filtruj wg rozszerzenia
sudo process-monitor --filter-ext pdf

# Wyjście JSON
sudo process-monitor --json | jq .

# Dashboard webowy (wymaga --features web)
sudo process-monitor --web 0.0.0.0:8080
```

## Klawisze TUI

### Nawigacja

| Klawisz | Akcja |
|---|---|
| `q` / `Esc` | Wyjście (lub wyczyść scroll) |
| `p` | Pauza / wznowienie |
| `c` | Wyczyść wszystkie panele |
| `↑`/`↓` / `k`/`j` | Przewijanie |
| `PgUp`/`PgDn` | Szybsze przewijanie |
| `g` / `Home` | Skok na górę |
| `G` / `End` | Skok na dół |

### Panele

| Klawisz | Akcja |
|---|---|
| `Tab` | Następny panel |
| `Shift+Tab` | Poprzedni panel |
| `1`-`7` | Skocz do panelu numerem |
| `Enter` | Otwórz szczegóły procesu |

### Szukanie

| Klawisz | Akcja |
|---|---|
| `/` | Tryb szukania |
| `Esc` | Anuluj szukanie |
| `Enter` | Zastosuj filtr |

### Układ

| Klawisz | Akcja |
|---|---|
| `[` / `]` | Zmień rozmiar lewego panelu |
| `{` / `}` | Zmień rozmiar środkowego panelu |

### Inne

| Klawisz | Akcja |
|---|---|
| `?` / `h` | Nakładka pomocy |
| `Ctrl+C` | Wymuś wyjście |

### Panele TUI (7)

| # | Panel | Opis |
|---|---|---|
| 1 | **EVENTS** | Dziennik zdarzeń z szukaniem/filtrowaniem |
| 2 | **PROCESSES** | Hierarchiczne drzewo procesów z mini-paskami |
| 3 | **NETWORK** | Połączenia sieciowe na żywo (connect/accept/send/recv) |
| 4 | **TOP FILES** | Najczęściej otwierane pliki z entropią Shannona |
| 5 | **FILE TYPES** | Częstotliwość rozszerzeń z kolorowymi paskami |
| 6 | **ALERTS** | Historia alertów z czasami |
| 7 | **HEATMAP** | Wizualizacja częstotliwości syscalli |

## Śledzenie sieci

Nowe w v0.4 — śledzenie syscalli sieciowych:

| Syscall | Typ zdarzenia | Przechwytuje |
|---|---|---|
| `connect` | `Connect` | Adres zdalny IPv4/IPv6/Unix |
| `accept` | `Accept` | Adres zdalny |
| `sendto` | `SendTo` | Cel + wysłane bajty |
| `recvfrom` | `RecvFrom` | Źródło + odebrane bajty |

## Dashboard webowy

Opcjonalny build z `--features web`:

```bash
cargo build --release --features web
sudo process-monitor --web 0.0.0.0:8080
```

### Endpoints

| Endpoint | Metoda | Opis |
|---|---|---|
| `/` | GET | Dashboard webowy cyberpunk |
| `/ws` | WebSocket | Strumień zdarzeń na żywo |
| `/api/v1/stats` | GET | Statystyki globalne |
| `/api/v1/processes` | GET | Śledzone procesy |
| `/api/v1/files` | GET | Najczęściej otwierane pliki |
| `/api/v1/extensions` | GET | Częstotliwość rozszerzeń |
| `/api/v1/threshold` | POST | Zmień próg w czasie rzeczywistym |
| `/metrics` | GET | Metryki Prometheus |

## Agent Go

Lekki CLI łączący się z daemonem:

```bash
cd go-agent && go build -o halcyon-agent .
./halcyon-agent stats
./halcyon-agent processes
./halcyon-agent watch    # WebSocket na żywo
```

## Biblioteka C FFI

C-compatible library do integracji z innymi językami:

```c
#include "halcyon.h"

halcyon_monitor_t* monitor;
halcyon_monitor_create("/path/to/bpf.o", 50, &monitor);

halcyon_event_t events[100];
uint32_t count;
halcyon_monitor_poll(monitor, events, 100, &count);

for (uint32_t i = 0; i < count; i++) {
    printf("[%d] %s\n", events[i].pid, events[i].comm);
    halcyon_free_string(events[i].comm);
}
halcyon_free_events(events, count);
halcyon_monitor_destroy(monitor);
```

## Kubernetes

Wdróż Halcyon jako DaemonSet na każdym węźle:

```bash
kubectl create namespace observability
kubectl apply -f k8s/
```

### Komponenty

| Zasób | Opis |
|---|---|
| `DaemonSet` | Uruchamia Halcyon na każdym węźle z dostępem do eBPF |
| `Service` | Serwis ClusterIP dla dostępu do API |
| `ConfigMap` | Konfiguracja (próg, poziom logowania, etc.) |
| `ServiceMonitor` | Integracja z operatorem Prometheus |

## Schemat JSON

```jsonc
{"ts": "14:09:16.531", "type": "open", "pid": 29645, "uid": 1000, "comm": "process-monitor", "file": "/dev/tty"}
{"ts": "14:09:16.973", "type": "alert", "pid": 2126, "uid": 1000, "comm": "Cache2 I/O", "opens_in_1s": 50}
{"ts": "14:09:17.100", "type": "connect", "pid": 1234, "uid": 1000, "comm": "curl", "file": "93.184.216.34:443"}
```

## Struktura projektu

```
halcyon-process-monitor/
├── process-monitor/          # Przestrzeń użytkownika: rdzeń + TUI + web + FFI
│   └── src/
│       ├── main.rs           # CLI, wybór trybu, obsługa sygnałów
│       ├── monitor.rs        # Ładowanie eBPF, czytnik perf, ruchome okno
│       ├── tui.rs            # Ultra-zaawansowany interfejs ratatui
│       ├── web.rs            # Serwer axum (opcjonalny, --features web)
│       └── ffi.md            # C FFI bindings (libhalcyon)
├── process-monitor-ebpf/     # Strona jądra (#![no_std], aya-ebpf)
│   └── src/
│       ├── main.rs           # Tracepointy execve/openat → PerfEventArray
│       └── network.rs        # Tracepointy connect/accept/sendto/recvfrom
├── go-agent/                 # Agent Go (klient HTTP/WebSocket)
├── c-api/                    # Nagłówek C dla libhalcyon
├── k8s/                      # Manifesty Kubernetes
├── proto/                    # Schemat Protobuf (definicja usługi gRPC)
├── build.sh                  # Skrypt budowania (--web, --all)
├── install.sh                # Instalator rozpoznający dystrybucję
├── NEW_FEATURES.md           # Dokumentacja funkcji v0.4
└── Cargo.toml                # Definicja workspace'a
```

## Bezpieczeństwo

- Instalator nie wykonuje **żadnych uwierzytelnionych żądań sieciowych**
- Zależności z oficjalnych repozytoriów dystrybucji + `rustup.rs`
- Kod jądra przestrzega ścisłych zasad eBPF: `bpf_probe_read_user` tylko
- Programy eBPF wymagają root

## Docker

```bash
docker build -t halcyon-process-monitor .
docker run --privileged -v /sys/kernel/btf:/sys/kernel/btf \
    halcyon-process-monitor process-monitor
```

## Rozwiązywanie problemów

```bash
sudo process-monitor --diagnose    # 5-sekundowa autodiagnostyka
```

- **Brak zdarzeń** → sprawdź tracepointy w `/sys/kernel/tracing/events/syscalls/`
- **Błąd ładowania eBPF** → podaj `--bpf` jawnie lub uruchom `./install.sh` ponownie
- **Brak root** → wymagane `CAP_BPF` lub `CAP_SYS_ADMIN`

## Dezinstalacja

```bash
./install.sh --uninstall            # lokalna
./install.sh --uninstall --system   # systemowa
```

## Licencja

MIT
