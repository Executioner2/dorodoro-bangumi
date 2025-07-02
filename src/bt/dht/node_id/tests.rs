use tracing::info;
use crate::dht::node_id::generate_node_id;

#[test]
fn test_generate_node_id() {
    let node_id = generate_node_id();
    info!("node_id: {:?}", node_id);
}