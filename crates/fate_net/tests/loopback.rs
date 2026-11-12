//! End-to-end loopback test across the public client and server roles.

use fate_net::{ClientSession, Limits, ServerSession};
use fate_sim::simulation::SimulationState;

fn apply_input(state: &mut SimulationState, input: &[u8]) -> Result<(), String> {
    let steps = input.first().copied().unwrap_or(0) as u32;
    for _ in 0..steps {
        let _ = state.random_u64();
    }
    Ok(())
}

#[test]
fn client_and_server_exchange_inputs_and_snapshots() {
    let limits = Limits::default();
    let mut server = ServerSession::new(limits).unwrap();
    let mut client = ClientSession::new(limits).unwrap();
    let mut last_snapshot_state = Vec::new();

    for step in 0..24_u8 {
        let input = client.make_input(vec![step % 4]).unwrap();
        let acked = server.accept_input(&input, apply_input).unwrap();
        assert_eq!(acked, u64::from(step) + 1);

        let snapshot = server.snapshot_bytes().unwrap();
        let info = client.receive_snapshot(&snapshot).unwrap();

        assert_eq!(info.acked_client_tick, acked);
        assert_eq!(info.server_tick, server.server_tick());
        assert_eq!(info.state_hash, server.state().state_hash());
        let restored = SimulationState::from_state_bytes(&info.state).unwrap();
        assert_eq!(restored.tick(), info.server_tick);
        assert_eq!(restored.state_hash(), server.state().state_hash());
        last_snapshot_state = info.state;
    }

    assert_eq!(client.next_input_tick(), 25);
    assert_eq!(client.last_acked_input_tick(), 24);
    assert_eq!(client.last_server_tick(), 24);

    // The snapshot carries the RNG stream, so a restored client state produces
    // the same future values as the server.
    let mut server_next = SimulationState::from_state_bytes(&server.state().state_bytes()).unwrap();
    let mut client_next = SimulationState::from_state_bytes(&last_snapshot_state).unwrap();
    for _ in 0..8 {
        assert_eq!(server_next.random_u64(), client_next.random_u64());
    }
}
