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
            form.step = (form.step + 1).min(2);
            form.error = None;
        }
        Err(error) => form.error = Some(error),
    }
}
pub(super) fn operation(form: &Editor) -> Result<VmLifecycleOperation, String> {
    let (vcpus, memory) = resources(form)?;
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
pub(super) fn editor(form: &Editor, k: &Kit) -> El {
    let mut body = col().w_full().gap(12.0).child(heading(
        match form.step {
            0 => "1 of 3 · Identity",
            1 => "2 of 3 · CPU, memory and firmware",
            _ => "3 of 3 · Storage, network and installation",
        },
        k,
    ));
    match form.step {
        0=>body=body.child(names(form,true,k)).child(caption("The selected template is preserved. The new VM will stay halted until you choose to start it.",k).wrap()),
        1=>{
            body=body.child(TextInput::new(&form.vcpus).label("Virtual CPUs").on_edit(|s:&mut State|&mut editor_mut(s).expect("creation editor open").vcpus).build(k).key("vm-lifecycle-vcpus"))
                .child(TextInput::new(&form.memory).label("Memory (bytes, KiB, MiB or GiB)").on_edit(|s:&mut State|&mut editor_mut(s).expect("creation editor open").memory).build(k).key("vm-lifecycle-memory"))
                .child(caption("CPU startup and maximum counts match. Memory uses a fixed allocation; the host validates template constraints.",k).wrap()).child(heading("Firmware",k));
            let configurable=form.initial.details.vms.get(&form.source.id).and_then(|details|details.boot_policy.as_deref())==Some("BIOS order");
            for (value,label,key) in [(GuestFirmware::Inherited,"Inherit template firmware","inherit"),(GuestFirmware::Bios,"BIOS","bios"),(GuestFirmware::Uefi,"UEFI","uefi")] {
                body=body.child(Button::new(if form.firmware==value {format!("✓ {label}")}else{label.into()}).variant(ButtonVariant::Outline).disabled(value!=GuestFirmware::Inherited&&!configurable)
                    .on_press(move|s:&mut State|if let Some(form)=editor_mut(s){form.firmware=value;}).build(k).key(format!("vm-lifecycle-firmware-{key}")));
            }
            if !configurable {body=body.child(caption("This template does not advertise a configurable HVM boot policy. Its firmware is inherited.",k).wrap());}
            body=body.child(caption("This changes firmware mode only. Copying a host's BIOS identity and preparing a cloud-init configuration drive are not available in this wizard yet.",k).wrap());
        }
        _=>{
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
            body=body.child(choices("Available ISO images","vm-lifecycle-iso",media,|form,id|form.iso=Some(id),k));
        }
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
    lines.push("The new VM stays halted. Each completed step and created VM/disk/interface UUID remains in VM changes if a later step fails. Uncertain operations are not automatically repeated.".into());
    lines.push("Other changes in this pool are blocked while creation is active or its outcome is unknown. Resolve or explicitly acknowledge an unknown outcome to release this block.".into());
    lines
}
