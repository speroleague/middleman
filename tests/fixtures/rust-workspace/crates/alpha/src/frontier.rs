#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub owner: String,
    pub expires_at: u64,
}

pub fn renew(lease: &Lease, owner: &str, now: u64, duration: u64) -> Option<Lease> {
    if lease.owner != owner || lease.expires_at <= now {
        return None;
    }

    Some(Lease {
        owner: lease.owner.clone(),
        expires_at: now.checked_add(duration)?,
    })
}
