use super::*;
use std::collections::BTreeSet;
use xenadmin_core::{NetworkTopologyOperation, ResourceAction, ReviewedVlan, VlanUplink};

pub(super) struct Editor {
    network: NetworkId,
    tag: TextEdit,
    selection: Selection,
}
enum Selection {
    Create(Vec<VlanUplink>),
    Retag(Vec<ReviewedVlan>),
}
fn editor(s: &mut State) -> &mut Editor {
    let Form::Topology(value) = &mut edit(s).form else {
        unreachable!()
    };
    value
}
fn uplinks(i: &Inventory, source: NetworkId) -> Result<Vec<VlanUplink>, String> {
    i.hosts.keys().map(|host| {
        let candidates=i.details.pifs.values().filter(|p|p.host==Some(*host)&&p.network==Some(source)&&p.vlan==Some(-1)&&p.managed==Some(true)).collect::<Vec<_>>();
        let [p]=candidates.as_slice() else {return Err("Choose an uplink network with exactly one managed untagged interface on every pool host".into())};
        Ok(VlanUplink {pif:p.id,expected:PifIdentity::from_inventory(p).ok_or("Uplink identity is incomplete")?})
    }).collect()
}
fn retag(i: &Inventory, network: NetworkId) -> Result<Vec<ReviewedVlan>, String> {
    let members = i
        .details
        .pifs
        .values()
        .filter(|p| p.network == Some(network));
    members
        .map(|p| {
            let matches = i
                .details
                .vlans
                .values()
                .filter(|v| v.untagged_pif == Some(p.id))
                .collect::<Vec<_>>();
            let [v] = matches.as_slice() else {
                return Err("Every interface on this network must be a VLAN interface".into());
            };
            let parent = v
                .tagged_pif
                .and_then(|id| i.details.pifs.get(&id))
                .ok_or("VLAN uplink is unavailable")?;
            Ok(ReviewedVlan {
                vlan: v.id,
                child: p.id,
                uplink: VlanUplink {
                    pif: parent.id,
                    expected: PifIdentity::from_inventory(parent)
                        .ok_or("Uplink identity is incomplete")?,
                },
                tag: v
                    .tag
                    .and_then(|v| u16::try_from(v).ok())
                    .ok_or("Current VLAN tag is unavailable")?,
                attached: p
                    .currently_attached
                    .ok_or("Interface attachment state is unavailable")?,
            })
        })
        .collect()
}
fn open(s: &mut State, changing: bool) {
    let result = (|| {
        let w = s
            .network_workspace
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == w.pool)
            .ok_or("The pool changed")?;
        let network = w.network.ok_or("Choose a network")?;
        Ok::<_, String>(Editor {
            network,
            tag: TextEdit::new("100"),
            selection: if changing {
                Selection::Retag(retag(i, network)?)
            } else {
                Selection::Create(vec![])
            },
        })
    })();
    match result {
        Ok(value) => set_form(s, Form::Topology(Box::new(value))),
        Err(error) => edit(s).error = Some(error),
    }
}
pub(super) fn buttons(enabled: bool, k: &Kit) -> El {
    row()
        .gap(8.0)
        .flex_wrap()
        .child(
            Button::new("Create pool-wide VLANs…")
                .disabled(!enabled)
                .on_press(|s: &mut State| open(s, false))
                .build(k)
                .key("topology-create"),
        )
        .child(
            Button::new("Retag pool-wide VLANs…")
                .disabled(!enabled)
                .on_press(|s: &mut State| open(s, true))
                .build(k)
                .key("topology-retag"),
        )
}
fn operation(editor: &Editor) -> Result<NetworkTopologyOperation, String> {
    let tag = editor
        .tag
        .text
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|v| *v <= 4094)
        .ok_or("Enter a VLAN tag from 0 to 4094")?;
    Ok(match &editor.selection {
        Selection::Create(uplinks) => NetworkTopologyOperation::CreateVlans {
            uplinks: uplinks.clone(),
            tag,
        },
        Selection::Retag(interfaces) => NetworkTopologyOperation::RetagVlans {
            interfaces: interfaces.clone(),
            tag,
        },
    })
}
fn review(
    editor: &Editor,
    i: &Inventory,
) -> Result<(NetworkTopologyOperation, Vec<String>), String> {
    let operation = operation(editor)?;
    let available = operation.availability(i, editor.network);
    if !available.is_available() {
        return Err(available.explanation());
    }
    let mut lines=vec![format!("Network: {} ({})",name(i,editor.network),editor.network),format!("New VLAN tag: {} · all {} pool hosts",editor.tag.text.trim(),i.hosts.len()),
        "Every pool host must be online. Interfaces are changed one host at a time. A failure can leave a partially changed network; verified identities remain in operation history.".into()];
    match &editor.selection {
        Selection::Create(uplinks) => {
            lines.push("Create and plug a new VLAN interface on each reviewed uplink. Tag 0 requests the API's native VLAN.".into());
            for p in uplinks {
                lines.push(format!(
                    "{} · host {} · uplink {}",
                    pif_name(i, p.pif),
                    p.expected.host,
                    p.pif
                ));
            }
        }
        Selection::Retag(interfaces) => {
            lines.push("Delete each old VLAN interface, create its replacement with the new tag, and restore its plugged or unplugged state. This requires an unused network with no VM attachments, IP configuration, interface purpose, protection or nested topology. No automatic rollback is performed.".into());
            for p in interfaces {
                lines.push(format!(
                    "{} · host {} · uplink {} · old VLAN {} / interface {} · tag {} → {} · {}",
                    pif_name(i, p.uplink.pif),
                    p.uplink.expected.host,
                    p.uplink.pif,
                    p.vlan,
                    p.child,
                    p.tag,
                    editor.tag.text.trim(),
                    if p.attached {
                        "plugged"
                    } else {
                        "unplugged (preserved)"
                    }
                ));
            }
        }
    }
    lines.push("A lost reply stops the workflow. Recheck inspects existing receipts; it never repeats a request or starts remaining host steps.".into());
    Ok((operation, lines))
}
pub(super) fn prepare(s: &mut State) {
    let result = (|| {
        let w = s
            .network_workspace
            .as_ref()
            .ok_or("Close and reopen networking")?;
        let Form::Topology(editor) = &w.form else {
            return Err("Choose a topology change".into());
        };
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == w.pool)
            .ok_or("The pool changed")?;
        let (operation, lines) = review(editor, i)?;
        Ok::<_, String>((
            ResourceTarget::Network(editor.network),
            ResourceAction::NetworkTopology(operation),
            lines,
        ))
    })();
    match result {
        Ok((target, operation, lines)) => prepare_action(s, target, operation, lines),
        Err(error) => edit(s).error = Some(error),
    }
}
pub(super) fn review_unavailable(s: &State, w: &Workspace) -> Option<String> {
    let review = w.review.as_ref()?;
    let ResourceAction::NetworkTopology(op) = &review.operation else {
        return None;
    };
    let ResourceTarget::Network(network) = review.target else {
        return Some("Network identity changed".into());
    };
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == w.pool) else {
        return Some("The pool changed. Close and reopen networking.".into());
    };
    let available = op.availability(i, network);
    (!available.is_available()).then(|| available.explanation())
}
pub(super) fn form(value: &Editor, i: &Inventory, k: &Kit) -> El {
    let mut body = col()
        .gap(10.0)
        .child(heading(
            if matches!(value.selection, Selection::Create(_)) {
                "Pool-wide VLAN creation"
            } else {
                "Pool-wide VLAN retagging"
            },
            k,
        ))
        .child(
            caption(
                format!(
                    "Network: {} · {} pool hosts",
                    name(i, value.network),
                    i.hosts.len()
                ),
                k,
            )
            .wrap(),
        )
        .child(caption("VLAN tag (0–4094)", k))
        .child(
            TextInput::new(&value.tag)
                .label("VLAN tag (0–4094)")
                .on_edit(|s: &mut State| &mut editor(s).tag)
                .build(k),
        );
    match &value.selection {
        Selection::Create(selected) => {
            body=body.child(caption("Choose an uplink network. It must have exactly one managed untagged interface on every pool host.",k).wrap());
            let sources = i
                .details
                .pifs
                .values()
                .filter_map(|p| p.network)
                .filter(|n| *n != value.network)
                .collect::<BTreeSet<_>>();
            for source in sources {
                let Ok(choices) = uplinks(i, source) else {
                    continue;
                };
                let chosen = selected == &choices;
                body = body.child(
                    Button::new(format!(
                        "{}{}",
                        if chosen { "✓ " } else { "" },
                        name(i, source)
                    ))
                    .on_press(move |s: &mut State| {
                        editor(s).selection = Selection::Create(choices.clone())
                    })
                    .build(k),
                );
            }
            for p in selected {
                body = body.child(
                    caption(format!("{} · {}", pif_name(i, p.pif), p.pif), k).wrap_anywhere(),
                );
            }
        }
        Selection::Retag(interfaces) => {
            body=body.child(caption("The captured interfaces will be replaced. Move VM attachments and IP configuration off this network first.",k).wrap());
            for p in interfaces {
                body = body.child(
                    caption(
                        format!(
                            "{} · VLAN {} · tag {} · {}",
                            pif_name(i, p.uplink.pif),
                            p.vlan,
                            p.tag,
                            if p.attached { "plugged" } else { "unplugged" }
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
            }
        }
    }
    if let Err(reason) = review(value, i) {
        body = body.child(caption(reason, k).wrap());
    }
    body
}
pub(super) fn scene(s: &mut State) {
    super::scene(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let underlay = NetworkId::from(uuid::Uuid::from_u128(0x51000000000000000000000000000001));
    let network = NetworkId::from(i.networks[0].uuid);
    let prototype = i
        .details
        .pifs
        .values()
        .find(|p| p.network == Some(underlay))
        .unwrap()
        .clone();
    i.details.pifs.clear();
    i.details.vlans.clear();
    i.details.vifs.clear();
    for (index, host) in i.hosts.keys().enumerate() {
        let mut p = prototype.clone();
        p.id = uuid::Uuid::from_u128(0x53000000000000000000000000000001 + index as u128).into();
        p.host = Some(*host);
        p.device = "eth1".into();
        p.mac = format!("02:00:00:00:00:{:02x}", index + 1);
        i.details.pifs.insert(p.id, p);
    }
    for class in ["pif", "vif", "vlan"] {
        i.details
            .classes
            .insert(class.into(), xenadmin_core::ClassAvailability::Available);
    }
    let selection = Selection::Create(uplinks(i, underlay).unwrap());
    set_form(
        s,
        Form::Topology(Box::new(Editor {
            network,
            tag: TextEdit::new("100"),
            selection,
        })),
    );
    prepare(s);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn retag_scene(s: &mut State) {
        scene(s);
        let w = s.network_workspace.as_ref().unwrap();
        let network = w.network.unwrap();
        let ResourceAction::NetworkTopology(NetworkTopologyOperation::CreateVlans {
            uplinks, ..
        }) = &w.review.as_ref().unwrap().operation
        else {
            panic!()
        };
        let uplinks = uplinks.clone();
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        for (index, parent) in uplinks.iter().enumerate() {
            let mut child = i.details.pifs[&parent.pif].clone();
            child.id = uuid::Uuid::from_u128(600 + index as u128).into();
            child.network = Some(network);
            child.vlan = Some(40);
            child.physical = Some(false);
            child.currently_attached = Some(index == 0);
            let vlan = uuid::Uuid::from_u128(700 + index as u128).into();
            i.details.vlans.insert(
                vlan,
                xenadmin_core::Vlan {
                    id: vlan,
                    tagged_pif: Some(parent.pif),
                    untagged_pif: Some(child.id),
                    tag: Some(40),
                },
            );
            i.details.pifs.insert(child.id, child);
        }
        open(s, true);
    }
    #[test]
    fn pool_vlan_review_captures_every_host_at_compact_and_wide_sizes_without_demo_dispatch() {
        for retag in [false, true] {
            for size in [(513.0, 652.0), (1440.0, 900.0)] {
                let mut s = crate::init();
                if retag {
                    retag_scene(&mut s);
                    prepare(&mut s);
                } else {
                    scene(&mut s);
                }
                let i = s.inventory.as_ref().unwrap();
                let w = s.network_workspace.as_ref().unwrap();
                let review = w.review.as_ref().unwrap();
                let ResourceAction::NetworkTopology(op) = &review.operation else {
                    panic!()
                };
                assert!(op.availability(i, w.network.unwrap()).is_available());
                assert!(review.lines.iter().any(|v| v.contains("never repeats")));
                if let NetworkTopologyOperation::RetagVlans { interfaces, .. } = op {
                    assert_eq!(interfaces.len(), i.hosts.len());
                    assert!(review.lines.iter().any(|v| v.contains("old VLAN")));
                }
                let mut runtime = rune::Runtime::new(
                    s,
                    crate::view,
                    size,
                    rune::Env {
                        system_dark: true,
                        scale: 1.0,
                    },
                );
                for _ in 0..30 {
                    runtime.frame(1.0 / 60.0);
                }
                let (x, y) = runtime.find("network-confirm").unwrap();
                assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
                runtime.pointer_down(x, y);
                runtime.pointer_up(x, y);
                runtime.frame(0.0);
                assert!(runtime.state.resource_operations.is_empty());
            }
        }
    }
    #[test]
    fn topology_review_refuses_changed_pool_graph_and_keeps_captured_identities() {
        let mut s = crate::init();
        scene(&mut s);
        let original = s
            .network_workspace
            .as_ref()
            .unwrap()
            .review
            .as_ref()
            .unwrap()
            .operation
            .clone();
        let ResourceAction::NetworkTopology(NetworkTopologyOperation::CreateVlans {
            uplinks, ..
        }) = &original
        else {
            panic!()
        };
        let pif = uplinks[0].pif;
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .pifs
            .get_mut(&pif)
            .unwrap()
            .mac = "02:00:00:00:ff:ff".into();
        assert!(review_unavailable(&s, s.network_workspace.as_ref().unwrap()).is_some());
        assert_eq!(
            s.network_workspace
                .as_ref()
                .unwrap()
                .review
                .as_ref()
                .unwrap()
                .operation,
            original
        );
        Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id = uuid::Uuid::new_v4().into();
        assert!(
            review_unavailable(&s, s.network_workspace.as_ref().unwrap())
                .unwrap()
                .contains("pool changed")
        );
    }
    #[test]
    fn retag_review_refuses_protection_consumers_missing_inventory_and_unchanged_tag() {
        for change in 0..5 {
            let mut s = crate::init();
            retag_scene(&mut s);
            let network = s.network_workspace.as_ref().unwrap().network.unwrap();
            let i = Arc::make_mut(s.inventory.as_mut().unwrap());
            let child = i
                .details
                .pifs
                .values()
                .find(|p| p.network == Some(network))
                .unwrap()
                .id;
            match change {
                0 => i.details.pifs.get_mut(&child).unwrap().disallow_unplug = Some(true),
                1 => {
                    i.details.pifs.get_mut(&child).unwrap().management_purpose =
                        Some("Storage".into())
                }
                2 => {
                    i.details.classes.remove("vlan");
                }
                3 => {
                    i.hosts.pop_first();
                }
                _ => editor(&mut s).tag = TextEdit::new("40"),
            }
            prepare(&mut s);
            assert!(s.network_workspace.as_ref().unwrap().review.is_none());
            assert!(s.network_workspace.as_ref().unwrap().error.is_some());
        }
    }
    #[test]
    fn vlan_zero_is_explicit_and_invalid_tags_cannot_reach_review() {
        for tag in ["0", "4094", "4095", "-1", "garbage"] {
            let mut s = crate::init();
            scene(&mut s);
            edit(&mut s).review = None;
            editor(&mut s).tag = TextEdit::new(tag);
            prepare(&mut s);
            assert_eq!(
                s.network_workspace.as_ref().unwrap().review.is_some(),
                matches!(tag, "0" | "4094"),
                "{tag}"
            );
        }
    }
    #[test]
    fn topology_export_contains_only_public_created_and_removed_ids() {
        let mut s = crate::init();
        scene(&mut s);
        let w = s.network_workspace.as_ref().unwrap();
        let row:xenadmin_core::ResourceOperationRecord=serde_json::from_value(serde_json::json!({"id":OperationId::new(),"pool":w.pool,"target":ResourceTarget::Network(w.network.unwrap()),"action":w.review.as_ref().unwrap().operation,"state":"PartiallyCompleted","progress":null,"detail":"Synthetic partial VLAN change","created_at_ms":0,"can_cancel":false,"cancel_requested":false,"steps":[],"created":null,"topology_changes":{"created_vlans":[uuid::Uuid::from_u128(800)],"created_pifs":[uuid::Uuid::from_u128(801)],"removed_vlans":[uuid::Uuid::from_u128(700)],"removed_pifs":[uuid::Uuid::from_u128(701)]}})).unwrap();
        let export = serde_json::to_value(crate::activity_export::Entry::resource(&row)).unwrap();
        assert_eq!(export["created"].as_array().unwrap().len(), 2);
        assert_eq!(export["removed"].as_array().unwrap().len(), 2);
        assert!(!export.to_string().contains("OpaqueRef:"));
    }
}
