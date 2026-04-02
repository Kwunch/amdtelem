# amdtelem

A terminal GPU telemetry dashboard for AMD GPUs on Linux, written in Rust.

Reads hardware sensor data directly from the Linux kernel's sysfs and debugfs 
interfaces and displays it in a live-updating terminal UI powered by ratatui.

## Features

- Automatic AMD GPU discovery across multi-GPU systems (selects discrete GPU by VRAM size)
- Dynamic hwmon path discovery — no hardcoded device paths
- Live updating terminal dashboard (1s poll interval)
- Temperature color coding — green/yellow/red based on thermal thresholds
- GPU load color coding
- Inline error display when sensor reads fail
- Errors also logged to `/tmp/amdtelem.log`
- Press `q` to exit cleanly

## Dashboard
```
┌AMD Radeon Telemetry──────────────────────────────────────┐
│GPU           | Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M│
│Temperatures  | Edge: 37.0°C  Junc: 49.0°C  Mem: 50.0°C  │
│Clocks        | SCLK:  85 MHz  MCLK: 772 MHz              │
│Power         | Avg:  38.00W   SoC:  13.24W               │
│Load          | GPU:    0%     VCN:    0%                  │
│Voltage       | VDDGFX: 629 mV  VDDNB: N/A                │
│Fan           | RPM:    0 RPM                              │
│GEM Clients───────────────────────────────────────────────│
│kgx (9032)  gnome-shell (3403)                            │
└──────────────────────────────────────────────────────────┘
```

## Requirements

- Linux with `amdgpu` kernel driver loaded
- AMD GPU (discrete GPU automatically selected on hybrid systems)
- `lspci` installed (`pciutils` package)
- Root access — required to read `/sys/kernel/debug/dri/`
- Rust toolchain (`cargo`)

## Build
```bash
git clone https://github.com/Kwunch/amdtelem
cd amdtelem
cargo build --release
```

## Run
```bash
sudo ./target/release/amdtelem
```

## Data Sources

| Field | Source |
|---|---|
| Edge/Junction/Memory temp | hwmon (`temp1/2/3_input`) |
| Shader clock (SCLK) | hwmon (`freq1_input`) |
| Memory clock (MCLK) | hwmon (`freq2_input`) with DRI fallback |
| Power (average) | hwmon (`power1_average`) |
| GPU/VCN load | debugfs (`amdgpu_pm_info`) |
| SoC power | debugfs (`amdgpu_pm_info`) |
| Fan RPM | hwmon (`fan1_input`) |
| Voltage (VDDGFX) | hwmon (`in0_input`) |
| GEM clients | debugfs (`amdgpu_gem_info`) |

## Architecture
```
main.rs       — terminal setup, event loop, ratatui draw call
telem.rs      — GpuTelemetry struct, sensor collection, GPU discovery
```

Data is collected each tick and stored in `TelemetryData`. The draw closure 
reads from this struct each frame. GPU and hwmon paths are discovered once at 
startup via sysfs traversal.

## Kernel Interfaces Used

- `/sys/class/drm/` — GPU discovery, vendor ID, VRAM size
- `/sys/class/drm/cardN/device/hwmon/` — hardware sensors via hwmon subsystem
- `/sys/kernel/debug/dri/0/` — amdgpu driver debug interface (requires root)

## Roadmap

- [ ] Kernel module (`/dev/amdtelem`) for atomic sensor reads
- [ ] Power limit adjustment via sysfs write
- [ ] Clock frequency override
- [ ] Fan curve control
- [ ] Multi-GPU selection via `--device` flag

## License

MIT
