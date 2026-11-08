# Cloud-init in the creation wizard

Implemented the retained P08-03 cloud-configuration option using the existing NoCloud drive
workflow. This is locally verified; live guest and hypervisor qualification remains deferred.

## Behavior

The template creation wizard has an optional fourth page for hostname, user-data and
network-config. Inputs use the existing bounded NoCloud validation. Review shows their
byte counts and hostname without copying configuration contents into the creation command,
operation history or connection profile. The draft stays in UI memory while creation runs.

After successful creation, a continuation panel displays the verified created VM UUID.
Its identity checks bind the admitted operation ID, pool, source-template UUID, immutable
creation parameters, original connection observation and resulting VM UUID. The current
result must remain an ordinary halted VM with no reported operation. Selection changes
cannot redirect the workflow. A reconnect, failed/partial/unknown/acknowledged result,
missing identity, running VM or missing SR blocks this continuation.

“Review configuration drive” hands the private inputs to the existing NoCloud media worker
for that exact VM and chosen SR. It chooses a currently unused device slot, prepares local
private media, then requires the existing upload/attachment review. Its backend rechecks
identity, permissions, storage and slot before mutation. This is two tracked stages, not
an atomic transaction. Neither stage starts a VM or establishes that cloud-init executed.
The template must already contain cloud-init with NoCloud support.

Closing the wizard discards its unsaved draft and leaves any admitted creation running.
The creation result remains in VM changes; the operator can configure the retained VM
separately. Disabling the option also discards its draft. No automatic continuation is
restored after application restart. The current user-data/network strings are zeroized on
draft drop; this does not claim comprehensive erasure of UI/framework allocation history.

The VM lifecycle dialog now keeps its buttons outside the scrolling form so creation,
clone and deletion reviews remain operable in compact windows.

## Verification

- All 207 desktop tests passed after the fixed-footer change, including existing creation,
  clone, deletion, cloud media and local reconnect fixtures. The added optional-page
  navigation/discard case and all four creation/cloud continuation cases then passed.
- Cases cover invalid input, private payload omission from review and commands, exact
  operation/pool/action/source/result matching, failed/unknown/partial outcomes, selection
  changes, missing current generation, running created VM and synthetic no-write behavior.
- The media handoff test verifies the exact VM/SR, private preparation, payload removal from
  the form after preparation and no automatic upload.
- Strict workspace/all-target Clippy, formatting and diff checks passed. Independent source
  review found no identity, privacy or continuation blocker.
- Native synthetic input and continuation renders were inspected at 1440×900 and 513×652.
  The fixed footer was added after the first compact render showed controls below the fold.

No production VM was created or configured. Actual template provisioning, NoCloud guest
consumption, restart/first-boot behavior and XO-unreachable acceptance still require the
prepared disposable lab.
