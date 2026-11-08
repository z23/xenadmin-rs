# Reviewed storage repair and read-cache policy

This increment implements the retained P09-07 multi-host repair workflow for existing attachments and the supported P09-14 read-cache policy controls. It uses direct XAPI and requires no Xen Orchestra service. Verification is synthetic; no production or disposable-lab storage mutation was performed.

## Behavior

Storage administration lets the operator select exact existing detached host attachments, then review their host/PBD UUIDs and safe location fields. The selected coordinator runs first. Each host task is separately journalled and verified; a failure stops later hosts, preserving earlier successful attachments. Local stop prevents remaining steps without claiming cancellation of host I/O. The UI distinguishes attached status from storage health.

The backend verifies pool, coordinator, SR/backend, host and PBD identities and configuration before submission. A private SHA-256 configuration fingerprint binds later steps to the prepared attachment, including configuration omitted from public projections. Neither raw configuration nor credentials are persisted or displayed. Existing authenticated attachments use their existing host credentials; this workflow does not copy or replace Secrets. Missing attachments are still created through the separate explicit location/credential workflow.

NFS, EXT and SMB data drivers must explicitly advertise `VDI_READ_CACHING` before a cache policy change is accepted. Enabling removes the captured `o_direct=true` override; disabling writes `o_direct=true`. Only that key is changed, preserving unrelated settings. A prior `false` override is removed before adding `true`, with independent receipts and an honest partial result if the second call fails. The review describes policy only: it does not reopen disks, restart guests, establish effective caching or convert the storage provisioning mode.

Driver `THIN_PROVISIONING` capability is displayed separately. Absence is labelled “not advertised”, not inferred as a complete description of backend allocation behavior.

## Recovery and privacy

A private immutable plan captures the reviewed operation, order, attachment fingerprints and bounded pre-existing task UUIDs. Task references remain private; each task UUID must be new and distinct from other steps. Public action/step shape and completion evidence are validated during journal load, admission and save. Public record serialization excludes the plan.

Missing replies are not inferred from an attached PBD or matching policy value. Failed or cancelled host tasks remain uncertain because they can have changed storage state. Late task/verification evidence is merged without reviving recovered or acknowledged work. Explicit recheck inspects previously submitted steps only; it never submits remaining hosts or policy writes. A successful task receipt plus exact fresh attachment state is required before recording a verified host result.

## Source audit and retained scope

Primary sources consulted:

- [XCP-ng storage documentation](https://docs.xcp-ng.org/storage/) describes local EXT/LVM, NFS, iSCSI/HBA and supported versus as-is drivers. Local device creation can format its target; it needs typed discovery and a separate destructive review, not an arbitrary device-config editor.
- [XCP-ng 8.3 SM package](https://github.com/xcp-ng-rpms/sm/blob/8.3/SPECS/sm.spec) identifies SM v3.2.12 and downstream patches, including the distinction between read caching and SR caching.
- [SM v3.2.12 blktap2](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/blktap2.py) applies `o_direct` while opening disk backing storage and independently considers driver, licensing and VDI constraints.
- [NFS driver](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/NFSSR.py) and [EXT driver](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/EXTSR.py) advertise read caching and thin provisioning capabilities.
- [LVHD over iSCSI](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDoISCSISR.py) and [LVHD over HBA](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDoHBASR.py) expose a thick/thin allocation parameter, with additional LVHDSR persisted metadata and thin-driver dependencies. Qualified allocation-mode creation remains a concrete follow-up; a missing THIN_PROVISIONING feature is not treated as proof that this parameter is unsupported.
- The local XenAdmin reference `XenModel/Actions/SR/SrRepairAction.cs` orders the coordinator first and creates/plugs host attachments. This implementation deliberately keeps missing-PBD creation explicit instead of copying credential-bearing device_config. `XenModel/XenAPI-Extensions/SR.cs` and `SrReadCachingEditPage.cs` confirm the `o_direct` policy convention. No reference source was copied.

Still retained: local EXT/LVM and HBA/Fibre Channel provisioning/discovery; further backend-specific allocation options; as-is driver wizards where supported; automatic missing-PBD graph reconstruction; maintenance-aware multipath detach/replug orchestration. This increment does not mark those capabilities complete. Existing typed NFS/ISO/SMB3/CHAP workflows remain unchanged.

## Verification

- Eleven synthetic protocol/journal test groups cover coordinator order, configured authenticated attachments, changed identity/configuration/coordinator, later refusal, lost reply, failed task, task UUID reuse, recovery/acknowledgement, explicit recheck without remaining writes, permission denial, capability-gated cache changes, unrelated map preservation malformed private plans at load/admission/save, and duplicate operation-ID admission returning the original durable result.
- Five Rune interaction tests cover captured targets, no demo dispatch, changed graph/policy refusal, explicit checkbox selection, and review/footer reachability at 1440×900 and 513×652.
- Full workspace: 937 tests passed (423 core unit/protocol, 256 direct-connection, 245 desktop, five profile, four TLS and four host-probe tests).
- Strict workspace/all-target Clippy and native workspace build passed. Six native Vulkan screenshots were rendered and visually inspected: attachment picker, repair review and cache review at 1440×900 and 513×652. Headings/footer controls are visible; long bodies scroll.
- Independent source review by the host workflow agent found no blocking issue in the backend/journal paths; the reviewer did not independently run fixtures.

Live XCP-ng storage-driver behavior, real authentication/mounting and effective caching remain deferred to the designated virtualized lab. Synthetic previews contain invented data only and are not committed.
