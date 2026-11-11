# Performance baseline, synthetic inventory

A first measurement of the desktop client on an inventory larger than the lab's, taken so
that later changes have something to be compared with. It sets no budget. It measures the
client's own work on synthetic data: no host, no network and no console are involved.

## What was measured

- Application: workspace version 0.18.0 in development, release profile, after the commit
  that adds this report.
- Dataset: one synthetic pool of 16 hosts and 2,000 guests (`demo_sized(s, 16, 2000)`, the
  demo generator at a larger size), four in five guests running, spread evenly over the
  hosts. One window at 1440 by 900 points, scale 1, dark theme, guest table showing.
- Workstation: AMD Ryzen 9 7940HS (16 threads), 46 GiB RAM, Linux 7.2.15, Mesa 26.2.5.
  Vulkan lists an AMD Radeon 780M (RADV) and an NVIDIA device; the headless renderer does
  not log which adapter it took.
- Method, CPU side: the ignored test `measure_large_inventory` in the desktop crate, run
  once with `cargo test --release -p xenadmin-desktop -- --ignored --nocapture
  measure_large_inventory`. It times each step with a monotonic clock and prints the median
  and the worst of its runs.
- Method, with the GPU: the release binary rendering the `large-inventory` scene headlessly
  for two seconds of frames, three times; the figures are the renderer's own.

## Results

| Step | Median | Worst | Runs |
| --- | --- | --- | --- |
| Build the inventory, its tree and its rows | 3.8 ms | 6.9 ms | 5 |
| Rebuild the tree after an inventory change | 2.1 ms | 2.3 ms | 20 |
| Filter 2,000 rows by a name fragment | 0.64 ms | 2.0 ms | 50 |
| Clear the filter | 1.6 ms | 1.8 ms | 50 |
| First frame on the CPU (view, layout, display list) | 10.3 ms | | 1 |
| Later frames on the CPU, nothing changing | 0.36 ms | 4.7 ms (95th percentile 0.39 ms) | 120 |

Headless renders of the same scene with the GPU, three runs of 121 frames each: renderer
start 175 to 260 ms, first frame 12.6 to 14.1 ms, later frames 0.48 to 0.52 ms at the
median and 1.0 to 1.1 ms at the 95th percentile, 1,938 drawn instances.

The table draws only the rows in view, so the frame cost does not grow with the row count;
the tree and the row list are rebuilt whole on each inventory change, which is the 2 ms
figure.

## What this does not show

- Nothing about a live host: login, the first inventory download, event batches, task
  polling and console decoding were not measured. On a real pool of tens of guests the
  operator reported the client responsive; that was not timed.
- Nothing about memory use, a long-running session, several pools at once, or a console
  open while the inventory changes.
- One workstation, one window size, one run of the CPU test. The figures are a starting
  point for comparison, not a claim about other machines.
