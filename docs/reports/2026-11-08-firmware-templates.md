# Guest firmware, Secure Boot policy and custom-template properties

This development slice adds reviewed firmware and BIOS identity choices to VM creation,
and extends the properties editor to custom templates. It uses direct XAPI; no Xen
Orchestra service or plugin is required. Evidence below is synthetic. Live XCP-ng and
native display qualification remain open.

## Implemented behavior

- Creation offers inherited firmware, BIOS, UEFI with Secure Boot off, or UEFI with
  Secure Boot on. Existing template data disks cannot change BIOS/UEFI mode. An empty
  eligible HVM template can choose its installation firmware explicitly. Ordinary VM
  property editing never switches installed guests between BIOS and UEFI.
- Secure Boot policy can be changed for a halted UEFI VM or custom template. Enabling
  requires the host's recognized `VM.get_secureboot_readiness` API before mutation;
  unsupported APIs and permission refusal remain explicit errors. This deliberately
  qualifies the XCP-ng 8.3 readiness path rather than assuming that accepting a platform
  map key establishes support on older versions. Unrelated platform keys are preserved.
- A read-only boot-readiness query reports guest certificate state and pool certificate
  defaults separately from the requested policy. Setup Mode, missing dbx, incomplete
  certificates, first boot, disabled policy and unsupported/unavailable queries have
  distinct descriptions. The query is bounded, generation/identity fenced and rechecks
  policy/NVRAM after sampling. It exposes no raw NVRAM or certificate content.
- Readiness means certificate presence, not measured guest enforcement. The UI says to
  verify enforcement inside the guest. No certificate propagation, NVRAM reset, TPM
  operation or automatic pool certificate replacement is performed.
- Eligible BIOS/HVM creation can copy BIOS identity from an explicitly selected pool
  host. The captured review names that host and explains that the API also sets home-host
  affinity. Source and destination BIOS identities must be empty; the chosen host UUID
  and privately captured BIOS map are revalidated, and both copied identity and affinity
  are checked before provisioning. Generic/inherited identity remains the default.
- Custom templates use the ordinary property editor for names/descriptions, halted CPU
  and memory limits, topology, scheduling weight, supported shadow memory, boot order,
  autostart, home-host preference and Secure Boot policy. Empty custom templates can
  explicitly review a BIOS/UEFI change; attached devices prevent it. Built-in templates
  remain read-only. Live-only edits cannot be constructed for templates.

## Identity, persistence and uncertainty

Settings commands carry an explicit VM/custom-template kind. Legacy journal settings
without that field still mean ordinary VM. Fresh reads enforce kind, ordinary-object
identity, default-template protection and no active operation at admission, each setter
and reconciliation. Converting a template while an edit is in flight prevents subsequent
writes. Existing VM power/device APIs do not acquire template authority.

Creation's optional BIOS host defaults to absent in legacy serialized commands. The
private lifecycle plan holds the BIOS baseline; public operation serialization does not
publish it. Journal updates enforce immutable source/baselines and monotonic task/result
receipts and created identities. Lifecycle receipt persistence preserves recovery and
acknowledgement states, and stops subsequent writes after either. Shared step dispatch
now refuses recovered/terminal rows and uses a single compare-and-swap before sending.
A lost BIOS-copy reply leaves an unknown outcome and a retained created VM; it never
implicitly provisions or repeats the copy. Read-only reconciliation does not submit
pending workflow steps. Partial resources remain visible for operator review.

## Verification

All 553 workspace tests passed: 4 host-probe, 257 core unit, 5 profile,
156 direct protocol, 4 TLS and 127 desktop tests. Strict workspace all-target Clippy,
formatting, diff and check-script syntax checks passed. The desktop binary built and
all four synthetic Vulkan renders were visually inspected. New coverage includes legacy command decoding, distinct
VM/template targeting, built-in protection, empty/installed-template firmware gates,
Secure Boot policy/readiness separation, missing API refusal, read-only readiness,
map-key preservation, changed-kind interruption, lost setter replies, copied identity and
affinity, and late BIOS-copy receipts after recovery or acknowledgement.

Rune interactions exercise compact (513 × 652) and wide (1440 × 900) forms, captured VM
and host identities, explicit policy review, blocked installed-image flips, built-in
read-only behavior and disabled synthetic submission. Four synthetic Vulkan renders
cover `firmware-create` and `secure-boot` at both sizes; the scenes are included in the
normal target-aware check script. No supplied screenshots or private inventories are used.

## Retained work and host qualification

Ordinary cloud-init configuration-drive generation/transfer is retained and is not part
of this slice. Neither guest certificate readiness nor a matching policy flag proves
actual boot enforcement, which requires a disposable guest test. Older host versions
without the readiness API fail the explicit enabling workflow; support for an alternative
capability probe needs separate qualification. Real firmware boot, installer behavior,
copied vendor identity, guest OS Secure Boot, and native Wayland/X11 accessibility remain
lab/platform gates.

## Primary/reference audit

- [XCP-ng advanced VM settings](https://docs.xcp-ng.org/vms/advanced/) warns against
  switching installed guests between BIOS and UEFI.
- [XCP-ng guest UEFI Secure Boot](https://docs.xcp-ng.org/guides/guest-UEFI-Secure-Boot/)
  distinguishes policy from Setup Mode and documents certificate defaults/updates.
- [XCP-ng 8.3 VM implementation](https://github.com/xcp-ng/xen-api/blob/8.3/ocaml/xapi/xapi_vm.ml)
  provides `get_secureboot_readiness`; its output reflects PK/KEK/db/dbx presence.
- [XCP-ng 8.3 pool implementation](https://github.com/xcp-ng/xen-api/blob/8.3/ocaml/xapi/xapi_pool.ml)
  provides `get_guest_secureboot_readiness` from active pool certificate defaults.
- [XCP-ng 8.3 VM helpers](https://github.com/xcp-ng/xen-api/blob/8.3/ocaml/xapi/xapi_vm_helpers.ml)
  establish the empty-BIOS requirement and the affinity side effect of `copy_bios_strings`.
- [XCP-ng 8.3 xenops adapter](https://github.com/xcp-ng/xen-api/blob/8.3/ocaml/xapi/xapi_xenops.ml)
  selects firmware from `HVM_boot_params`; the client also accepts modern HVM domain type.

Reference behavior was studied; no upstream implementation code was copied.
