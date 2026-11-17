//! Tests of the area.
use crate::Harness;

#[test]
fn dump() {
    for scene in ["connect", "connect-insecure", "connections", "connection-settings", "connection-trust", "certificate-review"] {
        let h = Harness::scene(scene);
        println!("== {scene}: surface {:?}", h.surface_id());
        for node in h.nodes() {
            println!("  {:<12} key={:<32} label={:?} value={:?} text={:?} disabled={:?} checked={:?}", node.role, node.key.clone().unwrap_or_default(), node.label, node.value, node.text, node.disabled, node.checked);
        }
    }
}
