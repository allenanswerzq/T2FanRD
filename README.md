# T2FanRD

A fork of [GnomedDev/t2fanrd](https://github.com/GnomedDev/t2fanrd), which is
archived.

Fan daemon for T2 Macs. It keeps the Mac cool by watching several
temperatures, not just the CPU: each curve looks at the hottest of its
sensors, and the fans follow the hottest curve. The control model follows
[fan2go](https://github.com/markusressel/fan2go):

- Each curve averages its sensor over its own time span, so a short CPU spike
  doesn't spin the fans up, and slow heat in the heatsinks or case still does.
- Hysteresis keeps a curve from following a falling temperature at once.
- The fans speed up at once and slow down at `ramp_down` percent per second.
- A sensor without a good reading for `stale_after` seconds sends the fans to
  full speed.
- After sleep, or when anything else changes the fans, the daemon takes them
  back. On exit, and on errors, it hands them back to the SMC.

## Compilation
`cargo build --release`

## Usage

```
t2fanrd [--config /etc/t2fanrd.toml] [--trace] [--dry-run] [--list-sensors] [--full-speed]
```

- `--list-sensors` prints every sensor a curve can use, as `chip:label`. The
  T2 SMC's sensors are `smc:*`; the others come from hwmon (`coretemp:Package id 0`).
- `--dry-run` computes the fan speeds and prints them every second without
  touching the fans, so it runs next to the daemon and without root.
- `--trace` prints the same lines while it controls the fans.
- `--full-speed` holds every fan at its maximum instead of following the
  curves, and takes the fans back after sleep like the curves do.

## Configuration

`/etc/t2fanrd.toml`; [t2fanrd.toml](t2fanrd.toml) is the config for the
13-inch 2020 MacBook Pro.

| Key | Default | Meaning |
|---|---|---|
| `interval` | `0.5` | Seconds between updates. |
| `stale_after` | `5.0` | Seconds without a good reading before the fans go to full speed. |
| `output.smoothing` | `0` | Low-pass time constant in seconds on the fan speed. |
| `output.deadband` | `1` | Changes smaller than this many percent are not written. |
| `output.ramp_up` | `0` | Percent per second the fans may speed up; 0 is at once. |
| `output.ramp_down` | `3` | Percent per second the fans may slow down; 0 is at once. |

Each `[[curve]]` maps its hottest sensor from 0% of every fan's RPM range at
`low` to 100% at `high`:

| Key | Meaning |
|---|---|
| `name` | Shown in `--trace` and in errors. |
| `sensors` | `chip:label` names from `--list-sensors`. |
| `low`, `high` | Degrees Celsius. |
| `average` | Time constant in seconds of the moving average; 0 uses each reading. |
| `hysteresis` | Degrees a temperature must fall before the curve follows it down. |

### NixOS
Add this repo to your flake inputs, the module `<name-of-input>.nixosModules.t2fanrd`
to your imports, and set `services.t2fanrd.enable = true;`. `services.t2fanrd.settings`
holds the config, with the MacBook Pro config above as its default.
