//! What a command asks the front end to do. The model does not open a window.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogId {
    AddServer,
    ReconnectAs,
    ChangeServerPassword,
    CopyVm,
    ConvertToTemplate,
    DeleteVm,
    Power,
    Snapshot,
    Maintenance,
    Options,
    Help,
    Other(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardId {
    NewVm,
    Migrate,
    Import,
    Export,
    NewStorage,
    NewNetwork,
    ConfigureHa,
    Other(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageId {
    General,
    Other(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Dialog(DialogId),
    Wizard(WizardId),
    Properties(PageId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    OpenSurface(Surface),
    None,
}
