use alpha::Lease;

fn main() {
    let lease = Lease { owner: "worker-a".into(), expires_at: 20 };
    println!("{}", lease.owner);
}
