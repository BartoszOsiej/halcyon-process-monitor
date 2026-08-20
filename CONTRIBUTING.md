# Contributing

Dziekuje za zainteresowanie projektem!

## Setup

1. Wymagany Rust nightly (build eBPF) lub toolchain z sekcji CI.
2. Sklonuj repo, zainstaluj zaleznosci, uruchom testy:
   - `cargo fmt --check`
   - `cargo clippy -- -D warnings`
   - `cargo test`

## Zglaszanie problemow

Zanim powstanie issue, sprawdz czy nie istnieje juz na liscie. Opisz:

- czego oczekiwales,
- co dostales (logi, stack trace),
- wersje systemu/toolchainu.

## Pull requesty

- Pisz male, zawarte zmiany (1 temat = 1 PR).
- Utrzymuj zielony CI (fmt + clippy + testy).
- Dodaj testy dla nowych zachowan.
- Opisuj "dlaczego", nie tylko "co".

Licencja projektu: MIT. Wysylajac PR akceptujesz ja dla swojej zmiany.
