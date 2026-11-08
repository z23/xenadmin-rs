use super::*;
use crate::{OperationState as State, management_ipv6_bond::Receipt};
impl crate::operations::Journal {
    pub(crate) fn claim_ipv6_predecessor(
        &self,
        r: &mut ResourceOperationRecord,
    ) -> Result<(), Error> {
        let ResourceAction::ManagementIpv6Bond(op) = &r.action else {
            return Ok(());
        };
        let Some(id) = op.predecessor else {
            return Ok(());
        };
        let mut parent = self.resource_load(id)?.ok_or(Error::OperationConflict)?;
        let old = parent
            .management_ipv6_bond_plan
            .as_ref()
            .ok_or(Error::OperationConflict)?;
        let mut expected = old.intent.clone();
        expected.predecessor = Some(id);
        expected.purpose = op.purpose;
        if parent.pool != r.pool
            || parent.target != r.target
            || parent.state != State::PartiallyCompleted
            || old.successor.is_some()
            || old.intent.purpose == crate::ManagementIpv6Next::LeavePartial
            || old.lineage >= 16
            || old.intent == **op
            || expected != **op
            || old.current().dispatched && !old.current().verified
            || old.depth == 4 && old.current().verified
        {
            return Err(Error::OperationConflict);
        }
        let mut child = old.clone();
        child.lineage += 1;
        child.predecessor = Some(id);
        child.successor = None;
        child.intent = (**op).clone();
        child.expires_at_ms = r.created_at_ms.saturating_add(10 * 60 * 1000);
        if op.purpose == crate::ManagementIpv6Next::LeavePartial {
            r.state = State::PartiallyCompleted;
            r.can_cancel = false;
            r.task = child.current().task.clone();
            r.result_reference = child.current().result.clone();
            r.detail="The verified partial configuration was deliberately left in place. The pool reservation is released. No host calls were sent; the retained temporary route and completed stages remain recorded. Continue is disabled; future changes require a new independent review.".into();
        } else {
            child.registered = false;
            child.temporary_route_verified = false;
            child.final_route_verified = false;
            if old.current().verified {
                child.depth += 1;
                child.receipts.push(Receipt::pending(child.stage()));
            }
        }
        child.project(r);
        r.management_ipv6_bond_plan = Some(child);
        let p = parent.management_ipv6_bond_plan.as_mut().unwrap();
        p.successor = Some(r.id);
        parent.management_ipv6_bond = Some(p.evidence(&parent.state));
        self.resource_save(&mut parent)
    }
    pub(crate) fn ipv6_evidence(
        &self,
        e: &ResourceOperationRecord,
    ) -> Result<ResourceOperationRecord, Error> {
        for _ in 0..16 {
            let mut r = self.resource_load(e.id)?.ok_or(Error::Journal)?;
            if r.pool != e.pool || r.action != e.action || r.target != e.target {
                return Err(Error::OperationConflict);
            }
            let new = e.management_ipv6_bond_plan.as_ref().ok_or(Error::Journal)?;
            let p = r.management_ipv6_bond_plan.as_mut().ok_or(Error::Journal)?;
            let mut merged = p.clone();
            let v = merged.current_mut();
            let n = new.current();
            if v.task
                .as_ref()
                .zip(n.task.as_ref())
                .is_some_and(|(a, b)| a != b)
            {
                return Err(Error::JournalChanged);
            }
            if v.task.is_none() {
                v.task = n.task.clone();
            }
            if v.task_uuid.is_none() {
                v.task_uuid = n.task_uuid;
            }
            if v.result.is_none() {
                v.result = n.result.clone();
            }
            v.completed |= n.completed;
            v.ended |= n.ended;
            v.verified |= n.verified;
            merged.temporary_route_verified |= new.temporary_route_verified;
            merged.final_route_verified |= new.final_route_verified;
            match (&mut merged.change_baseline, &new.change_baseline) {
                (Some(a), Some(b)) => a.merge(b)?,
                (None, None) => {}
                _ => return Err(Error::JournalChanged),
            }
            if !p.accepts(&merged) || !new.accepts(&merged) {
                return Err(Error::JournalChanged);
            }
            r.task = merged.current().task.clone();
            r.result_reference = merged.current().result.clone();
            merged.project(&mut r);
            r.management_ipv6_bond_plan = Some(merged);
            match self.resource_save(&mut r) {
                Ok(()) => return Ok(r),
                Err(Error::JournalChanged) => {}
                Err(e) => return Err(e),
            }
        }
        Err(Error::JournalChanged)
    }
}
