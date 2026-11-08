use super::*;
type Outcome = Result<CredentialOutcome, xenadmin_core::CredentialStoreError>;

pub(super) struct CredentialJob {
    #[cfg(not(test))]
    task: xenadmin_core::CredentialTask,
    #[cfg(test)]
    pub(super) result: std::cell::RefCell<Option<Outcome>>,
}
impl CredentialJob {
    pub(super) fn load(key: CredentialKey, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::load(key, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (key, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub(super) fn save(key: CredentialKey, credentials: Credentials, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::save(key, credentials, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (key, credentials, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub(super) fn remove(id: ProfileId, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::remove(id, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (id, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub(super) fn take_result(&self) -> Option<Outcome> {
        #[cfg(not(test))]
        {
            self.task.take_result()
        }
        #[cfg(test)]
        {
            self.result.borrow_mut().take()
        }
    }
}
