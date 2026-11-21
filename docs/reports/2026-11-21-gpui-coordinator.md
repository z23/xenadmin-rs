# Coordinator handover and maintenance through the GPUI front end

2026-11-21. The operator authorized these two checks on the disposable lab. They are
the ignored test `lab_p3_coordinator_handover_and_maintenance` in
`crates/xenadmin-desktop/tests/lab_live.rs`: the real shell and core, no window, on
the Kit's headless platform. The lab hosts were started for the run. High availability
was off. Evidence is
`evidence/gpui/1795227389-p3-coordinator-handover-and-maintenance` beside the lab
manifest, outside this repository. The summary says passed, 95.3 seconds, application
`0.20.0`.

What the run did:

- Connected to `xenadmin-lab-9e8e4f49-h1` with pinned certificates. That host was the
  coordinator; `h2` was the enabled member.
- Hand Over Coordinator, confirmed with the pool UUID, made `h2` the coordinator in
  34.1 seconds. The same command made `h1` the coordinator again in 35.0 seconds.
  Both hosts were enabled after each leg, and the connection was current.
- Enter Maintenance Mode on `h1`, then Exit Maintenance Mode. The review listed no
  resident guest. Enter finished in 0.1 seconds and exit in 1.0 seconds, each adding
  one Succeeded resource operation. This run did not evacuate a guest.
- The role ended on `h1`, and both hosts were enabled.

Not this run: a window, a hardware GPU, a keyring, the host's own refusal of an
evacuation, and an operation still in flight when a toolstack restarts.
