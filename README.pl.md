# Halcyon Process Monitor

> **Telemetria procesów i operacji plikowych w czasie rzeczywistym dla Linuksa, oparta na eBPF.**

Halcyon Process Monitor śledzi syscalle `execve` i `openat` na poziomie jądra
przez tracepointy eBPF, strumieniuje zdarzenia do przestrzeni użytkownika przez
bufor per-CPU perf, i prezentuje je w terminalowym TUI na żywo — jednocześnie
oceniając tempo otwierania plików przez każdy proces w ruchomym oknie, aby
wykrywać masowy dostęp do plików w stylu ransomware.

```
                 ┌──────────────────────────────────────────────┐
                 │            Przestrzeń jądra (eBPF)           │
   execve  ──► sys_enter_execve ─┐                               │
   openat  ──► sys_enter_openat ─┼──► ProcessEvent ──► EVENTS   │
                                 │         (map)     (perf array)│
                 └───────────────────────────┬──────────────────┘
                                             │  bufory per-CPU perf
                 ┌───────────────────────────▼──────────────────┐
                 │          Przestrzeń użytkownika (Rust)       │
                 │  wątek czytający ──► kanał ──► Monitor       │
                 │        │                        │            │
                 │        │                  ruchome okno       │
                 │        │                  + alerty           │
                 │        └──► TUI / JSON / plain / diagnose    │
                 └──────────────────────────────────────────────┘
```

## Funkcje

| Możliwość | Opis |
|---|---|
| **Śledzenie na poziomie jądra** | Tracepointy `execve` i `openat` podpięte na każdym aktywnym CPU |
| **Kod jądra bezpieczny dla verifiera** | Wskaźniki przestrzeni użytkownika czytane wyłącznie przez `bpf_probe_read_user` — nigdy przez dereferencję |
| **Potok zdarzeń zero-copy** | Rekordy `ProcessEvent` o stałym rozmiarze strumieniowane przez bufory per-CPU `PerfEventArray` |
| **TUI na żywo** | Dziennik zdarzeń, tabela statystyk per-proces i panel alertów renderowane przez `ratatui` |
| **Heurystyka ruchomego okna** | 1-sekundowe okno per PID; alert, gdy proces przekroczy skonfigurowane tempo otwierania |
| **Wiele trybów wyjścia** | TUI dla człowieka, JSON z podziałem na linie, zwykły dziennik tekstowy i wbudowana autodiagnostyka |
| **Liczenie utraconych zdarzeń** | Przepełnienia bufora perf są liczone i raportowane, nigdy cicho pomijane |
| **Pojedynczy statyczny binarny plik** | Pełne LTO, `panic = "abort"`, profil release z usuniętymi symbolami |

## Wymagania

| Wymaganie | Uwagi |
|---|---|
| Jądro Linux **5.8+** | wsparcie eBPF + tracepointów |
| **root** (`CAP_BPF` / `CAP_SYS_ADMIN`) | wymagane do załadowania i podpięcia programów eBPF |
| Rust **nightly** + `rust-src` | buduje program eBPF z `-Z build-std` |
| `bpf-linker`, `clang`, kompilator C | toolchain linkowania eBPF |
| BTF (`/sys/kernel/btf/vmlinux`) | zalecane dla zgodności CO-RE |

Binarka przestrzeni użytkownika buduje się na stabilnym Rust; tylko crate eBPF
wymaga nightly.

## Szybki start

Najszybsza ścieżka to instalator rozpoznający dystrybucję — wykrywa menedżer
pakietów (apt, dnf, pacman, zypper, apk, xbps), instaluje zależności buildowe
z oficjalnych repozytoriów dystrybucji, provisionuje toolchain Rusta, buduje
i instaluje:

```bash
# Instalacja systemowa do /usr/local (pyta przed każdą zmianą):
./install.sh --system

# Instalacja lokalna do ~/.local (bez sudo):
./install.sh
```

Albo buduj ręcznie:

```bash
./build.sh
sudo target/release/process-monitor
```

## Użycie

```bash
# TUI (domyślnie, gdy stdout to terminal)
sudo process-monitor

# Podnieś próg alertu (otwarcia plików na sekundę przed alertem)
sudo process-monitor --alert-threshold 100

# Wyjście maszynowe do potoków / agregacji logów
sudo process-monitor --json | jq .

# Zwykły dziennik tekstowy (bez TUI, bez JSON)
sudo process-monitor --plain

# Jawna ścieżka do skompilowanego obiektu eBPF (inaczej auto-wykrywanie)
sudo process-monitor --bpf /path/to/process-monitor-ebpf

# 5-sekundowa autodiagnostyka end-to-end, potem wyjście
sudo process-monitor --diagnose
```

### Referencja opcji CLI

| Flaga | Domyślnie | Opis |
|---|---|---|
| `-b, --bpf <PATH>` | auto-wykrywanie | Ścieżka do skompilowanego obiektu eBPF |
| `--alert-threshold <N>` | `50` | Alert, gdy proces otworzy N+ plików w 1 s (`0` wyłącza alerty) |
| `--json` | wył. | Wyjście JSON z podziałem na linie (bez TUI) |
| `--plain` | wył. | Zwykły dziennik tekstowy (bez TUI); koliduje z `--json` |
| `--tui` | wył. | Wymuś TUI, nawet gdy stdout nie jest terminalem |
| `--diagnose` | wył. | Uruchom 5-sekundową autodiagnostykę end-to-end i wyjdź |

### Klawisze TUI

