# Guest tools installation media

Implemented and locally verified for retained row P06-11. Live XCP-ng/guest qualification
is deferred until the prepared disposable lab is available.

## Workflow

The VM details and devices views open a guest-tools media chooser. It identifies tools
ISOs using the host's `is_tools_iso` flag or the reference client's three recognized legacy
filenames. The source of that classification is shown; it is not an authenticity check.
The operator explicitly selects the ISO and reviews its UUID, SR, CD drive and any media
that will be ejected. Running and halted ordinary VMs are supported. Guest-agent status,
OS and addresses remain available through the existing information views.

The operation resolves the captured pool/VM/CD/VDI/SR identities, requires idle VM/drive,
checks the CD operation advertised by XAPI, and verifies that the read-only ISO exists on
an attached SR accessible to the running host (or an enabled host for a halted VM). Those
checks run before ejecting and again before insertion. Known VM work is also explained in
the chooser's review. The selected CD must still contain the exact reviewed prior image.

Eject and insert are separate durable steps. A failed insertion may leave an empty drive;
there is no automatic remount or rollback. Lost replies remain explicit unknown outcomes.
Rechecking reads postconditions and never replays a write or continues an interrupted
sequence. The existing permission review and backend permission guard cover both methods.
Changing UI selection or switching pools cannot retarget a captured review; stale
connection generations cannot submit it. Live operation handles retain the desktop's cap.

Mounting is followed by installation inside the guest. The client does not execute the
installer or claim it ran. The view links the [official guest tools documentation](https://docs.xcp-ng.org/vms/).
It does not silently repair a disconnected tools SR; the storage workflow remains the
explicit place to repair connectivity. No production guest or host was modified.

## Local verification

- 795 full-workspace tests passed on the slice: 353 core unit, 223 direct protocol,
  206 Rune, 5 profile, 4 TLS and 4 probe tests.
- New fixtures cover empty/occupied CDs, exact eject/insert order, foreign ownership,
  changed prior image, missing/writable ISO, disconnected PBD, template/busy VM,
  wrong SR, insertion refusal, changed media between steps, lost replies and no replay.
- Inventory fixtures distinguish the host flag, known legacy names and unrecognized media.
- Rune fixtures cover explicit media choice, immutable reviewed identity, cross-pool
  device-view rejection, replacement/no-device explanations, disabled synthetic submission
  and reachable controls at 513×652 and 1440×900.
- Strict workspace/all-target Clippy, formatting and diff checks passed.
- Native synthetic chooser/review renders at both sizes were inspected. A clipped compact
  filename found during inspection was corrected to wrapped details and a short button.
- Independent source review found no backend blocker. Its known-busy UX suggestion was
  incorporated. Separate host/topology fixture synchronization fixes were used for the
  full run; they do not change product mutation behavior.

Actual media availability, running-guest CD behavior, package installation, Windows/Linux
agent reporting and XO-unreachable live acceptance remain unqualified.
