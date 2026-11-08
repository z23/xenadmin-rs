use crate::{Error, TransferFile, VmId, VmWorkflowBatch};

pub(crate) fn bind(
    selection: &VmWorkflowBatch,
    outputs: Vec<(VmId, TransferFile)>,
) -> Result<Vec<TransferFile>, Error> {
    if !selection.is_export() {
        return if outputs.is_empty() {
            Ok(Vec::new())
        } else {
            Err(Error::InvalidResourceCommand)
        };
    }
    if outputs.len() != selection.items.len() {
        return Err(Error::TransferFile);
    }
    let mut outputs = outputs
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    if outputs.len() != selection.items.len() {
        return Err(Error::TransferFile);
    }
    let files = selection
        .items
        .iter()
        .map(|i| outputs.remove(&i.vm).ok_or(Error::TransferFile))
        .collect::<Result<Vec<_>, _>>()?;
    validate(&files)?;
    Ok(files)
}
pub(crate) fn validate(files: &[TransferFile]) -> Result<(), Error> {
    for (index, file) in files.iter().enumerate() {
        if !file.path.is_absolute()
            || file.path.as_os_str().len() > 4096
            || file.identity.is_some()
            || file.manifest.is_some()
            || file.disk.is_some()
            || file.range.is_some()
            || file.ovf.is_some()
            || file.host_backup.is_some()
        {
            return Err(Error::TransferFile);
        }
        for other in &files[..index] {
            if file.path.starts_with(&other.path)
                || other.path.starts_with(&file.path)
                || file.parent.same_inode(&other.parent)
                    && file.path.file_name() == other.path.file_name()
            {
                return Err(Error::TransferDestinationExists);
            }
        }
    }
    Ok(())
}
pub(crate) fn preflight(files: &[TransferFile]) -> Result<(), Error> {
    validate(files)?;
    for file in files {
        let current = TransferFile::destination(&file.path)?;
        if current.path != file.path || !current.parent.same_inode(&file.parent) {
            return Err(Error::TransferFileChanged);
        }
        crate::transfer::files::parent_handle(file)?;
    }
    Ok(())
}
