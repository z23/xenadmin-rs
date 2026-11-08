# VM creation, templates and deletion increment

This increment adds the ordinary halted-VM and custom-template workflows recorded in
[the P08 ledger](../PARITY-IMPLEMENTATION.md). It continues the retained parity plan;
the ledger explicitly preserves remaining work and live qualification gates.

## Implemented workflows

- A three-step template wizard reviews name/description, CPU count, fixed memory,
  destination storage, network, optional additional disk, installation ISO and inherited,
  BIOS or UEFI firmware. Existing template disks are fully copied to the chosen SR;
  diskless templates use cloning before provisioning. All inherited NICs use the selected
  network, or one NIC is added if none exists. Omitting an ISO retains inherited media.
- Fast clone and full copy are separate choices for halted VMs and templates. Full copy
  selects a destination SR; either workflow can produce a VM or custom template.
- A halted ordinary VM can become a template after explicit review. Custom templates
  support name/description editing and deletion. Built-in templates are protected.
- Deletion defaults to removing only the VM/template record and its attachment records.
  The review separately selects exact owned data disks, halted snapshots and eligible
  snapshot data. It lists consumers and retained resources. Shared/unowned disks are
  never added automatically, and memory checkpoints use the dedicated snapshot flow.

The new VM is not started automatically. The completed creation configuration is checked
through the host's read-only compatibility query. Unsupported settings or no compatible
host leave an explicit retained partial result. Firmware changes preserve unrelated boot
parameters; unsupported template modes and BIOS with inherited secure boot are refused.

## Identity, ownership and recovery

Source pool, connection generation and UUIDs remain captured throughout editing and
review. Fresh host checks revalidate capabilities and resource ownership before each
mutation. Copy results must have new VM/device identities absent from persisted baselines
and a matching disk/network graph before being exposed as created resources. Newly
copied exclusive disks receive separately verified ownership so a retained created VM can
be deleted through the normal reviewed workflow without claiming its source disks.

Every step is persisted before dispatch. Lost replies remain unknown; read-only recheck
does not repeat writes or continue remaining steps. Active and unknown operations reserve
their created VM/device identities against conflicting commands from this client.
Creation and copying also reserve the entire pool from other mutations until they finish
or their unknown outcome is resolved or explicitly acknowledged. This covers the interval
before the client has learned which new resource identities the host created.
Cancellation can prevent later steps without proving that an accepted host task stopped.
There is no automatic rollback of previously completed changes.

The global **VM changes** view remains available after the source VM disappears and while
disconnected. It lists pool/source identity, created VM/device UUIDs, step outcomes and
recheck/cancellation/unknown-acknowledgment controls. Controls require the matching active
writable pool. Shared desktop history retains the newest 100 terminal records and all
unresolved records across connection switches; a synthetic demo clears this private state.
The created VM's inspector also shows its creation operation.

## Remaining scope and verification

Copied host BIOS identity, ordinary cloud-init configuration-drive/transfer workflows,
broader custom-template property editing and standalone detached-orphan cleanup remain
pending. ISO insertion retains the template's boot order; legacy PV/eliloader installation
repository and kernel-argument configuration are not added by this wizard. The legacy
CoreOS/xscontainer plugin path is excluded with specialized guests;
that exclusion does not remove ordinary cloud-init from the retained plan.

All 55 desktop tests passed, including eight new lifecycle tests. They exercise the wizard
at wide and compact widths, ISO choice, exact clone/copy/deletion targets, template
protection, changing ownership, stale/pool context, invalid resource values and offline
history after source deletion. Strict desktop all-target Clippy passed.

The final workspace run passed 212 tests: 116 core unit tests, five connection-profile
tests, 28 direct-host protocol tests, four TLS tests, four host-probe tests and 55 desktop
tests. The 25 new backend lifecycle cases cover copied-resource identities and ownership,
explicit deletion, task-poll uncertainty, lost replies, read-only restart reconciliation,
partial outcomes, concurrent metadata edits, firmware refusal and creation reservations
in both admission directions. Workspace strict Clippy, formatting and diff checks passed.
The synthetic `--scene vm-create` wizard rendered with Rune's Vulkan backend at 1440×900
and was visually inspected. The image is outside the repository.

The workflow follows the documented [XAPI VM clone/copy/provision APIs](https://xapi-project.github.io/xen-api/classes/vm.html)
and the reference client's `XenModel/Actions/VM/CreateVMAction.cs`: remove the inherited
`other_config.disks` entry and create explicitly reviewed disks rather than interpreting
template provisioning XML. The [XAPI clone implementation](https://github.com/xapi-project/xen-api/blob/master/ocaml/xapi/xapi_vm_clone.ml)
clears the built-in-template marker; this client verifies that property and checks fresh
UUIDs because cloned records do not retain source-parent lineage. No reference source was copied.

All fixtures and UI previews use synthetic inventory. No production-host changes or
private captures were used. These checks do not establish actual XCP-ng compatibility,
native desktop accessibility or the required disposable-lab destructive qualification.
