# HBA discovery, creation and allocation choices

This increment implements typed HBA/Fibre Channel discovery and reviewed LVM-HBA creation,
plus driver-advertised thick/thin allocation for new LVM-iSCSI and LVM-HBA repositories.
It is fixture-verified implementation, not live storage qualification.

## Workflow

Storage administration now offers LVM over HBA/Fibre Channel. Discovery runs through the
selected authenticated host and returns bounded SCSI ID, capacity, vendor and serial fields.
Selecting a result fills the form; it does not create storage. The review captures the selected
host and exact device, explains permanent data loss and external ownership limits, and requires
the LUN SCSI ID to be typed before submission. It is reachable at compact and wide widths.

The backend requires a supported probing driver, a current exact host, a complete scan of
registered SR/PBD ownership (including detached attachments), no repository reported on the
LUN, and a fresh matching discovery result. The host driver independently omits its root disk
and registered LUNs. These checks do not prove that another pool, storage appliance or operating
system is not using the LUN. Only an operator can establish that external ownership before
formatting. No existing SR is automatically forgotten, destroyed or reformatted.

New iSCSI/HBA repositories offer driver default, thick and thin allocation. Explicit modes
require the driver's `allocation` configuration capability both in the form and fresh backend
preflight. The request sets the device configuration, then verifies the driver's persisted
`SR.sm_config` and created attachment. Existing repositories are never converted; introduce
continues to register an existing UUID without formatting. Thin allocation is described as
growing VHD disk usage, with raw disks remaining fully allocated and the need to monitor free
space; it does not imply unlimited capacity.
Authenticated iSCSI uses the existing transient CHAP/mutual-CHAP and server Secret workflow.

## Identity, recovery and privacy

The reviewed HBA tuple and allocation choice join the existing immutable action and
pool/generation observation. The durable single-task storage worker excludes pre-existing SR
and attachment identities from its creation result. Successful task completion and fresh
identity/configuration verification are required before success. Failed/cancelled creation,
lost responses and mismatched allocation retain an unknown outcome and its reservation; a
recheck cannot submit another create or erase a LUN again. No cleanup of unowned storage is
introduced.

Discovery accepts bounded XML only from the intended known probe error codes. Driver stderr,
raw configuration and credentials are not public errors. HBA projection omits device paths,
adapter details and arbitrary XML fields. Legacy iSCSI locations and create actions retain
driver-default behavior and their prior serialized representation.

## Primary evidence

- [XCP-ng 8.3 SM package](https://github.com/xcp-ng-rpms/sm/blob/8.3/SPECS/sm.spec)
  identifies SM v3.2.12 and packages the `lvhd-thin` plugin. Qualification still depends on
  the installed driver's advertised capabilities and actual host behavior.
- [LVHD over iSCSI](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDoISCSISR.py)
  and [LVHD over HBA](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDoHBASR.py)
  advertise `allocation` with `thin`/`thick` values. A missing `THIN_PROVISIONING` feature alone
  is not treated as evidence that this explicit option is unsupported.
- [LVHDSR](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/LVHDSR.py) reads allocation
  from device configuration and persists it in SR metadata; existing SR metadata takes precedence.
- [HBA discovery](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/HBASR.py) and
  [device scanning](https://github.com/xapi-project/sm/blob/v3.2.12/drivers/devscan.py) return
  `Devlist`/`BlockDevice` records and exclude the host root disk and known registered SCSI IDs.
- The local XenAdmin `FibreChannelProbeAction`, `FibreChannelDevice` and `LVMoHBA` wizard
  establish the missing-SCSI-ID probe convention and selected-LUN repository probing.
  No reference implementation source was copied.

## Verification and retained work

- Five new synthetic protocol test groups cover HBA discovery/creation with all three allocation
  choices, changed/used/existing/malformed device refusal, unsupported allocation and mismatched
  readback, lost/failed/cancelled tasks with recheck without reformatting, and authenticated thin
  iSCSI with CHAP/mutual CHAP and credential privacy.
- Two model/decoder test groups cover legacy action/location roundtrip, mandatory HBA selection,
  invalid allocation and ambiguous, duplicate or unbounded discovery XML.
- Four Rune interaction test groups cover selection-before-review, captured host/device/allocation,
  stale discovery refusal, exact confirmation reachability and disabled synthetic submission at
  1440×900 and 513×652.
- Full workspace: 969 tests passed (434 core unit/protocol, 267 direct-connection, 255 desktop,
  five profile, four TLS and four host-probe tests). Strict workspace/all-target Clippy passed.
- Native workspace build and four synthetic Vulkan renders passed; discovery and destructive
  review were visually inspected at both widths. Heading/footer controls remain visible and
  the long review body scrolls. Render artifacts are private temporary files, not repository data.
- Independent source review found no blocking issue in the new backend/model/discovery/UI paths;
  the reviewer did not independently rerun fixtures.

Local EXT/LVM device discovery/creation, other supported/as-is backend wizards, automatic
missing-PBD graph reconstruction and complete multipath maintenance orchestration remain
retained work. Existing-SR allocation conversion is not provided by this increment. Real
array zoning, multipath behavior, device identity stability, thin allocation and data integrity
remain deferred to the designated virtualized lab. All test data and renders are synthetic;
no private screenshots or live inventory were published.
