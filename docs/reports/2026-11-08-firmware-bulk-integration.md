# Firmware and bulk power integration

The firmware/custom-template increment and bulk power backend/UI are integrated after the
0.8 checkpoint. The OVF summary display now handles the new Secure UEFI enum variant;
OVF export still explicitly refuses Secure Boot guests rather than silently downgrading
their policy. The existing OVF parser accepts only its documented BIOS/UEFI subset.

The complete `scripts/check.sh` run passed 617 workspace tests (4 probe, 302 core unit,
5 profiles, 159 direct protocol, 4 TLS and 143 desktop), 46 vendored console tests and
8 lab helper tests. Formatting, strict Clippy, workspace compilation and 64 synthetic
renders passed. Wide/compact firmware and bulk reviews were rendered; compact reviews and
the normal device page were inspected. Earlier source-worktree counts remain historical
evidence, not the current integrated count.

These checks use local protocol fixtures and synthetic inventory. No production host
mutation or live XCP-ng qualification was performed. The prepared offline lab awaits
virtualization as requested. The existing 0.8 release archive is unchanged and does not
contain these subsequent commits.
