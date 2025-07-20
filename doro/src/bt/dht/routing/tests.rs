use crate::dht::routing::NodeId;
use tracing::info;

#[test]
fn test_generate_node_id() {
    let node_id = NodeId::random();
    info!("routing: {:?}", node_id);
}