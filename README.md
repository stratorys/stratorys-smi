# Stratorys SMI

A small, native Rust hardware dashboard for Linux/NVIDIA and macOS on Apple silicon; Intel Macs are not supported. All output is English. Run it locally on the machine doing inference; no daemon, network listener, CUDA toolkit, or configuration file is needed.

```sh
cargo build --locked -p stratorys-smi --release
./target/release/stratorys-smi
./target/release/stratorys-smi --interval 2
./target/release/stratorys-smi --once
./target/release/stratorys-smi --details
./target/release/stratorys-smi --no-smart
```

The release executable embeds its logo. Copy that executable to a compatible host; neither the assets directory nor Rust is needed at runtime. NVIDIA monitoring requires the installed NVIDIA driver and its NVML library. macOS uses system frameworks, SMC, IOHID and IOReport through the macOS-only [Stratorys fork of macmon](https://github.com/stratorys/macmon) library without its application features; one native worker owns a single SMC connection. Optional sources can fail without stopping CPU and memory monitoring.

## Display

The overview is centered horizontally and vertically and fits one screen. Each logical CPU core has its own usage bar and percentage. A spacious screen shows the Stratorys logo, a system table, GPU cards, CPU/memory panels, and histories. Split panes retain the header and metric cards by reducing padding and hiding history plots before switching to the smallest layout. The aligned system table stays visible even when the logo is hidden. The app preserves the terminal background and does not change fan speeds, clocks, power limits, or system configuration.

- `--details`: opt into full sensor and diagnostic pages at launch.
- Arrow keys, Page Up/Down, or the mouse wheel: change detail pages.
- Home/End: first/last detail page.
- `q` or Ctrl+C: exit and restore the terminal.

The minimum width is 60 columns. The minimum height is the height of the smallest layout, at least 18 rows: one line per metric, with every GPU and every CPU core visible without scrolling. The core grid uses between one and eight columns according to screen width. Below the minimum, a resize prompt replaces the dashboard. Resizing automatically updates the layout.

Ghostty and Kitty display the PNG rendered from the original square dark SVG; iTerm2 uses its inline image protocol. The memory card is present from the first frame; runtime warmup does not move panels. Aspect ratio, rounded navy background (#0F172A), original bar geometry, colors, and alpha are preserved inside the logo; the dashboard still uses the terminal background. The logo is hidden in compact layouts. Other terminals and tmux display the text identity without an approximated logo. `--once` and redirected output contain no terminal image, ANSI escape sequences or history plots.

## Measurements and limits

Linux/NVIDIA reports GPU activity, memory-interface activity, VRAM used/free/total, core and memory clocks, performance state, power and enforced power limit, temperature, slowdown threshold, fan percentage, and active clock-limiting reasons, where supported. Memory-interface activity measures time accessing memory, not achieved bandwidth or computational efficiency. Power limiting is reported as an operating state, not automatically classified as a fault.

The NVIDIA VRAM meter reports occupied memory (`total - free`), including the driver reserve. Its breakdown shows allocated (`used` from NVML v2), reserved, and free memory separately. The allocated value corresponds to `nvidia-smi` memory.used; older NVML clients such as btop may include the reserve in their used value. Values below 1 GiB use MiB so small allocations remain visible.

Linux also reports CPU I/O wait, swap-in/out in pages per second, disk read/write rates for whole backing devices, and CPU/memory/I/O PSI `some avg10`. The first counter sample is unavailable; a second sample establishes rates. `--once` primes counters and CPU usage before producing its snapshot. Counter resets discard the affected rate rather than reporting a spike. Virtual device layers and partitions are excluded from aggregate disk rates to avoid double counting.

macOS reports CPU, RAM, swap, available temperature sensors, and readable SMC fans/power. Apple Silicon GPU identity and SMC GPU temperature are shown independently of GPU activity support. A native worker reads SMC and IOHID sensors and IOReport, which provides Apple GPU active residency, core frequency, CPU/GPU power, E/P CPU cluster frequencies, and ANE/DRAM power. It publishes only its latest result, so the UI stays responsive; runtime samples expire after three seconds. IOHID sensors sharing a name are shown once. Ordinary IOReport errors trigger a fresh IOReport reader after 30 seconds, preserving SMC/IOHID connections. Runtime panics disable IOReport until restart; failed complete sampler initialization retains the standalone sensor fallback until restart. Unsupported metrics remain unavailable rather than being reported as zero. The hottest identified CPU/GPU sensor is used for summaries; generic PMU sensors are not relabeled as CPU cores. Raw SMC keys remain visible so model-specific labels are not guessed.

The macOS `sysinfo` noncompressed-memory measurement is labeled separately: it is not free RAM or additional allocation headroom. Apple unified system RAM is not reported as dedicated GPU VRAM. No CPU temperature is substituted for a missing GPU thermistor.

Inventory and sensor discovery refresh every 30 seconds. The full macOS SMC and IOHID sensor lists refresh every five seconds; IOReport runtime measurements refresh every second. SATA `drivetemp` is sampled by a cancellable worker every 30 seconds; `--no-smart` disables it. The app does not issue SMART commands. SATA samples older than 60 seconds are excluded. Telemetry reads are capped at 64 KiB; sensor names are bounded. Limits include 32 NVIDIA GPUs, 256 displayed CPU threads, 1,024 sensor readings/session maxima, and 640 cached SMC keys. Histories retain at most 120 seconds and 601 samples per series; GPU identities absent for more than 120 seconds are evicted, including their gap points; unavailable samples leave visible gaps. Notices are deduplicated and bounded.

The application performs read-only monitoring without network access. It starts no subprocess itself; on macOS, macmon runs `system_profiler` once only if native chip detection fails. That behavior is not an enforced sandbox. Vendor libraries and OS sensor calls can still block, so maximum collection latency must be measured on the actual host.

## Development and validation

Rust 1.98.1 is pinned in `rust-toolchain.toml`. Formatting uses nightly-2026-09-25. GitHub Actions are pinned to verified full commit SHAs. CI checks macOS (Apple silicon) and Linux and produces one release executable for each runner architecture.

```sh
rustup toolchain install nightly-2026-09-25 --profile minimal --component rustfmt
cargo +nightly-2026-09-25 fmt --all --check
cargo +1.98.1 check --locked --all-targets
cargo +1.98.1 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.98.1 test --locked --all-features
```

Check terminal resizing, the minimum-size prompt, detail paging, and terminal restoration manually after changes to terminal handling. Measure CPU usage and RSS separately under a repeatable workload.

Performance targets are about 1.3% of one CPU core and under 30 MiB RSS at a one-second refresh. On an M4 Pro, IOReport sampling alone uses about 0.7% of one core and SMC/IOHID sensors about 0.4%; RSS is about 19 MiB. These are targets, not promises. On NVIDIA hosts, compare readings with `nvidia-smi` under a repeatable workload.

Logo assets are checked in under `assets/`. Keep the SVG, PNG, and base64 representation consistent when updating the logo; regeneration is not part of the build.

## Internal boundaries

`cli` parses and validates arguments; `main` handles startup and exit codes; `app`
owns sampling, histories, terminal events and shutdown.

Collection has one shared core and one adapter per OS. `source` defines the `Source`
trait: a source adds its measurements and notices to the current `Snapshot` and may
describe inventory such as storage. The `Collector` samples every registered source in
order. `Background` runs a blocking source on its own thread, keeps only its latest
result, shows it as warming up or stale when it is missing or too old, and joins on
drop. `platform::sources` registers the portable `system` source (CPU, memory, host
inventory) and the sources of the current OS: `platform/linux` (hwmon, `/proc`
counters, NVML, SATA `drivetemp` in the background) or `platform/macos` (volumes and
the native IOReport/SMC/IOHID worker). Adding hardware support means implementing
`Source` in the OS module and registering it there; the model and the UI do not
change.

The model is platform-independent. It owns validated measurement types, availability
states, sensor categories, bounded histories and typed diagnostics: a `Diagnostic` is
a `SourceId` and a `Problem`, deduplicated with the most serious first. Only
`SourceId` lists per-OS sources.

The UI separates layout, panels, charts, rendering primitives, logo protocols and
terminal lifecycle. Panels render from one `View` and show what the snapshot contains,
without checking the OS. The overview renders the spacious, then the compact layout and
keeps the first that fits, falling back to minimal. Detailed paging is separate.
Sensor categories and unified/dedicated memory topology are adapter metadata,
independent of display names.

On macOS, the application uses only macmon's public bindings, not its `Sampler`:
`platform/macos/ioreport` subscribes to the IOReport CPU, GPU and energy channels and
derives clocks and power itself, following macmon's channel rules, while the native
worker owns the SMC and IOHID readers. IOReport metrics therefore do not depend on SMC
reads. An IOReport sampling failure is retried with a fresh subscription after 30
seconds; a caught panic disables IOReport for the session; SMC and IOHID stay
independent. macmon keeps its FFI `unsafe` code; the application itself has none.

The dependency is pinned to commit
[`3919334d132ed904a594192ed5b26fba025ac49b`](https://github.com/stratorys/macmon/commit/3919334d132ed904a594192ed5b26fba025ac49b)
for its SMC validation and IOReport ownership fixes. Initialization and sampling use
the fork's recoverable IOReport APIs. The local adapters retain generic SMC decoding,
sensor classification, caching and frequency calculations because the fork does not
expose equivalent standalone helpers. Invalid residency counters or frequency tables
leave the affected clock measurement unavailable.

## License

Stratorys SMI is licensed under the [Mozilla Public License 2.0](LICENSE). IOReport
channel handling in `src/platform/macos/ioreport.rs` follows
[macmon](https://github.com/vladkens/macmon) (MIT).
