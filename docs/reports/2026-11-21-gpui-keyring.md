# A saved password restored from a private keyring

2026-11-21. The ignored test `lab_p3_keyring_restore` in
`crates/xenadmin-desktop/tests/lab_live.rs` ran the real shell and core, with no
window, on the Kit's headless platform. Evidence is
`evidence/gpui/1795229815-p3-keyring-restore` beside the lab manifest, outside
this repository. The summary says passed, 14.1 seconds, application `0.20.0`.

The Secret Service provider was a `gnome-keyring-daemon` started for this run
with its own runtime directory. The operator's keyring daemon stayed the one
whose control socket is `/run/user/1000/keyring/control`.

What the run did:

- Add Server to `192.168.254.11`, with both certificates pinned. "Remember
  password in desktop keyring" and "Connect automatically when XenAdmin RS
  starts" were on. Connect took 0.2 seconds once the certificates were pinned.
- At 13.1 seconds the saved connection was set to start automatically and to
  keep its password in the keyring. The state directory did not contain the
  password.
- The process was closed. A second process, on the same state directory, was
  connected at 14.0 seconds. Nothing was typed. The status bar said "Connected
  directly • https://192.168.254.11/".

The 14.1 seconds are the whole scenario, most of it the two certificate
reviews. The second start's login is the interval from 13.1 seconds to 14.0
seconds.

Not this run: a window, a hardware GPU, the operator's own keyring, and a
locked keyring.
