# Integrated 0.13 development progress

The combined source at `6e0464a` includes reviewed HBA provisioning and LVM allocation choices, complete XVA disk-stream validation, guest SSH/RDP and trusted local tool launch, reviewed guest NIC migration during management bond changes, and whole-row/keyboard inventory context menus.

The workspace passed 1,019 tests: 450 core unit, 278 direct-protocol, 278 Rune, 5 profile, 4 TLS and 4 probe. Strict workspace/all-target Clippy, formatting and diff checks passed. Individual feature reports record their synthetic native renders, failure fixtures and supported boundaries.

These changes remain development work after the immutable 0.12 package checkpoint. No 0.13 package or release was published. The retained parity scope is still open; checkpoint archive handling, local disk provisioning, additional management topologies, grouped workflows and other ledger items continue separately.

No production mutation or private screenshot was used for these checks. The local disposable lab is prepared offline; live infrastructure and real desktop qualification remain deferred until virtualization is available.
