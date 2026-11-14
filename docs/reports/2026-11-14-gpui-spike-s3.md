# GPUI spike S3: runtime bridge and scale

Result: **partial**. The bridge, a 16-host / 2,000-VM inventory, and a headless tree and table ran for 60 seconds. Scrolling on a real display was not measured. `Backend` was started and never connected.

## What ran

Ubuntu 24.04.5, kernel 6.12.113+. No host, lab, or production machine.

- `WakeBridge` coalesces wakes with an `AtomicBool` and `std::sync::mpsc`. The receiving side does not use Tokio. `grep block_on` over `crates/xenadmin-gpui` finds none.
- `ScaleModel::demonstration(16, 2_000)` builds a public `xenadmin_core::Inventory` with the same host and VM counts as the Rune `large-inventory` scene. The Rune scene function was not moved. Snapshot details from `demo_details` are not filled in.
- A thread calls `touch` every 10 ms and then the waker. The foreground renders only after `poll_wake`.
- `Backend::with_state_directory` starts the core runtime on a temporary directory. `connect` is not called. Shutdown order printed by the example: updater stopped, bridge dropped, backend dropped.
- The window is a `Tree` of the hosts and a `DataTable` of the VMs, opened with `show: false` through `HeadlessAppContext`. No headless renderer is installed, so this is layout and the test harness frame, not a Vulkan frame.

`XENADMIN_SPIKE_SECONDS=60 cargo run -p xenadmin-gpui --example s3_scale --features scenes`:

```text
seconds=60 frames=5450 frame_us_p50=10173 frame_us_p95=16003
selection_held=true
selected=Some(VmId(30000000-0000-0000-0000-000000000000))
idle_frames=0
rss_kib_before=59168 rss_kib_after=59540
shutdown=updater_stopped,bridge_dropped,backend_dropped
```

Wall time was 60.7 seconds. The selected VM id was still in the inventory after every applied update. After the updater stopped and queued wakes were drained, 200 ms of polling rendered nothing. Resident set grew by 372 KiB over the minute. That is the whole process, not a console framebuffer.

A unit test checks the same selection property without GPUI. A five-second run earlier in the same session reported p50 9.0 ms and p95 9.5 ms, RSS 58,656 KiB to 58,920 KiB.

## What this does not establish

The idle count means the spike did not call `render_frame` when the bridge was quiet. It does not mean the compositor skipped a frame. Frame times are headless test frames, not Wayland scrolling. No pool was connected, so this is not a live inventory. Headless output does not establish Wayland behavior.