| Klawisz | Akcja |
|---|---|
| `q`, `Esc`, `Ctrl+C` | Wyjście |
| `p` | Pauza / wznowienie strumienia zdarzeń |
| `c` | Wyczyść dziennik zdarzeń |
| `↑` / `↓`, `j` / `k` | Przewijanie dziennika zdarzeń |
| `PgUp` / `PgDn` | Szybsze przewijanie |
| `Home` / `End` | Skok na górę / dół |

## Schemat wyjścia JSON

Każde zdarzenie to jeden obiekt JSON na linię (JSON z podziałem na linie):

```jsonc
// Zdarzenie procesu
{"ts": "14:09:16.531", "type": "open", "pid": 29645, "uid": 1000, "comm": "process-monitor", "file": "/dev/tty"}
{"ts": "14:09:16.532", "type": "exec", "pid": 29645, "uid": 1000, "comm": "bash", "file": "/usr/bin/ls"}

// Alert
{"ts": "14:09:16.973", "type": "alert", "pid": 2126, "uid": 1000, "comm": "Cache2 I/O", "opens_in_1s": 50}
```

| Pole | Występuje w | Znaczenie |
|---|---|---|
| `ts` | wszystkie | Znacznik czasu lokalnego, `HH:MM:SS.mmm` |
| `type` | wszystkie | `exec`, `open` lub `alert` |
| `pid` | wszystkie | ID procesu, który wywołał syscall |
| `uid` | wszystkie | ID użytkownika procesu |
| `comm` | wszystkie | Nazwa procesu (16-bajtowe `comm`, obcięte) |
| `file` | exec/open | Ścieżka docelowa (bufor 64-bajtowy, obcięty) |
| `opens_in_1s` | alert | Otwarcia plików zliczone w bieżącym ruchomym oknie |

## Jak to działa

1. Dwa tracepointy jądra (`sys_enter_execve`, `sys_enter_openat`) zapisują PID,
   UID, `comm` i nazwę pliku docelowego do zwartego rekordu `ProcessEvent`
   o stałym układzie i wrzucają go do `EVENTS` `PerfEventArray`.
2. Dedykowany wątek czytający otwiera jeden bufor perf na każdy aktywny CPU
   i przekazuje zdekodowane zdarzenia przez kanał MPSC do rdzenia monitora.
3. Monitor utrzymuje 1-sekundowe ruchome okno otwarć plików per PID i podnosi
   alert, gdy proces przekroczy skonfigurowany próg.
4. Wyjście jest renderowane zgodnie z wybranym trybem: TUI, JSON, plain lub
   raport diagnostyczny.

Pełny projekt: **[ARCHITECTURE.md](ARCHITECTURE.md)**.

## Struktura projektu

```
halcyon-process-monitor/
├── process-monitor/          # Przestrzeń użytkownika: rdzeń monitora + TUI + tryby wyjścia
│   └── src/
│       ├── main.rs           # CLI, wybór trybu, obsługa sygnałów
│       ├── monitor.rs        # Ładowanie eBPF, czytnik perf, tracker ruchomego okna
│       └── tui.rs            # Interfejs ratatui (zdarzenia / statystyki / alerty)
├── process-monitor-ebpf/     # Strona jądra (#![no_std], aya-ebpf)
│   └── src/main.rs           # Hooki tracepoint → PerfEventArray
├── build.sh                  # Skrypt budowania (nightly dla eBPF, stable dla TUI)
├── install.sh                # Instalator / dezinstalator rozpoznający dystrybucję
└── Cargo.toml                # Definicja workspace'a
```

## Uwagi bezpieczeństwa

- Instalator nigdy nie czyta ani nie loguje tokenów, poświadczeń ani sekretów
  środowiskowych i nie wykonuje **żadnych uwierzytelnionych żądań sieciowych**.
- Zależności instalowane są tylko z oficjalnych repozytoriów dystrybucji;
  jedyne pobrania to oficjalny instalator `rustup.rs` (z pytaniem) i crates
  z crates.io.
- Wszystkie komendy shell są cytowane i bezpieczne domyślnie; instalacje
  lokalne nie wymagają `sudo`.
- Kod jądra przestrzega ścisłych zasad bezpieczeństwa eBPF: cała pamięć
  przestrzeni użytkownika czytana jest przez `bpf_probe_read_user`, nigdy
  przez dereferencję.
- Programy eBPF muszą działać jako root; uruchamiaj monitor przez `sudo`.

## Rozwiązywanie problemów

Uruchom wbudowaną autodiagnostykę, aby zweryfikować toolchain, dostępność
tracepointów, ładowanie eBPF i przepływ zdarzeń end-to-end:

```bash
sudo process-monitor --diagnose
```

Typowe sprawdzenia:

- **Brak zdarzeń w ogóle** → potwierdź, że tracepointy istnieją
  (`/sys/kernel/tracing/events/syscalls/sys_enter_execve/id`) i że bufory
  perf się otworzyły (`[halcyon] opening perf buffers on N CPUs`).
- **`failed to load eBPF program`** → ścieżka do obiektu eBPF jest błędna;
  podaj `--bpf` jawnie lub uruchom ponownie `./install.sh`.
- **Brak uprawnień root** → `Monitor::start` kończy się jasnym komunikatem
  `CAP_BPF / CAP_SYS_ADMIN`.

## Dezinstalacja

```bash
./install.sh --uninstall            # usuń instalację lokalną
./install.sh --uninstall --system   # usuń instalację systemową
```

## Licencja

MIT — crates deklarują `license = "MIT"` w manifestach `Cargo.toml`.
