use super::*;

fn parse_bytes(text: &str, alignment: u64) -> Result<u64, String> {
    let compact = text.split_whitespace().collect::<String>();
    let (number, multiplier) = if let Some(n) = compact.strip_suffix("GiB") {
        (n, 1u64 << 30)
    } else if let Some(n) = compact.strip_suffix("MiB") {
        (n, 1 << 20)
    } else if let Some(n) = compact.strip_suffix("KiB") {
        (n, 1 << 10)
    } else {
        (compact.as_str(), 1)
    };
    number.parse::<u64>().ok().and_then(|n|n.checked_mul(multiplier)).filter(|n|*n>0&&*n<=i64::MAX as u64&&n%alignment==0)
        .ok_or_else(||format!("Enter a positive capacity in {alignment}-byte increments, as bytes or a whole number with KiB, MiB or GiB."))
}
fn resources(form: &Editor) -> Result<(u64, u64), String> {
    let cpus = form
        .vcpus
        .text
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|n| (1..=256).contains(n))
        .ok_or("Enter 1–256 virtual CPUs; the host validates its supported limit.")?;
    let memory = parse_bytes(&form.memory.text, 1024)?;
    let minimum = form
        .initial
        .details
        .vms
        .get(&form.source.id)
        .and_then(|details| details.memory_static_min)
        .unwrap_or(0)
        .max(16 << 20);
    if memory < minimum {
        return Err(format!(
            "Memory must be at least {} for this template.",
            storage_bytes(Some(minimum))
        ));
    }
    Ok((cpus, memory))
}
pub(super) fn next(s: &mut State) {
    let Some(form) = editor_mut(s) else { return };
    let valid = match form.step {
        0 => validate_name(&form.name.text)
            .and_then(|_| validate_description(&form.description.text))
            .map(|_| ()),
        _ => resources(form).map(|_| ()),
    };
    match valid {
        Ok(()) => {
            form.step = (form.step + 1).min(creation_cloud::last_step(form));
            form.error = None;
        }
        Err(error) => form.error = Some(error),
    }
}
pub(super) fn operation(form: &Editor) -> Result<VmLifecycleOperation, String> {
    let (vcpus, memory) = resources(form)?;
    if form.cloud {
        form.cloud_input.validate()?;
    }
    Ok(VmLifecycleOperation::Create {
        name: validate_name(&form.name.text)?,
        description: validate_description(&form.description.text)?,
        sr: form.sr.ok_or("Select a destination storage repository.")?,
        network: form.network.ok_or("Select a network.")?,
        iso: form.iso,
        vcpus,
        memory,
        disk_size: if form.disk_size.text.trim().is_empty() {
            None
        } else {
            Some(parse_bytes(&form.disk_size.text, 512)?)
        },
        firmware: form.firmware,
        bios_host: form.bios_host,
    })
}
fn inherited_disks(form: &Editor) -> Vec<VdiId> {
    form.initial
        .details
        .vbds
        .values()
        .filter(|vbd| vbd.vm == Some(form.source.id) && vbd.kind == "Disk")
        .filter_map(|vbd| vbd.vdi)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn firmware_allowed(form: &Editor, mode: GuestFirmware) -> bool {
    let requested = if mode == GuestFirmware::Bios {
        "bios"
    } else {
        "uefi"
    };
    inherited_disks(form).is_empty()
        || form
            .initial
            .details
            .vms
            .get(&form.source.id)
            .and_then(|d| d.firmware.as_deref())
            == Some(requested)
}
fn bios_eligible(form: &Editor) -> bool {
    let Some(d) = form.initial.details.vms.get(&form.source.id) else {
        return false;
    };
    d.bios_identity_empty == Some(true)
        && (d.domain_type.as_deref() == Some("hvm")
            || d.boot_policy.as_deref() == Some("BIOS order"))
        && (form.firmware == GuestFirmware::Bios
            || form.firmware == GuestFirmware::Inherited && d.firmware.as_deref() == Some("bios"))
}
pub(super) fn editor(form: &Editor, k: &Kit) -> El {
    let mut body = col().w_full().gap(12.0).child(heading(
        match form.step {
            0 if form.cloud => "1 of 4 · Identity",
            0 => "1 of 3 · Identity",
            1 if form.cloud => "2 of 4 · CPU, memory and firmware",
            1 => "2 of 3 · CPU, memory and firmware",
            2 if form.cloud => "3 of 4 · Storage, network and installation",
            2 => "3 of 3 · Storage, network and installation",
            _ => "4 of 4 · Cloud-init configuration",
        },
        k,
    ));
    match form.step {
        0=>body=body.child(names(form,true,k)).child(caption("The selected template is preserved. The new VM will stay halted until you choose to start it.",k).wrap()),
        1=>{
            body=body.child(TextInput::new(&form.vcpus).label("Virtual CPUs").on_edit(|s:&mut State|&mut editor_mut(s).expect("creation editor open").vcpus).build(k).key("vm-lifecycle-vcpus"))
                .child(TextInput::new(&form.memory).label("Memory (bytes, KiB, MiB or GiB)").on_edit(|s:&mut State|&mut editor_mut(s).expect("creation editor open").memory).build(k).key("vm-lifecycle-memory"))
                .child(caption("CPU startup and maximum counts match. Memory uses a fixed allocation; the host validates template constraints.",k).wrap()).child(heading("Firmware",k));
            let configurable=form.initial.details.vms.get(&form.source.id).is_some_and(|d| d.domain_type.as_deref()==Some("hvm") || d.boot_policy.as_deref()==Some("BIOS order"));
            for (value,label,key) in [(GuestFirmware::Inherited,"Inherit template firmware","inherit"),(GuestFirmware::Bios,"BIOS","bios"),(GuestFirmware::Uefi,"UEFI · Secure Boot off","uefi"),(GuestFirmware::SecureUefi,"UEFI · Secure Boot on","secure-uefi")] {
                body=body.child(Button::new(if form.firmware==value {format!("✓ {label}")}else{label.into()}).variant(ButtonVariant::Outline).disabled(value!=GuestFirmware::Inherited&&(!configurable || !firmware_allowed(form,value)))
                    .on_press(move|s:&mut State|if let Some(form)=editor_mut(s){form.firmware=value; form.bios_host=None;}).build(k).key(format!("vm-lifecycle-firmware-{key}")));
            }
            if !configurable {body=body.child(caption("This template does not advertise a configurable HVM boot policy. Its firmware is inherited.",k).wrap());}
            body=body.child(caption("Installed template disks keep their BIOS/UEFI mode. Secure Boot selects a policy; certificate readiness and guest enforcement must be checked after creation. Pool certificates are not changed.",k).wrap());
            if bios_eligible(form) {
                body=body.child(heading("BIOS identity and home host",k)).child(Button::new(if form.bios_host.is_none(){"✓ Use generic BIOS identity"}else{"Use generic BIOS identity"}).variant(ButtonVariant::Outline).on_press(|s:&mut State|if let Some(f)=editor_mut(s){f.bios_host=None;}).build(k).key("vm-lifecycle-bios-generic"));
                let hosts=form.initial.hosts.values().map(|host|(host.id,format!("Copy from {}",host.name),form.bios_host==Some(host.id))).collect();
                body=body.child(choices("Copy host BIOS identity","vm-lifecycle-bios-host",hosts,|f,id|f.bios_host=Some(id),k));
                body=body.child(caption("Copying BIOS identity also sets the new VM's home-host preference to that exact host. The API only accepts an empty BIOS identity; this cannot be overwritten later.",k).wrap());
            }

        }
        2=>{
            body=body.child(storage_choices(form,k));
            let disks=inherited_disks(form);
            body=body.child(heading("Template disks",k));
            if disks.is_empty() {body=body.child(caption("No attached data disks are reported. Enter a capacity below to create an installation disk, or leave it blank for a diskless template.",k).wrap());}
            for disk in disks {body=body.child(caption(format!("Copy {} to the selected repository.",disk_label(&form.initial,disk)),k).wrap_anywhere());}
            body=body.child(TextInput::new(&form.disk_size).label("Additional new disk capacity (optional)").placeholder("32 GiB")
                .on_edit(|s:&mut State|&mut editor_mut(s).expect("creation editor open").disk_size).build(k).key("vm-lifecycle-disk-size"))
                .child(caption("An entered capacity creates an additional blank disk. Existing template disks are preserved as copies.",k).wrap());
            let mut networks:Vec<_>=form.initial.networks.iter().map(|network|{let id=NetworkId::from(network.uuid);(id,network_label(&form.initial,id),form.network==Some(id))}).collect();
            networks.sort_by_cached_key(|(_,name,_)|name.to_lowercase());
            body=body.child(choices("Network for all VM interfaces","vm-lifecycle-network",networks,|form,id|form.network=Some(id),k))
                .child(caption("All inherited NICs use this network. A NIC is added if the template has none.",k).wrap())
                .child(heading("Installation media",k))
                .child(Button::new(if form.iso.is_none(){"✓ Keep inherited installation media"}else{"Keep inherited installation media"}).variant(ButtonVariant::Outline)
                    .on_press(|s:&mut State|if let Some(form)=editor_mut(s){form.iso=None;}).build(k).key("vm-lifecycle-no-iso"));
            let mut media:Vec<_>=form.initial.details.vdis.values().filter(|disk|disk.kind=="iso"&&disk.missing==Some(false)&&disk.read_only==Some(true))
                .map(|disk|(disk.id,disk_label(&form.initial,disk.id),form.iso==Some(disk.id))).collect();
            media.sort_by_cached_key(|(_,name,_)|name.to_lowercase());
            body=body.child(choices("Available ISO images","vm-lifecycle-iso",media,|form,id|form.iso=Some(id),k))
                .child(Checkbox::new(form.cloud).label("Configure cloud-init before first boot").on_toggle(|s:&mut State|if let Some(f)=editor_mut(s){f.cloud = !f.cloud;if !f.cloud {f.cloud_input=creation_cloud::Draft::default();}}).build(k).key("creation-cloud-enable"))
                .child(caption("For templates with cloud-init and NoCloud support installed. After successful VM creation, review a private configuration drive for that exact new VM. The VM stays halted.",k).wrap());
        }
        _=>body=body.child(creation_cloud::inputs(form,k)),
    }
    body
}
pub(super) fn review(form: &Editor) -> Vec<String> {
    let (cpus, memory) = resources(form).expect("creation was validated");
    let sr = form.sr.expect("creation storage selected");
    let network = form.network.expect("creation network selected");
    let mut lines = vec![
        format!(
            "Create VM “{}” from template “{}”.",
            form.name.text.trim(),
            form.source.name
        ),
        format!("Description: {}", form.description.text),
        format!(
            "Resources: {cpus} virtual CPUs; {} memory; {:?} firmware.",
            storage_bytes(Some(memory)),
            form.firmware
        ),
        format!("Destination storage: {}", storage_label(&form.initial, sr)),
        format!(
            "Network for all interfaces: {}",
            network_label(&form.initial, network)
        ),
    ];
    if let Some(host) = form.bios_host {
        lines.push(format!("Copy BIOS identity from {} ({host}) and set its home-host preference to that host before provisioning. This does not guarantee placement.",form.initial.hosts.get(&host).map_or("Unknown host",|h|h.name.as_str())));
    }
    if form.firmware == GuestFirmware::SecureUefi {
        lines.push("Request Secure Boot policy. This is not proof of guest enforcement; inspect guest/pool certificate readiness after creation.".into());
    }
    let disks = inherited_disks(form);
    if disks.is_empty() {
        lines.push("Clone the diskless template and provision it without inherited disk-layout instructions.".into());
    }
    for disk in disks {
        lines.push(format!(
            "Full copy of template disk: {}",
            disk_label(&form.initial, disk)
        ));
    }
    if !form.disk_size.text.trim().is_empty() {
        let size = parse_bytes(&form.disk_size.text, 512).expect("disk size validated");
        lines.push(format!(
            "Create an additional blank disk: {size} bytes ({}).",
            storage_bytes(Some(size))
        ));
    }
    lines.push(
        form.iso
            .map(|id| {
                format!(
                    "Replace the first CD drive's media with {}. A CD drive is created if needed.",
                    disk_label(&form.initial, id)
                )
            })
            .unwrap_or_else(|| {
                "Retain inherited installation media; no ISO replacement was selected.".into()
            }),
    );
    if form.cloud {
        lines.push(format!("Cloud-init: hostname {}; {} bytes user-data; {} bytes network-config. After successful creation, review a new NoCloud drive on the selected SR for the exact created VM. Closing the wizard discards this unsaved configuration; it does not cancel creation.",form.cloud_input.hostname.text.trim(),form.cloud_input.user_data.text.len(),form.cloud_input.network.text.len()));
        lines.push("User-data contents are not included in creation history. Configuration is a separately reviewed, tracked operation; neither stage starts the guest.".into());
    }
    lines.push("The new VM stays halted. Each completed step and created VM/disk/interface UUID remains in VM changes if a later step fails. Uncertain operations are not automatically repeated.".into());
    lines.push("Other changes in this pool are blocked while creation is active or its outcome is unknown. Resolve or explicitly acknowledge an unknown outcome to release this block.".into());
    lines
}
