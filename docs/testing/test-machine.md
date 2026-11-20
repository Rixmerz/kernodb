# Test machine

Recorded 2026-11-19. This is the owner's own computer, and it is a laptop. It is
the machine for the final runs listed in [owner-decisions](../owner-decisions.md)
(OPS-13, OPS-06 U2 on disk, OPS-10, U6 medians).

## Measured configuration

| Item | Value |
|---|---|
| CPU | AMD Ryzen 7 4800H, 8 cores / 16 threads |
| RAM | 14 GiB, plus 15 GiB zram swap |
| Storage | NVMe WDC PC SN730 512 GB, the `/home` filesystem (88% full, 60 GB free) |
| OS and kernel | Linux 7.3.3-1-cachyos |

## The reading, and why the machine was not quiet

At the time of the reading the load average was **19.4 / 41.4 / 40.9**
(agent builds running), and one thermal zone read **95 °C**. The machine was
**not quiet**, so no result taken then can count.

A spot check made while writing this page, with the same commands, showed the
same kind of load:

```text
$ uptime
 18:08:28 up  3:32,  1 user,  load average: 8.00, 2.86, 5.12
$ cat /sys/class/thermal/thermal_zone*/temp   # first zone, millidegrees
89000
$ df -h /home
/dev/nvme0n1p2  473G  399G   70G  86% /home
$ lscpu
CPU(s):                                  16
Core(s) per socket:                      7
```

Two differences from the owner's figures: `lscpu` reports 7 cores per socket
(the table above keeps the owner's 8 cores / 16 threads), and the spot check
shows 86% used with 70 GB free. The owner's reading is the one kept here.

`free` shows 30 GiB of swap in total. The table keeps only the 15 GiB zram
device; the other swap device was not identified.

## Risks

- **Thermal throttling.** The laptop reached 95 °C at the reading. A
  throttled CPU changes throughput and p99 numbers during a run.
- **Power profile.** The profile at the spot check was `balanced`
  (`powerprofilesctl get`). A power-saving profile, or battery power, changes
  the clocks.
- **Background agents.** Agent builds raised the load to 19.4 at the reading.
  Any load above 1 breaks the condition of
  [owner-decisions](../owner-decisions.md).
- **Disk space.** `/home` has 60 GB free. Long fuzzing writes corpora and
  artefacts; a full disk makes a run fail without a clear message.

## Pilot procedure

Run this before the full cycle. The owner's rule applies: no run counts unless
the load average stays below 1 for the whole measurement.

1. Stop agents and background builds.
2. Plug in to AC power (`cat /sys/class/power_supply/AC*/online` prints `1`).
3. Set the performance profile (`powerprofilesctl set performance`).
4. Record the configuration: the table above, `uname -r`, `lscpu`, `free -h`,
   `df -h /home`, and the toolchain versions.
5. Log `uptime` and the thermal zone temperature every 10 minutes for the
   whole pilot and the whole measured run.
6. **Abort criterion:** a load average of **1 or more** aborts the run. Record
   the run as inconclusive.
7. Record the pilot's load and duration, then start the full cycle with the
   durations set in [owner-decisions](../owner-decisions.md).
