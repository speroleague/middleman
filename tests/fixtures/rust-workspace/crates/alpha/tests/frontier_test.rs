use alpha::Lease;
use alpha::frontier::renew;

#[test]
fn only_the_active_owner_can_renew() {
    let lease = Lease { owner: "worker-a".into(), expires_at: 20 };

    assert_eq!(renew(&lease, "worker-a", 10, 30).unwrap().expires_at, 40);
    assert!(renew(&lease, "worker-b", 10, 30).is_none());
    assert!(renew(&lease, "worker-a", 20, 30).is_none());
}
