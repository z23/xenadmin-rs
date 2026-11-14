# GPUI migration ledger

Phase 3. The Rune binary is still the released artifact. Supported areas in the GPUI shell are headless previews and do not call a host. The GPUI shell in `xenadmin-gpui` draws the gate scenes headlessly. It does not connect to a pool. Catalogued Rune buttons below stay on the Rune front end until a later phase ports that surface. `xenadmin-desktop` is frozen except for release-blocking fixes; each such fix adds a mirror row.

## Commands

| CommandId | label | menu path | object kinds | Rune source | model status | GPUI status | package |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `file.import` | Import | File | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `file.exit` | Exit | File | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `pool.new` | New Pool | Pool | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `pool.add-server` | Add Server | Pool | see `commands.rs` | menu seed | specified | shell preview, no host | phase 2 |
| `pool.disconnect` | Disconnect | Pool | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `pool.properties` | Properties | Pool | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `server.reboot` | Reboot | Server | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `server.maintenance` | Enter or Exit Maintenance Mode | Server | see `commands.rs` | menu seed | specified | review dialog, no host | phase 3 |
| `vm.new` | New VM | VM | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `vm.start` | Start | VM | see `commands.rs` | menu seed | specified | review dialog, no host | phase 2 |
| `vm.shutdown` | Shut Down | VM | see `commands.rs` | menu seed | specified | review dialog, no host | phase 3 |
| `vm.migrate` | Migrate to Server | VM | see `commands.rs` | menu seed | specified | wizard preview, no host | phase 3 |
| `vm.snapshot` | Take a Snapshot | VM | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `vm.delete` | Delete VM | VM | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `storage.new` | New Storage | Storage | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `storage.repair` | Repair | Storage | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `templates.new` | New VM wizard | Templates | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `tools.options` | Options | Tools | see `commands.rs` | menu seed | specified | not started | phase 1 |
| `help.about` | About | Help | see `commands.rs` | menu seed | specified | not started | phase 1 |
| | Back | | | `activity.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck outcome | | | `activity.rs` Button::new | catalogued | not started | phase 1 |
| | Export filtered entries… | | | `activity.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `activity.rs` Button::new | catalogued | not started | phase 1 |
| | Export | | | `activity_tests.rs` Button::new | catalogued | not started | phase 1 |
| | Configure alarms | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm change | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | Reload settings | | | `alarms_view.rs` Button::new | catalogued | not started | phase 1 |
| | New vApp | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review name / description | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Delete empty vApp | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review order and delays | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review new vApp | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Apply reviewed change | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh | | | `appliance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Move selection to another pool… | | | `bulk_cross_pool.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `bulk_cross_pool.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_cross_pool.rs` Button::new | catalogued | not started | phase 1 |
| | Move reviewed VMs to the other pool | | | `bulk_cross_pool.rs` Button::new | catalogued | not started | phase 1 |
| | Delete selection… | | | `bulk_delete.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `bulk_delete.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_delete.rs` Button::new | catalogued | not started | phase 1 |
| | Delete reviewed targets | | | `bulk_delete.rs` Button::new | catalogued | not started | phase 1 |
| | Review deletions | | | `bulk_delete.rs` Button::new | catalogued | not started | phase 1 |
| | Export selection… | | | `bulk_export.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `bulk_export.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_export.rs` Button::new | catalogued | not started | phase 1 |
| | Review exports | | | `bulk_export.rs` Button::new | catalogued | not started | phase 1 |
| | Export selection… | | | `bulk_export_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Move selection… | | | `bulk_move.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `bulk_move.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_move.rs` Button::new | catalogued | not started | phase 1 |
| | Move reviewed VMs | | | `bulk_move.rs` Button::new | catalogued | not started | phase 1 |
| | Review moves | | | `bulk_move.rs` Button::new | catalogued | not started | phase 1 |
| | Power selection… | | | `bulk_power.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `bulk_power.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_power.rs` Button::new | catalogued | not started | phase 1 |
| | Run batch | | | `bulk_power.rs` Button::new | catalogued | not started | phase 1 |
| | Review batch | | | `bulk_power.rs` Button::new | catalogued | not started | phase 1 |
| | Edit selection… | | | `bulk_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `bulk_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `bulk_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Apply changes | | | `bulk_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Review changes | | | `bulk_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `cloud_init_view.rs` Button::new | catalogued | not started | phase 1 |
| | Discard and edit | | | `cloud_init_view.rs` Button::new | catalogued | not started | phase 1 |
| | Upload and attach drive | | | `cloud_init_view.rs` Button::new | catalogued | not started | phase 1 |
| | Prepare and review | | | `cloud_init_view.rs` Button::new | catalogued | not started | phase 1 |
| | Save | | | `console_export.rs` Button::new | catalogued | not started | phase 1 |
| | Reconnect | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Fullscreen | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Open in separate window | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Release input | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Save screenshot… | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Hold right button | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Type clipboard as keys | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Type it | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `console_view.rs` Button::new | catalogued | not started | phase 1 |
| | Show window | | | `console_windows.rs` Button::new | catalogued | not started | phase 1 |
| | Close console | | | `console_windows.rs` Button::new | catalogued | not started | phase 1 |
| | Done | | | `console_windows.rs` Button::new | catalogued | not started | phase 1 |
| | Close all consoles | | | `console_windows.rs` Button::new | catalogued | not started | phase 1 |
| | Move / copy to another pool… | | | `cross_pool_vm.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Apply reviewed transfer | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Next | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck both pools | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Acknowledge and release both pools | | | `cross_pool_vm_view.rs` Button::new | catalogued | not started | phase 1 |
| | Virtual disks… | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Grow, copy or clean up disk… | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Operation history | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | All disks | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Previous | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Next | | | `disk_workspace.rs` Button::new | catalogued | not started | phase 1 |
| | Guest SSH, RDP and tools… | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Configure local tools… | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Use a manually supplied address | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Review command | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Launch reviewed tool | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Save tool declarations | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Remove tool | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `external_tools.rs` Button::new | catalogued | not started | phase 1 |
| | Explain outcome… | | | `failure_help.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `failure_help.rs` Button::new | catalogued | not started | phase 1 |
| | Mount tools media | | | `guest_tools_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `guest_tools_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review mounting | | | `guest_tools_view.rs` Button::new | catalogued | not started | phase 1 |
| | Installation guide | | | `guest_tools_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `guest_tools_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review timeout default… | | | `ha_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Restore host timeout default… | | | `ha_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Host administration… | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Monitoring recording… | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Multipathing with storage reconnection… | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | BMC / power-on settings… | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm host change | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `host_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Open host text console | | | `host_console.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Previous sources | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Next sources | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review start recording | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review history deletion | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh catalog | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | History | | | `host_monitoring_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review unused Secret cleanup | | | `host_power_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `host_power_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `host_power_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm host change | | | `host_power_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `host_power_view.rs` Button::new | catalogued | not started | phase 1 |
| | Host backup and support… | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Filesystem backup | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Support bundle | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Stage recovery… | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Previous dumps | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Next dumps | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review deletion… | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Permanently delete crashdump | | | `host_support_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh from client | | | `inventory_information.rs` Button::new | catalogued | not started | phase 1 |
| | Pool information | | | `inventory_information.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `inventory_information.rs` Button::new | catalogued | not started | phase 1 |
| | Choose source image… | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Choose new output file… | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Previous history page | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Next history page | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review temporary file cleanup | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Remove recorded temporary files | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel local work | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Discard review | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Convert reviewed image | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Inspect and review | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review disk import… | | | `local_conversion_view.rs` Button::new | catalogued | not started | phase 1 |
| | Add connection | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Connect | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Cross-pool transfers | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Connections | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Details | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Options | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Help | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Alarms | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Convert local disk… | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Search pools | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Snapshot schedules | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Archives | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Organize | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Activity | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Disconnect | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Pool | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | New storage | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | New network | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Acknowledge outcome | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Template properties | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Disks and network interfaces | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Properties | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Snapshots | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Guest tools media | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | General information | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Check action availability | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck outcome | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Review unknown outcome | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Open console | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Certificate options | | | `main.rs` Button::new | catalogued | not started | phase 1 |
| | Staged IPv6 management bond… | | | `management_bond_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Create management bond… | | | `management_bond_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Remove management bond… | | | `management_bond_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Leave partial configuration… | | | `management_ipv6_bond_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Change management IP… | | | `management_network_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Move management… | | | `management_network_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Save current graph | | | `metric_presets.rs` Button::new | catalogued | not started | phase 1 |
| | Remove | | | `metric_presets.rs` Button::new | catalogued | not started | phase 1 |
| | Back to graphs | | | `metric_presets.rs` Button::new | catalogued | not started | phase 1 |
| | Performance | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back to inventory | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Data sources… | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Reconnect metrics | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Graph presets… | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Common sources | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Apply sources | | | `metrics_view.rs` Button::new | catalogued | not started | phase 1 |
| | Host networking… | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Add VLAN interface… | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Create bond… | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Change MTU… | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | IPv4 configuration… | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Operation history | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm network change | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Review network change | | | `network_admin.rs` Button::new | catalogued | not started | phase 1 |
| | IPv6 configuration… | | | `network_interfaces.rs` Button::new | catalogued | not started | phase 1 |
| | Interface purpose… | | | `network_interfaces.rs` Button::new | catalogued | not started | phase 1 |
| | Create pool-wide VLANs… | | | `network_topology_ui.rs` Button::new | catalogued | not started | phase 1 |
| | Retag pool-wide VLANs… | | | `network_topology_ui.rs` Button::new | catalogued | not started | phase 1 |
| | vApps · ordered VM groups | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Select filtered | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Clear selection | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Apply changes | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review changes | | | `organization_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `pool_dissolve_view.rs` Button::new | catalogued | not started | phase 1 |
| | Dissolve reviewed pool | | | `pool_dissolve_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh security status | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Reconnect as local administrator… | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review joining directory | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review leaving · local configuration only | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review leaving and remove machine accounts | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Resolve directory identity | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review adding access | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review removing subject access | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Validate selected PEM files | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review installing certificate | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review resetting to self-signed | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm change | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Pick | | | `pool_security_view.rs` Button::new | catalogued | not started | phase 1 |
| | Join another pool… | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Authentication, access and certificates… | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Change pool root password… | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review failure tolerance | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Dissolve pool… | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm change | | | `pool_view.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `profile_pool_join.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `profile_pool_join.rs` Button::new | catalogued | not started | phase 1 |
| | Join pool | | | `profile_pool_join.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `profile_reconnect.rs` Button::new | catalogued | not started | phase 1 |
| | Reconnect | | | `profile_reconnect.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `profile_root_password.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `profile_root_password.rs` Button::new | catalogued | not started | phase 1 |
| | Change password | | | `profile_root_password.rs` Button::new | catalogued | not started | phase 1 |
| | Review change | | | `profile_root_password.rs` Button::new | catalogued | not started | phase 1 |
| | Forget proxy password | | | `profile_trust.rs` Button::new | catalogued | not started | phase 1 |
| | Inspect certificate | | | `profile_trust.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `profile_trust.rs` Button::new | catalogued | not started | phase 1 |
| | Connections | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Saved connection options | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Choose CA certificate… | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Save without connecting | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Load saved password | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Forget saved password | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review this endpoint's certificate… | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | View pool | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Disconnect | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel connection attempt | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Connect / edit | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Remove | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Reconnect as another account… | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Connect all | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Disconnect all | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Add connection | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Choose CA | | | `profiles_view.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck outcome | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Acknowledge unknown outcome… | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Pool information | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Pool policies, HA and membership… | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Edit pool name and description | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Change default storage… | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Pool VM autostart… | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `resource_admin.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Search | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Filters | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Saved searches | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Apply filters | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Save | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Import… | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Export… | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Remove | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Add definitions | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Discard import | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Export resource results… | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Open in pool | | | `search_view.rs` Button::new | catalogued | not started | phase 1 |
| | Configure external tools… | | | `settings.rs` Button::new | catalogued | not started | phase 1 |
| | z23/xenadmin-rs on GitHub | | | `settings.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `settings.rs` Button::new | catalogued | not started | phase 1 |
| | Apply | | | `settings.rs` Button::new | catalogued | not started | phase 1 |
| | Load saved preview | | | `snapshot_inspection.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `snapshot_inspection.rs` Button::new | catalogued | not started | phase 1 |
| | New schedule | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Edit settings | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Run now | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Delete empty schedule | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Operation history | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm change | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `snapshot_policy_view.rs` Button::new | catalogued | not started | phase 1 |
| | Restore this connection… | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Open Activity history | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm storage changes | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review changes… | | | `storage_graph_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review selected repairs… | | | `storage_maintenance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh storage capabilities | | | `storage_maintenance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `storage_maintenance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm change | | | `storage_maintenance_view.rs` Button::new | catalogued | not started | phase 1 |
| | Use this result | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Previous | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Next | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Storage attachments and lifecycle… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Repair missing or detached host connections… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Remove detached attachment… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Create missing host attachment… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Forget repository… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Destroy repository and data… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back to storage form | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Create new | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Introduce existing | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review unused Secret cleanup… | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Delete unused temporary Secrets | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm storage change | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review | | | `storage_view.rs` Button::new | catalogued | not started | phase 1 |
| | Export… | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Export disk image | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | History | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Import image | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Stop transfer | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Release reservation | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Acknowledge after inspection | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | XVA | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Begin transfer | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Review transfer | | | `transfers.rs` Button::new | catalogued | not started | phase 1 |
| | Review configuration drive | | | `vm_creation_cloud.rs` Button::new | catalogued | not started | phase 1 |
| | Close and discard draft | | | `vm_creation_cloud.rs` Button::new | catalogued | not started | phase 1 |
| | Insert ISO… | | | `vm_device_actions.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_device_actions.rs` Button::new | catalogued | not started | phase 1 |
| | Apply change | | | `vm_device_actions.rs` Button::new | catalogued | not started | phase 1 |
| | Review change | | | `vm_device_actions.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_device_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_device_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Review change | | | `vm_device_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck outcome | | | `vm_device_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Disk properties… | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | NIC rate limit… | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Apply change | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Review change | | | `vm_device_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Back to inventory | | | `vm_devices.rs` Button::new | catalogued | not started | phase 1 |
| | Mount guest tools media | | | `vm_devices.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | VM changes | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck outcome | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Review unknown outcome acknowledgment… | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Stop remaining steps | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `vm_lifecycle.rs` Button::new | catalogued | not started | phase 1 |
| | Migrate / move VM | | | `vm_migration.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_migration.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_migration.rs` Button::new | catalogued | not started | phase 1 |
| | Apply migration | | | `vm_migration.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_placement.rs` Button::new | catalogued | not started | phase 1 |
| | Choose another host | | | `vm_placement.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh boot readiness | | | `vm_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Apply changes | | | `vm_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Review changes | | | `vm_properties.rs` Button::new | catalogued | not started | phase 1 |
| | Recovery controls… | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Review cancellation request… | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Close | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Confirm reviewed recovery action | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Refresh captured state | | | `vm_recovery_view.rs` Button::new | catalogued | not started | phase 1 |
| | Back to inventory | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Create snapshot | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Restore snapshot | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Edit properties | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Create VM | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Create template | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Inspect snapshot | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Activity | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Delete snapshot | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Stop remaining steps | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Recheck snapshot outcome | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |
| | Cancel | | | `vm_snapshots.rs` Button::new | catalogued | not started | phase 1 |

## Scenes

| scene name | Rune source | surface | status | package |
| --- | --- | --- | --- | --- |
| demo | `main.rs` | Rune scene | shell preview | phase 2 |
| large-inventory | `main.rs` | Rune scene | shell preview | phase 2 |
| host-version-notice | `main.rs` | Rune scene | shell preview | phase 2 |
| interface-large | `main.rs` | Rune scene | shell preview | phase 2 |
| inventory-context | `main.rs` | Rune scene | shell preview | phase 2 |
| general-information | `main.rs` | Rune scene | shell preview | phase 2 |
| options | `main.rs` | Rune scene | not ported | phase 1 |
| help | `main.rs` | Rune scene | not ported | phase 1 |
| console-details | `main.rs` | Rune scene | not ported | phase 1 |
| batch-power | `main.rs` | Rune scene | not ported | phase 1 |
| batch-properties | `main.rs` | Rune scene | not ported | phase 1 |
| batch-delete | `main.rs` | Rune scene | not ported | phase 1 |
| lifecycle-permissions | `main.rs` | Rune scene | not ported | phase 1 |
| batch-move | `main.rs` | Rune scene | not ported | phase 1 |
| batch-cross-pool | `main.rs` | Rune scene | not ported | phase 1 |
| batch-export | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-batch-export | `main.rs` | Rune scene | not ported | phase 1 |
| console-recovery | `main.rs` | Rune scene | not ported | phase 1 |
| console-frame | `main.rs` | Rune scene | shell preview, no guest cursor | phase 3 |
| console-actual-size | `main.rs` | Rune scene | not ported | phase 1 |
| console-typing-review | `main.rs` | Rune scene | not ported | phase 1 |
| multiple-consoles | `main.rs` | Rune scene | not ported | phase 1 |
| guest-connections | `main.rs` | Rune scene | not ported | phase 1 |
| external-tools | `main.rs` | Rune scene | not ported | phase 1 |
| host-console | `main.rs` | Rune scene | shell preview, no guest cursor | phase 3 |
| search-pools | `main.rs` | Rune scene | shell preview | phase 2 |
| performance | `main.rs` | Rune scene | shell preview, charts not sampled | phase 3 |
| metric-presets | `main.rs` | Rune scene | not ported | phase 1 |
| unavailable-actions | `main.rs` | Rune scene | shell preview | phase 2 |
| vm-devices | `main.rs` | Rune scene | not ported | phase 1 |
| properties | `main.rs` | Rune scene | not ported | phase 1 |
| properties-running | `main.rs` | Rune scene | not ported | phase 1 |
| vm-create | `main.rs` | Rune scene | not ported | phase 1 |
| device-properties | `main.rs` | Rune scene | not ported | phase 1 |
| home-host | `main.rs` | Rune scene | not ported | phase 1 |
| firmware-create | `main.rs` | Rune scene | not ported | phase 1 |
| creation-cloud | `main.rs` | Rune scene | not ported | phase 1 |
| creation-cloud-pending | `main.rs` | Rune scene | not ported | phase 1 |
| secure-boot | `main.rs` | Rune scene | not ported | phase 1 |
| vm-migration | `main.rs` | Rune scene | wizard preview, no host | phase 3 |
| vm-recovery | `main.rs` | Rune scene | not ported | phase 1 |
| vm-recovery-force | `main.rs` | Rune scene | not ported | phase 1 |
| cross-pool | `main.rs` | Rune scene | not ported | phase 1 |
| archive-export | `main.rs` | Rune scene | not ported | phase 1 |
| ha-policy | `main.rs` | Rune scene | not ported | phase 1 |
| failure-help | `main.rs` | Rune scene | not ported | phase 1 |
| checkpoint-export | `main.rs` | Rune scene | not ported | phase 1 |
| xva-compression | `main.rs` | Rune scene | not ported | phase 1 |
| cloud-init | `main.rs` | Rune scene | not ported | phase 1 |
| cloud-init-review | `main.rs` | Rune scene | not ported | phase 1 |
| ovf-import | `main.rs` | Rune scene | not ported | phase 1 |
| ovf-export | `main.rs` | Rune scene | not ported | phase 1 |
| disk-export | `main.rs` | Rune scene | not ported | phase 1 |
| snapshots | `main.rs` | Rune scene | list preview, no host | phase 3 |
| snapshot-create | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-ancestry | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-information | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-restore | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-edit | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-capture | `main.rs` | Rune scene | not ported | phase 1 |
| connections | `main.rs` | Rune scene | shell preview | phase 2 |
| connection-trust | `main.rs` | Rune scene | shell preview | phase 2 |
| certificate-review | `main.rs` | Rune scene | shell preview | phase 2 |
| network-interfaces | `main.rs` | Rune scene | not ported | phase 1 |
| network-services | `main.rs` | Rune scene | not ported | phase 1 |
| network-fabric | `main.rs` | Rune scene | not ported | phase 1 |
| network-fabric-pool | `main.rs` | Rune scene | not ported | phase 1 |
| network-fabric-pool-select | `main.rs` | Rune scene | not ported | phase 1 |
| network-sriov | `main.rs` | Rune scene | not ported | phase 1 |
| network-ipv6 | `main.rs` | Rune scene | not ported | phase 1 |
| network-topology | `main.rs` | Rune scene | not ported | phase 1 |
| management-network | `main.rs` | Rune scene | not ported | phase 1 |
| management-dhcp | `main.rs` | Rune scene | not ported | phase 1 |
| management-vlan | `main.rs` | Rune scene | not ported | phase 1 |
| management-bond | `main.rs` | Rune scene | not ported | phase 1 |
| management-ipv6-bond | `main.rs` | Rune scene | not ported | phase 1 |
| management-bond-guests | `main.rs` | Rune scene | not ported | phase 1 |
| management-vlan-bond | `main.rs` | Rune scene | not ported | phase 1 |
| host-administration | `main.rs` | Rune scene | review preview, no host | phase 3 |
| host-memory | `main.rs` | Rune scene | not ported | phase 1 |
| host-monitoring | `main.rs` | Rune scene | not ported | phase 1 |
| local-conversion | `main.rs` | Rune scene | not ported | phase 1 |
| local-conversion-review | `main.rs` | Rune scene | not ported | phase 1 |
| host-monitoring-catalog | `main.rs` | Rune scene | not ported | phase 1 |
| host-support | `main.rs` | Rune scene | not ported | phase 1 |
| host-restore | `main.rs` | Rune scene | not ported | phase 1 |
| host-crashdump-delete | `main.rs` | Rune scene | not ported | phase 1 |
| host-power-settings | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-schedules | `main.rs` | Rune scene | not ported | phase 1 |
| snapshot-schedule-review | `main.rs` | Rune scene | not ported | phase 1 |
| alarms | `main.rs` | Rune scene | not ported | phase 1 |
| alarms-review | `main.rs` | Rune scene | not ported | phase 1 |
| alarms-email | `main.rs` | Rune scene | not ported | phase 1 |
| network-create | `main.rs` | Rune scene | not ported | phase 1 |
| activity | `main.rs` | Rune scene | empty preview, recheck does not run | phase 3 |
| organization | `main.rs` | Rune scene | not ported | phase 1 |
| organization-review | `main.rs` | Rune scene | not ported | phase 1 |
| appliances | `main.rs` | Rune scene | not ported | phase 1 |
| appliance-review | `main.rs` | Rune scene | not ported | phase 1 |
| disk-cleanup | `main.rs` | Rune scene | not ported | phase 1 |
| disk-grow | `main.rs` | Rune scene | not ported | phase 1 |
| disk-copy | `main.rs` | Rune scene | not ported | phase 1 |
| storage-create | `main.rs` | Rune scene | not ported | phase 1 |
| storage-review | `main.rs` | Rune scene | not ported | phase 1 |
| storage-local | `main.rs` | Rune scene | not ported | phase 1 |
| storage-local-review | `main.rs` | Rune scene | not ported | phase 1 |
| storage-hba | `main.rs` | Rune scene | not ported | phase 1 |
| storage-hba-review | `main.rs` | Rune scene | not ported | phase 1 |
| storage-repair | `main.rs` | Rune scene | not ported | phase 1 |
| storage-graph | `main.rs` | Rune scene | not ported | phase 1 |
| storage-multipath | `main.rs` | Rune scene | not ported | phase 1 |
| storage-maintenance | `main.rs` | Rune scene | not ported | phase 1 |
| storage-cache | `main.rs` | Rune scene | not ported | phase 1 |
| storage-discovery | `main.rs` | Rune scene | not ported | phase 1 |
| storage-auth-review | `main.rs` | Rune scene | not ported | phase 1 |
| pool-security | `main.rs` | Rune scene | not ported | phase 1 |
| pool-join | `main.rs` | Rune scene | not ported, debug-only | phase 1 |
| root-password | `main.rs` | Rune scene | not ported | phase 1 |
| pool-dissolve | `main.rs` | Rune scene | not ported | phase 1 |
| guest-tools | `main.rs` | Rune scene | not ported | phase 1 |
| guest-tools-review | `main.rs` | Rune scene | not ported | phase 1 |
| pool-security-review | `main.rs` | Rune scene | not ported | phase 1 |
| pool-policies | `main.rs` | Rune scene | not ported | phase 1 |
| pool-policy-review | `main.rs` | Rune scene | not ported | phase 1 |
| pool-administration | `main.rs` | Rune scene | not ported | phase 1 |
| connection-settings | `main.rs` | Rune scene | shell preview | phase 2 |
| connect | `main.rs` | Rune scene | shell preview | phase 2 |
| connect-insecure | `main.rs` | Rune scene | shell preview | phase 2 |

## Tests

| Rune test | kind | disposition | new location |
| --- | --- | --- | --- |
| `crates/xenadmin-desktop/src/activity_tests.rs` `activity_review_captures_exact_target_and_never_dispatches_from_synthetic_or_stale_views` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `message_review_keeps_original_content_digest_when_selection_or_inventory_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `clear_buttons_empty_the_activity_filters_and_restore_the_rows` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `filters_cover_message_body_priority_task_state_and_cached_pools` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `history_hiding_is_terminal_only_pool_scoped_restorable_and_survives_disconnect` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `public_export_projection_excludes_task_refs_private_plans_and_resource_baselines` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `save_dialog_exports_captured_selection_even_after_switch_and_cancel_writes_nothing` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `outcome_help_opens_from_tasks_and_both_history_families_without_mutating_or_retargeting` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/activity_tests.rs` `successful_history_and_message_text_never_offer_error_code_guidance` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/activity_tests.rs` |
| `crates/xenadmin-desktop/src/alarms_view.rs` `alarm_review_converts_percent_and_retains_original_pool_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/alarms_view.rs` |
| `crates/xenadmin-desktop/src/alarms_view.rs` `invalid_threshold_and_custom_source_refuse_review_but_explicit_disable_is_captured` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/alarms_view.rs` |
| `crates/xenadmin-desktop/src/alarms_view.rs` `alarm_email_and_refresh_reviews_explain_configuration_and_delivery_separately` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/alarms_view.rs` |
| `crates/xenadmin-desktop/src/appliance_view.rs` `power_review_captures_members_order_delays_and_requires_force_confirmation` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/appliance_view.rs` |
| `crates/xenadmin-desktop/src/appliance_view.rs` `membership_review_preserves_exact_group_and_refuses_stealing_other_group_vm` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/appliance_view.rs` |
| `crates/xenadmin-desktop/src/appliance_view.rs` `compact_review_keeps_controls_visible_and_refuses_preview_submission` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/appliance_view.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `the_inventory_selection_offers_the_move_to_another_pool_beside_the_same_pool_move` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `the_review_lists_every_vm_with_its_mapping_both_pools_and_a_reachable_footer` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `each_stage_asks_for_its_choices_and_the_acknowledgement_is_given_once_per_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `a_vm_or_destination_that_does_not_fit_refuses_the_whole_selection_and_omits_none` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `a_review_that_no_longer_matches_the_pools_or_lacks_a_current_session_is_not_submitted` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` `history_shows_a_batch_with_each_vm_and_offers_recheck_stop_and_acknowledgement_on_the_batch_only` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_cross_pool_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_delete_tests.rs` `compact_and_wide_reviews_keep_exact_targets_and_the_confirmation_footer` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_delete_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_delete_tests.rs` `changed_or_missing_target_refuses_the_entire_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_delete_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_delete_tests.rs` `selected_disk_data_is_explicit_and_new_ownership_cannot_be_silently_omitted` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_delete_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_delete_tests.rs` `template_and_read_only_reviews_do_not_drop_ineligible_rows` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_delete_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `several_snapshots_selected_in_the_view_open_one_review_and_single_actions_take_one` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `review_names_each_snapshot_its_memory_choice_output_and_format_with_a_reachable_footer` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `including_memory_is_chosen_for_each_checkpoint_by_itself_and_ends_a_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `a_checkpoint_in_the_selection_leaves_only_the_xva_formats` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `a_changed_snapshot_memory_image_or_destination_blocks_the_whole_review` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` `history_names_each_snapshot_of_a_batch_and_stops_after_the_current_snapshot` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_snapshots_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_tests.rs` `compact_and_wide_export_review_keeps_exact_targets_and_safe_footer` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_tests.rs` `native_output_selection_is_bound_to_editor_identity_and_format` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_tests.rs` `overlapping_destinations_and_changed_sources_block_entire_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_export_tests.rs` `batch_history_keeps_parent_reachable_and_children_have_no_independent_controls` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_export_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_move_tests.rs` `compact_and_wide_review_capture_all_vms_and_keep_safe_footer` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_move_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_move_tests.rs` `mapping_edits_are_per_target_and_review_exact_prior_and_new_values` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_move_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_move_tests.rs` `changed_or_unsupported_targets_refuse_whole_selection_without_omission` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_move_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_power_tests.rs` `multi_selection_survives_inventory_refresh_and_reviews_captured_vms_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_power_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_power_tests.rs` `context_menu_targets_clicked_row_and_keeps_non_vm_menus_relevant` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_power_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_power_tests.rs` `batch_uses_only_visible_selection_and_rejects_over_limit_without_silent_subset` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_power_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_multiselect_review_is_immutable_and_footer_reachable_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_names_every_unchanged_target_and_refuses_all_noop` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_missing_unsupported_and_changed_targets_block_whole_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_all_existing_property_kinds_bind_per_target_previous_values` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_live_limits_capabilities_and_stale_review_refuse_submission` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/bulk_properties_tests.rs` `property_batch_picker_and_template_entry_work_with_keyboard_menu_capture` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/bulk_properties_tests.rs` |
| `crates/xenadmin-desktop/src/checkpoint_transfer_tests.rs` `checkpoint_export_explicitly_reviews_memory_and_captured_identity_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/checkpoint_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/checkpoint_transfer_tests.rs` `checkpoint_import_review_uses_separate_action_and_retains_suspended_state` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/checkpoint_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` `cloud_init_review_keeps_target_and_resources_at_compact_and_wide_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` |
| `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` `cloud_init_bad_inputs_preserve_editor_and_pool_changes_disable_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` |
| `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` `cloud_init_cancelling_preparation_drops_late_media_without_reopening` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` |
| `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` `cloud_init_slot_validation_refuses_occupied_and_noncanonical_slots` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` |
| `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` `creation_handoff_prepares_private_media_for_the_exact_created_target_and_chosen_sr` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cloud_init_view_tests.rs` |
| `crates/xenadmin-desktop/src/compression_transfer_tests.rs` `compression_choice_invalidates_destination_and_review_keeps_exact_source_and_format` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/compression_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/compression_transfer_tests.rs` `changing_compression_discards_pending_picker_result_and_package_changes_clear_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/compression_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/console_export.rs` `chooser_captures_one_frame_and_cancel_or_console_switch_does_not_write` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_export.rs` |
| `crates/xenadmin-desktop/src/console_export.rs` `screenshot_round_trip_is_private_and_invalid_frames_do_not_replace_files` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_export.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `external_move_reboot_and_generation_change_close_then_reopen_once` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `retry_budget_is_bounded_and_brief_connections_do_not_reset_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `stale_missing_halted_and_read_only_targets_cannot_reopen_or_reset_failures` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `optional_boot_metadata_never_causes_a_spurious_reopen_and_opt_out_is_honored` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `synchronous_admission_failure_consumes_attempts_and_releases_old_input` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `inventory_observation_uses_pinned_pool_and_vm_and_rejects_missing_or_wrong_kind` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_recovery_tests.rs` `automatic_reconnect_toggle_and_pinned_identity_render_at_both_window_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_recovery_tests.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `console_defaults_apply_only_when_opening_a_new_target` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `clipboard_text_is_typed_only_after_a_review_that_never_shows_it` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `a_repeat_after_release_does_not_press_the_key_again` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `the_console_canvas_is_laid_out_in_fit_and_actual_size_modes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `pointer_matches_letterboxing_actual_size_and_resize` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `wheel_notches_and_precise_motion_reach_guest_without_losing_small_notches` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_view.rs` `extended_scancodes_follow_qemu_make_code_encoding` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_view.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `independent_console_windows_keep_targets_frames_and_input_state_separate` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `reusable_slot_rejects_old_adoption_close_and_focus_requests` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `window_capacity_is_bounded_and_closing_returns_the_slot` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `manager_is_reachable_with_eight_targets_at_compact_and_wide_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `a_console_window_is_drawn_at_the_interface_size_and_follows_changes_to_it` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/console_windows.rs` `moving_console_invalidates_old_file_dialog_without_disabling_new_window_export` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/console_windows.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `reviewed_mapping_and_host_transport_warning_render_at_both_widths_without_demo_dispatch` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `choosing_a_pool_requires_explicit_device_mappings_and_new_transport_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `current_source_membership_and_destination_attachments_gate_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `halted_copy_review_preserves_source_and_uses_distinct_identity_semantics` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `offline_history_keeps_both_pool_namespaces_and_partial_resources_after_source_disappears` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` `destination_console_requires_proven_result_and_exact_current_pool_vm_host_and_power_state` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/cross_pool_vm_tests.rs` |
| `crates/xenadmin-desktop/src/disk_workspace_tests.rs` `compact_and_wide_copy_review_capture_target_and_block_demo_dispatch` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/disk_workspace_tests.rs` |
| `crates/xenadmin-desktop/src/disk_workspace_tests.rs` `growth_rejects_shrink_and_copy_requires_name_and_chosen_storage` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/disk_workspace_tests.rs` |
| `crates/xenadmin-desktop/src/disk_workspace_tests.rs` `cleanup_requires_local_ownership_and_current_consumer_graph` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/disk_workspace_tests.rs` |
| `crates/xenadmin-desktop/src/disk_workspace_tests.rs` `cleanup_review_keeps_exact_uuid_and_explicit_destructive_confirmation_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/disk_workspace_tests.rs` |
| `crates/xenadmin-desktop/src/external_tool_command.rs` `local_launch_preserves_argv_without_shell_or_secret_environment` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tool_command.rs` |
| `crates/xenadmin-desktop/src/external_tool_command.rs` `command_requires_reviewed_executable_and_bounded_typed_inputs` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tool_command.rs` |
| `crates/xenadmin-desktop/src/external_tool_command.rs` `rdp_uri_escapes_username_and_brackets_ipv6_without_password` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tool_command.rs` |
| `crates/xenadmin-desktop/src/external_tool_command.rs` `local_declarations_are_versioned_bounded_and_do_not_accept_hidden_fields` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tool_command.rs` |
| `crates/xenadmin-desktop/src/external_tools_tests.rs` `reviewed_guest_command_keeps_target_and_rejects_changed_input_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tools_tests.rs` |
| `crates/xenadmin-desktop/src/external_tools_tests.rs` `local_tool_configuration_saves_separate_arguments_and_can_remove_every_tool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tools_tests.rs` |
| `crates/xenadmin-desktop/src/external_tools_tests.rs` `captured_guest_checks_pool_generation_power_and_current_reported_ip` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tools_tests.rs` |
| `crates/xenadmin-desktop/src/external_tools_tests.rs` `delayed_command_review_cannot_apply_after_edit_or_close` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/external_tools_tests.rs` |
| `crates/xenadmin-desktop/src/failure_help.rs` `exact_envelopes_only_and_unknown_outcome_takes_precedence_over_host_advice` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/failure_help.rs` |
| `crates/xenadmin-desktop/src/failure_help.rs` `captured_help_remains_read_only_and_close_is_visible_at_both_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/failure_help.rs` |
| `crates/xenadmin-desktop/src/guest_tools_view.rs` `tools_review_captures_explicit_media_and_identity_and_fixed_footer_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/guest_tools_view.rs` |
| `crates/xenadmin-desktop/src/guest_tools_view.rs` `tools_review_explains_existing_media_and_missing_devices_without_issuing_commands` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/guest_tools_view.rs` |
| `crates/xenadmin-desktop/src/guest_tools_view.rs` `tools_panel_refuses_device_view_from_another_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/guest_tools_view.rs` |
| `crates/xenadmin-desktop/src/ha_policy_view.rs` `ha_policy_review_binds_vm_and_shared_vapp_settings_at_both_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ha_policy_view.rs` |
| `crates/xenadmin-desktop/src/ha_policy_view.rs` `ha_policy_edits_refuse_invalid_inputs_and_keep_timeout_precedence_explicit` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ha_policy_view.rs` |
| `crates/xenadmin-desktop/src/ha_policy_view.rs` `ha_policy_missing_membership_refuses_review_and_selecting_vm_resets_draft` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ha_policy_view.rs` |
| `crates/xenadmin-desktop/src/host_admin.rs` `maintenance_review_keeps_host_and_guest_identity_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_admin.rs` |
| `crates/xenadmin-desktop/src/host_admin.rs` `memory_editor_rejects_zero_overflow_and_physical_overcommit` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_admin.rs` |
| `crates/xenadmin-desktop/src/host_admin.rs` `logging_review_keeps_exact_old_and_new_destination` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_admin.rs` |
| `crates/xenadmin-desktop/src/host_admin.rs` `autostart_is_explicitly_pool_scoped_and_unknown_is_not_false` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_admin.rs` |
| `crates/xenadmin-desktop/src/host_admin.rs` `forget_requires_offline_disabled_noncoordinator_and_explicit_power_off_ack` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_admin.rs` |
| `crates/xenadmin-desktop/src/host_console.rs` `host_console_keeps_host_details_and_never_offers_guest_power_or_device_actions` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_console.rs` |
| `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` `destructive_review_keeps_captured_source_and_reachable_controls_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` |
| `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` `refresh_discards_catalog_and_review_and_enabled_source_cannot_start_again` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` |
| `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` `history_uses_captured_pool_after_switching_the_visible_inventory` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_monitoring_view_tests.rs` |
| `crates/xenadmin-desktop/src/host_power_view.rs` `review_has_private_password_and_captured_target_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_power_view.rs` |
| `crates/xenadmin-desktop/src/host_power_view.rs` `configuration_review_refuses_invalid_inputs_and_unknown_configuration` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_power_view.rs` |
| `crates/xenadmin-desktop/src/host_power_view.rs` `switching_mode_discards_pending_password_and_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_power_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `host_archive_review_captures_target_and_keeps_compact_footer_visible` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `crashdump_delete_review_captures_exact_dump_and_keeps_footer_visible` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `crashdump_catalogue_pages_make_every_captured_candidate_reachable` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `crashdump_delete_refuses_unadvertised_review_or_changed_host` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `stale_native_file_callback_cannot_set_a_reopened_host_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `unknown_capabilities_and_changed_host_version_cannot_be_reviewed` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `restore_native_open_inspects_captured_host_and_requires_exact_confirmation` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/host_support_view.rs` `restore_rejects_wrong_host_and_cancels_obsolete_inspections` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/host_support_view.rs` |
| `crates/xenadmin-desktop/src/instance_launch.rs` `previews_and_help_do_not_claim_or_activate_live_desktop` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/instance_launch.rs` |
| `crates/xenadmin-desktop/src/instance_launch.rs` `a_version_question_is_recognised_wherever_it_stands` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/instance_launch.rs` |
| `crates/xenadmin-desktop/src/instance_launch.rs` `quiet_state_has_no_backend_ownership_profile_restore_or_inspection` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/instance_launch.rs` |
| `crates/xenadmin-desktop/src/inventory_information.rs` `information_keeps_its_object_and_pool_and_refreshes_only_the_same_pool` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_information.rs` |
| `crates/xenadmin-desktop/src/inventory_information.rs` `captured_host_hardware_and_pool_entry_point_are_reachable` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_information.rs` |
| `crates/xenadmin-desktop/src/inventory_menu_tests.rs` `right_click_any_column_or_padding_targets_the_visible_row_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_menu_tests.rs` |
| `crates/xenadmin-desktop/src/inventory_menu_tests.rs` `menu_key_and_shift_f10_preserve_keyboard_selection_and_open_current_row` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_menu_tests.rs` |
| `crates/xenadmin-desktop/src/inventory_menu_tests.rs` `header_and_blank_space_do_not_invent_a_row_target` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_menu_tests.rs` |
| `crates/xenadmin-desktop/src/inventory_menu_tests.rs` `stale_menu_cannot_apply_to_a_removed_row_or_changed_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_menu_tests.rs` |
| `crates/xenadmin-desktop/src/inventory_menu_tests.rs` `keyboard_menu_follows_virtual_scrolling_and_changed_selection_cannot_start_a_batch` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_menu_tests.rs` |
| `crates/xenadmin-desktop/src/inventory_report.rs` `reports_keep_pool_scope_and_explicit_stale_incomplete_coverage` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_report.rs` |
| `crates/xenadmin-desktop/src/inventory_report.rs` `report_limits_refuse_silent_truncation_or_invalid_filters` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_report.rs` |
| `crates/xenadmin-desktop/src/inventory_report.rs` `native_report_export_captures_results_and_ignores_cancelled_or_stale_dialogs` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/inventory_report.rs` |
| `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` `offline_form_and_review_keep_heading_and_final_controls_reachable_without_host_access` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` |
| `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` `native_file_dialog_cancellation_and_late_result_do_not_change_closed_form` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` |
| `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` `background_job_controls_do_not_retarget_and_disconnection_is_reported` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` |
| `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` `successful_conversion_only_offers_import_and_never_creates_a_host_operation` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/local_conversion_view_tests.rs` |
| `crates/xenadmin-desktop/src/main.rs` `the_shared_registry_lists_every_app_scene` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `sidebar_links_to_the_repository_without_a_demo_entry` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `action_checks_follow_vm_availability_without_expiring_on_unrelated_events` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `action_checks_require_the_same_current_connection_and_vm_state` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `devices_keep_their_target_and_leave_compact_details` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `editing_the_host_resets_a_remembered_certificate_bypass` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `connection_form_exposes_one_opt_out_and_keeps_verified_options_available` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `search_and_host_filter_select_matching_objects` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `real_widgets_open_the_connection_dialog_and_filter_demo_inventory` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `measure_large_inventory` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `hosts_outside_the_qualified_release_are_named_in_a_notice` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `dialogs_outside_the_supported_core_carry_the_experimental_notice` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `interface_size_scales_the_layout_and_follows_ctrl_plus_minus_and_zero` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `column_and_sidebar_widths_reach_the_preferences_and_survive_a_table_rebuild` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `the_details_pane_width_reaches_the_preferences_and_the_next_start_uses_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `widths_are_saved_shortly_after_they_stop_changing_without_waiting_for_close` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `search_clear_button_empties_the_search_and_restores_every_row` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/main.rs` `narrow_window_keeps_connection_and_vm_details_reachable` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/main.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `management_guest_query_result_is_fenced_to_exact_form_and_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `management_guest_impact_review_lists_moved_retained_and_no_eligible_guests` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `management_guest_impact_becomes_stale_when_consumers_or_guest_placement_change` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `management_bond_review_has_exact_members_and_recovery_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `removal_review_captures_the_existing_primary_member_and_refuses_changed_bond` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `management_vlan_bond_review_lists_recreated_ids_and_captures_parent_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_bond_ui.rs` `changed_member_or_added_consumer_invalidates_captured_bond_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` `ipv6_stage_review_renders_at_two_widths_and_keeps_demo_apply_disabled` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` `ipv6_review_refuses_changed_temporary_or_member_identity` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` `ipv6_continuation_retains_original_review_and_targets_exact_predecessor` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_ipv6_bond_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `vlan_over_bond_review_captures_full_topology_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `topology_review_tracks_membership_protection_and_metadata_but_not_link_counters` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `bond_members_and_nested_vlan_targets_do_not_form_a_management_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `automatic_address_review_captures_mode_expected_ip_and_does_not_dispatch_demo` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `automatic_review_rejects_wrong_family_hostnames_noop_and_changed_lease` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `ipv6_dhcp_and_autoconf_review_require_same_family_and_capture_stable_recovery` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `management_review_captures_recovery_settings_at_both_widths_without_demo_dispatch` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `management_review_refuses_changed_source_pool_ha_and_nonstandard_mtu` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `management_move_review_captures_the_destination_and_retains_old_address` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/management_network_ui.rs` `management_ipv6_review_keeps_installed_family_and_rejects_invalid_recovery_address` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/management_network_ui.rs` |
| `crates/xenadmin-desktop/src/metric_presets.rs` `save_apply_and_remove_presets_preserve_the_target_at_both_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metric_presets.rs` |
| `crates/xenadmin-desktop/src/metric_presets.rs` `missing_sources_and_other_target_types_never_silently_change_the_graph` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metric_presets.rs` |
| `crates/xenadmin-desktop/src/metric_presets.rs` `invalid_preset_does_not_replace_previous_definition` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metric_presets.rs` |
| `crates/xenadmin-desktop/src/metrics_view.rs` `graph_preserves_missing_samples_time_gaps_zero_and_finite_extremes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metrics_view.rs` |
| `crates/xenadmin-desktop/src/metrics_view.rs` `timestamps_have_explicit_utc_calendar_dates_for_long_ranges` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metrics_view.rs` |
| `crates/xenadmin-desktop/src/metrics_view.rs` `performance_controls_show_actual_coverage_and_change_ranges_without_host_writes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/metrics_view.rs` |
| `crates/xenadmin-desktop/src/network_admin.rs` `compact_and_wide_network_reviews_remain_visible_and_preview_cannot_submit` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_admin.rs` |
| `crates/xenadmin-desktop/src/network_admin.rs` `mtu_review_captures_original_value_instead_of_chasing_inventory` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_admin.rs` |
| `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` `exact_order_and_all_hosts_are_reviewed_at_both_widths_and_demo_cannot_apply` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` |
| `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` `pool_controller_and_input_changes_invalidate_captured_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` |
| `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` `input_editor_requires_all_missing_hosts_and_preserves_exact_selection` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_pool_ui.rs` |
| `crates/xenadmin-desktop/src/network_fabric_ui.rs` `host_scoped_tunnel_review_is_visible_at_both_sizes_and_demo_cannot_submit` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_ui.rs` |
| `crates/xenadmin-desktop/src/network_fabric_ui.rs` `review_fences_changed_controller_transport_and_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_ui.rs` |
| `crates/xenadmin-desktop/src/network_fabric_ui.rs` `sriov_review_names_driver_impact_and_reboot_boundary_at_both_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_fabric_ui.rs` |
| `crates/xenadmin-desktop/src/network_interfaces.rs` `ipv6_review_is_visible_at_both_widths_and_keeps_captured_identity` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_interfaces.rs` |
| `crates/xenadmin-desktop/src/network_interfaces.rs` `interface_review_refuses_changed_identity_configuration_and_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_interfaces.rs` |
| `crates/xenadmin-desktop/src/network_interfaces.rs` `purpose_label_is_separate_from_unplug_protection_and_invalid_ipv6_is_refused` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_interfaces.rs` |
| `crates/xenadmin-desktop/src/network_service_ui.rs` `review_captures_plaintext_policy_and_interfaces_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_service_ui.rs` |
| `crates/xenadmin-desktop/src/network_service_ui.rs` `changed_interface_pool_or_service_policy_invalidates_captured_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_service_ui.rs` |
| `crates/xenadmin-desktop/src/network_topology_ui.rs` `pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_topology_ui.rs` |
| `crates/xenadmin-desktop/src/network_topology_ui.rs` `topology_review_refuses_changed_pool_graph_and_keeps_captured_identities` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_topology_ui.rs` |
| `crates/xenadmin-desktop/src/network_topology_ui.rs` `retag_review_refuses_protection_consumers_missing_inventory_and_unchanged_tag` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_topology_ui.rs` |
| `crates/xenadmin-desktop/src/network_topology_ui.rs` `vlan_zero_is_explicit_and_invalid_tags_cannot_reach_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_topology_ui.rs` |
| `crates/xenadmin-desktop/src/network_topology_ui.rs` `topology_export_contains_only_public_created_and_removed_ids` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/network_topology_ui.rs` |
| `crates/xenadmin-desktop/src/organization_view.rs` `folder_rename_updates_descendants_and_registry_without_touching_other_paths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/organization_view.rs` |
| `crates/xenadmin-desktop/src/organization_view.rs` `custom_field_deletion_reviews_values_before_removing_definition` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/organization_view.rs` |
| `crates/xenadmin-desktop/src/organization_view.rs` `exact_tag_and_folder_filters_are_distinct_from_name_search` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/organization_view.rs` |
| `crates/xenadmin-desktop/src/organization_view.rs` `compact_review_is_reachable_and_cannot_submit_demo_metadata` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/organization_view.rs` |
| `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` `ovf_import_requires_explicit_network_mapping_and_reviews_captured_storage` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` `ova_and_ovf_export_use_new_destinations_and_keep_captured_source` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` `changing_package_format_discards_inspection_and_stale_picker_results` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/ovf_transfer_tests.rs` |
| `crates/xenadmin-desktop/src/pool_dissolve_view.rs` `dissolve_review_keeps_exact_members_and_confirmation_reachable_at_both_widths` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_dissolve_view.rs` |
| `crates/xenadmin-desktop/src/pool_dissolve_view.rs` `dissolve_capture_refuses_unknown_coordinator_ha_and_enabled_members_cannot_submit` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_dissolve_view.rs` |
| `crates/xenadmin-desktop/src/pool_security_view.rs` `certificate_files_are_regular_bounded_and_errors_do_not_include_contents` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_security_view.rs` |
| `crates/xenadmin-desktop/src/pool_security_view.rs` `compact_review_keeps_footer_visible_and_captured_identity_immutable` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_security_view.rs` |
| `crates/xenadmin-desktop/src/pool_security_view.rs` `native_certificate_picker_discards_closed_or_retargeted_requests_and_cancel_preserves_selection` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_security_view.rs` |
| `crates/xenadmin-desktop/src/pool_security_view.rs` `late_certificate_validation_is_discarded_when_host_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_security_view.rs` |
| `crates/xenadmin-desktop/src/pool_view.rs` `review_captures_target_and_compact_footer_never_submits_demo` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_view.rs` |
| `crates/xenadmin-desktop/src/pool_view.rs` `ejection_requires_host_uuid_and_priority_unknown_is_not_disabled` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/pool_view.rs` |
| `crates/xenadmin-desktop/src/profile_pool_join.rs` `compact_join_review_keeps_footer_reachable_and_never_dispatches_demo` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_pool_join.rs` |
| `crates/xenadmin-desktop/src/profile_pool_join.rs` `destination_change_and_stale_connection_clear_transient_credentials` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_pool_join.rs` |
| `crates/xenadmin-desktop/src/profile_reconnect.rs` `reconnect_is_ephemeral_cancels_same_pool_aliases_and_retains_other_pool` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_reconnect.rs` |
| `crates/xenadmin-desktop/src/profile_reconnect.rs` `changed_profile_refuses_reconnect_and_clears_entered_password` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_reconnect.rs` |
| `crates/xenadmin-desktop/src/profile_reconnect.rs` `compact_reconnect_review_cancel_never_changes_current_session` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_reconnect.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `bulk_loading_is_bounded_and_disconnect_all_invalidates_pending_credentials` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `edited_removed_and_cancelled_profiles_discard_late_results` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `retry_does_not_accept_an_old_attempt_and_editor_trust_edits_discard_loaded_passwords` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `concurrent_successes_connect_their_own_profiles_without_replacing_the_open_editor` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `rune_bulk_controls_show_profile_errors_and_cancel_pending_attempts` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `forgetting_saved_credentials_also_disables_startup_restore` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `headless_flags_do_not_restore_a_saved_session` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_restore_tests.rs` `rune_startup_restore_checkbox_requires_explicit_keyring_choice` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_restore_tests.rs` |
| `crates/xenadmin-desktop/src/profile_root_password.rs` `late_positive_evidence_never_saves_unknown_acknowledged_or_interrupted_completion` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_root_password.rs` |
| `crates/xenadmin-desktop/src/profile_root_password.rs` `review_is_pool_scoped_masked_and_footer_stays_reachable` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_root_password.rs` |
| `crates/xenadmin-desktop/src/profile_root_password.rs` `mismatched_passwords_and_stale_profile_clear_without_submission` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_root_password.rs` |
| `crates/xenadmin-desktop/src/profile_root_password.rs` `password_store_failure_preserves_host_result_and_never_retries_automatically` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_root_password.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `compact_and_wide_editor_keep_header_and_connect_controls_visible` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `reviewed_changed_pin_requires_exact_endpoint_and_explicit_fingerprint_check` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `delayed_inspection_and_review_cannot_follow_changed_route_pin_or_editor` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `proxy_credentials_are_session_only_and_bound_to_the_exact_proxy_configuration` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `synthetic_inspection_cannot_contact_an_endpoint_or_load_a_password` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `endpoint_edit_drops_the_previous_certificate_pin` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `endpoint_edit_keeps_a_different_fingerprint_on_the_new_endpoint_only` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `unchanged_endpoint_keeps_its_certificate_pin` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profile_trust_tests.rs` `reviewing_failed_member_certificate_does_not_load_saved_host_credentials` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profile_trust_tests.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `ca_picker_applies_to_its_editor_only_and_cancel_preserves_the_path` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `saving_and_editing_a_profile_needs_no_connection_and_does_not_save_passwords` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `invalid_profile_settings_do_not_replace_saved_settings` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `second_window_cannot_write_profiles_or_migrate_legacy_settings` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `connections_dialog_exposes_saved_profiles_and_opens_an_editor` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/profiles_view.rs` `saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/profiles_view.rs` |
| `crates/xenadmin-desktop/src/resource_admin.rs` `joined_pool_history_is_visible_at_destination_and_acknowledges_both_reservations` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_admin.rs` |
| `crates/xenadmin-desktop/src/resource_admin.rs` `network_review_captures_pool_name_mtu_and_never_submits_from_preview` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_admin.rs` |
| `crates/xenadmin-desktop/src/resource_admin.rs` `resource_review_keeps_original_selection_and_rejects_invalid_or_unchanged_values` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_admin.rs` |
| `crates/xenadmin-desktop/src/resource_admin.rs` `used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_admin.rs` |
| `crates/xenadmin-desktop/src/resource_admin.rs` `compact_network_controls_remain_inside_the_window` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_admin.rs` |
| `crates/xenadmin-desktop/src/resource_details.rs` `resource_details_show_relationships_and_unknown_attachment_state` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/resource_details.rs` |
| `crates/xenadmin-desktop/src/search_files.rs` `local_search_reader_is_bounded_and_refuses_symlinks_and_special_files` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_files.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `search_entry_point_is_reachable_at_both_window_sizes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `aliases_deduplicate_by_pool_and_prefer_current_selected_session_without_comparing_revisions` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `navigation_refuses_removed_and_orphan_snapshots_instead_of_opening_another_vm` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `cross_pool_results_keep_duplicate_vm_ids_distinct_and_synthetic_navigation_disabled` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `saving_and_loading_does_not_erase_imported_complex_filters` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `imported_queries_require_review_and_conflicts_leave_existing_definitions_intact` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/search_tests.rs` `native_search_picker_cancellation_and_closed_workspace_callbacks_are_ignored` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/search_tests.rs` |
| `crates/xenadmin-desktop/src/session_permissions_view.rs` `known_denial_disables_power_without_hiding_its_reason_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/session_permissions_view.rs` |
| `crates/xenadmin-desktop/src/session_permissions_view.rs` `unknown_permissions_allow_review_but_grants_never_override_object_ineligibility` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/session_permissions_view.rs` |
| `crates/xenadmin-desktop/src/session_permissions_view.rs` `review_permission_notice_is_reachable_without_bypassing_synthetic_target_guard` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/session_permissions_view.rs` |
| `crates/xenadmin-desktop/src/settings.rs` `options_cancel_and_apply_preserve_other_preferences_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/settings.rs` |
| `crates/xenadmin-desktop/src/settings.rs` `legacy_preferences_default_safely_and_force_review_cannot_be_disabled` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/settings.rs` |
| `crates/xenadmin-desktop/src/settings.rs` `help_topics_and_close_remain_reachable_on_compact_windows` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/settings.rs` |
| `crates/xenadmin-desktop/src/settings.rs` `compact_console_details_returns_to_same_pinned_guest` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/settings.rs` |
| `crates/xenadmin-desktop/src/snapshot_capture.rs` `cancelled_or_uncertain_snapshot_never_authorizes_preview_upload` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_capture.rs` |
| `crates/xenadmin-desktop/src/snapshot_capture.rs` `snapshot_capture_encoder_bounds_large_frames_and_rejects_invalid_data` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_capture.rs` |
| `crates/xenadmin-desktop/src/snapshot_capture.rs` `snapshot_preview_opt_in_is_explicit_and_synthetic_confirm_cannot_dispatch` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_capture.rs` |
| `crates/xenadmin-desktop/src/snapshot_inspection.rs` `snapshot_inspection_captures_identity_and_close_is_reachable_at_both_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_inspection.rs` |
| `crates/xenadmin-desktop/src/snapshot_inspection.rs` `tree_and_list_keep_selection_and_show_current_branch` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_inspection.rs` |
| `crates/xenadmin-desktop/src/snapshot_inspection.rs` `decoder_bounds_dimensions_and_supports_png_and_jpeg` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_inspection.rs` |
| `crates/xenadmin-desktop/src/snapshot_inspection.rs` `closing_or_replacing_the_panel_does_not_release_a_running_decoder_slot` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_inspection.rs` |
| `crates/xenadmin-desktop/src/snapshot_inspection.rs` `preview_request_refuses_a_changed_or_disconnected_workspace` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_inspection.rs` |
| `crates/xenadmin-desktop/src/snapshot_policy_view.rs` `schedule_review_captures_coordinator_and_pool_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_policy_view.rs` |
| `crates/xenadmin-desktop/src/snapshot_policy_view.rs` `invalid_minutes_retention_and_unknown_clock_block_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_policy_view.rs` |
| `crates/xenadmin-desktop/src/snapshot_policy_view.rs` `weekly_day_selection_and_checkpoint_are_reviewed` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/snapshot_policy_view.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `reviewed_storage_graph_is_captured_reachable_and_never_submits_demo` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `missing_repair_requires_selection_and_rejects_new_pair_before_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `changed_multipath_attachment_and_pool_disable_captured_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `multipath_refuses_unknown_handler_and_nonmaintenance_host` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `edit_host_selection_then_review_keeps_exact_uuid` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `partial_outcome_restore_is_a_new_review_and_unknown_cannot_offer_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` `restore_cannot_turn_into_detach_after_state_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_graph_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_local_view.rs` `local_review_captures_operator_declaration_host_and_device_and_keeps_confirmation_reachable` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_local_view.rs` |
| `crates/xenadmin-desktop/src/storage_local_view.rs` `local_review_requires_current_dual_probe_and_explicit_operator_acknowledgement` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_local_view.rs` |
| `crates/xenadmin-desktop/src/storage_local_view.rs` `local_known_repository_selection_chooses_nonformatting_introduction` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_local_view.rs` |
| `crates/xenadmin-desktop/src/storage_local_view.rs` `local_form_exposes_declaration_and_erase_acknowledgement_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_local_view.rs` |
| `crates/xenadmin-desktop/src/storage_local_view.rs` `local_path_focus_preserves_probe_but_text_edit_invalidates_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_local_view.rs` |
| `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` `repair_review_captures_targets_and_remains_reachable_at_compact_and_wide_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` `repair_requires_selection_and_rejects_changed_attachment_graph` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` `read_cache_review_explains_policy_and_preserves_target_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` `cache_unknown_or_changed_current_policy_cannot_be_reviewed_or_submitted` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` `selecting_a_host_attachment_reviews_only_that_exact_target_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_maintenance_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` `hba_selection_requires_review_and_captures_host_device_and_allocation_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` `hba_review_refuses_unselected_device_changed_host_and_unadvertised_allocation` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` `hba_discovery_results_cannot_be_selected_after_host_or_connection_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` `hba_review_explains_external_ownership_destructive_format_and_no_automatic_cleanup` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_provisioning_view_tests.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `authenticated_storage_review_is_captured_and_footer_remains_reachable` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `storage_discovery_result_fence_includes_credential_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `discovery_selection_is_reachable_bound_to_inputs_and_keeps_review_separate` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `chosen_lun_requires_separate_destructive_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `storage_review_preserves_inputs_and_compact_footer_never_submits_preview` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/storage_view.rs` `storage_forget_and_destroy_have_distinct_review_and_exact_disk_set` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/storage_view.rs` |
| `crates/xenadmin-desktop/src/theme.rs` `controls_are_distinguishable_from_the_surfaces_they_sit_on` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/theme.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `native_save_captures_source_and_reviews_without_demo_writes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `cancelled_or_stale_file_dialog_does_not_replace_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `offline_unknown_history_preserves_evidence_and_cannot_release_guard` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `switched_pool_invalidates_review_and_import_uses_open_dialog` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `native_raw_import_reviews_name_capacity_and_exact_storage_without_overwrite` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `disk_export_captures_identity_and_rejects_capacity_or_attachment_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/transfers.rs` `format_change_invalidates_selected_file_and_blank_disk_name_cannot_apply` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/transfers.rs` |
| `crates/xenadmin-desktop/src/vm_creation_cloud.rs` `optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_creation_cloud.rs` |
| `crates/xenadmin-desktop/src/vm_creation_cloud.rs` `creation_cloud_inputs_are_validated_and_review_omits_the_private_payload` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_creation_cloud.rs` |
| `crates/xenadmin-desktop/src/vm_creation_cloud.rs` `creation_cloud_pending_requires_exact_success_and_never_uses_the_selection` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_creation_cloud.rs` |
| `crates/xenadmin-desktop/src/vm_creation_cloud.rs` `creation_cloud_pending_controls_are_visible_and_synthetic_continue_never_prepares_media` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_creation_cloud.rs` |
| `crates/xenadmin-desktop/src/vm_device_actions.rs` `growth_input_preserves_exact_capacity_and_rejects_overflow` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_actions.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `disk_creation_reviews_selected_storage_size_and_captured_vm_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `attach_and_add_cd_review_exact_targets_without_deleting_existing_data` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `detach_preserves_data_delete_reviews_consumers_and_ownership_changes_disable_it` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `nic_editor_reviews_generation_and_replacement_and_captures_original_identity` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `invalid_resource_choices_and_user_values_cannot_reach_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `device_classes_are_read_using_canonical_inventory_keys` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `reviewed_changes_cannot_follow_a_pool_switch_or_missing_live_observation` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` `shared_disk_consumers_are_described_and_deletion_stays_unavailable` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` `priority_review_keeps_device_identity_at_both_widths_and_cannot_submit_synthetic_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` `nic_rate_review_has_correct_units_and_preserves_captured_qos` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` |
| `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` `metadata_cbt_confirmation_and_invalid_values_require_explicit_valid_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_device_properties_tests.rs` |
| `crates/xenadmin-desktop/src/vm_devices.rs` `absent_permission_and_empty_are_distinct` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_devices.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `creation_wizard_reviews_typed_resources_and_keeps_its_source_at_both_widths` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `cloning_distinguishes_fast_and_full_copy_and_exact_destination` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `conversion_and_template_metadata_have_reviewed_captured_sources` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `deletion_defaults_record_only_and_captures_exact_selected_data_and_snapshots` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `invalid_resources_and_stale_or_changed_sources_cannot_submit` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `global_history_survives_deleted_source_and_is_reachable_offline_at_compact_width` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `shared_history_retains_unresolved_records_and_bounds_terminal_records` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `installation_iso_is_selected_by_uuid_and_built_in_templates_cannot_be_edited_or_deleted` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` `copied_bios_choice_captures_host_and_affinity_in_the_creation_review` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_lifecycle_tests.rs` |
| `crates/xenadmin-desktop/src/vm_migration_tests.rs` `compact_and_wide_review_capture_exact_targets_and_remain_preview_only` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_migration_tests.rs` |
| `crates/xenadmin-desktop/src/vm_migration_tests.rs` `destination_and_mapping_controls_produce_reviewed_typed_plan` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_migration_tests.rs` |
| `crates/xenadmin-desktop/src/vm_migration_tests.rs` `stale_or_missing_vm_and_changed_devices_prevent_apply` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_migration_tests.rs` |
| `crates/xenadmin-desktop/src/vm_migration_tests.rs` `halted_storage_move_requires_transfer_network_and_fresh_home_host` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_migration_tests.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `home_host_picker_reviews_original_identity_and_keeps_synthetic_apply_disabled` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `properties_review_is_reachable_and_demo_apply_is_disabled` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `memory_input_is_exact_and_bounded` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `property_draft_keeps_the_original_target_and_values` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `running_guest_draft_uses_live_cpu_and_dynamic_memory_commands` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `secure_boot_review_preserves_vm_identity_and_does_not_claim_enforcement` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_properties.rs` `empty_custom_template_has_explicit_firmware_review_and_builtin_is_read_only` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_properties.rs` |
| `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` `compact_and_wide_boot_review_keeps_target_and_visible_controls` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` |
| `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` `force_review_captures_exact_tasks_and_refuses_changed_tasks_or_admin_blocks` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` |
| `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` `a_forced_action_over_a_migration_warns_and_waits_for_the_operator_to_accept_it` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` |
| `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` `unsupported_guest_and_missing_task_identity_do_not_offer_recovery_dispatch` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` |
| `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` `cancellation_review_is_bound_to_owned_operation_and_does_not_force` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_recovery_view_tests.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `snapshot_metadata_creation_is_editable_at_compact_and_wide_sizes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `restore_requires_explicit_shutdown_and_start_choices_and_captures_identity` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `snapshot_editor_keeps_reviewed_metadata_when_selection_changes` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `checkpoint_restore_offers_resume_and_retains_saved_memory` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `cloning_reviews_the_captured_snapshot_in_both_modes_and_stays_safe_in_demo` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `deletion_reviews_only_captured_owned_disks_and_lists_retained_disks` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `deletion_does_not_offer_cleanup_without_complete_disk_inventory` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `explicit_checkpoint_block_overrides_advertised_memory_mode` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `snapshot_review_requires_the_same_pool_parent_state_and_exact_capability` | pure | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |
| `crates/xenadmin-desktop/src/vm_snapshots.rs` `snapshot_widgets_review_captured_targets_and_demo_cannot_confirm_changes` | runtime | stays with the Rune crate until its view is ported | `crates/xenadmin-desktop/src/vm_snapshots.rs` |

## Mirror fixes

| Rune commit | summary | mirrored in |
| --- | --- | --- |
| | `xenadmin-desktop` frozen after this phase except release-blocking fixes | |

## Deviations

| package | what differs | reason |
| --- | --- | --- |
| phase 1 | Rune-free files that `use super::*` stayed in `xenadmin-desktop` | They are not standalone. Only `demo_details` moved. |
| phase 1 | `Preferences` stayed in the desktop crate | Its fields are desktop types. The stored JSON shape was not rewritten. |
| phase 1 | The command table is the target menu seed. Rune buttons are catalogued and still dispatch themselves | Rewiring every button is the GPUI shell, not this extraction. |
| phase 1 | Tier A scenes are marked ported because their inventory comes from `xenadmin_ui::demonstration` | The view is still Rune. |
| G0 | GPUI pointer delivery is an open limit | The Hyprland probe received no pointer event. Accepted, not a stop. |
| G0 | No hidden cursor style. The guest cursor is not drawn on the GPUI console | The local cursor stays visible. The Rune console still draws the guest cursor, because it can hide the local one. |
| G0 | Screencopy frames are an open limit | Fit and actual-size pixels were not read back. |
| G0 | Clipboard without focus is an open limit | The probe window never became the active window. |
| G0 | Route A stays. Route B is not vendored. Route C is not adopted | Physical keys use the second `wl_keyboard`. |
