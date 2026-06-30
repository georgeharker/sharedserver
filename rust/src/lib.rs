// Core library modules
pub mod core;

// Re-export commonly used types and functions
pub use core::{
    clients_lock_exists, delete_clients_lock, delete_server_lock, get_server_state,
    is_process_alive, parse_duration, read_clients_lock, read_server_lock, server_lock_exists,
    with_lock, write_clients_lock, write_server_lock, ClientInfo, ClientsLock, ServerLock,
    ServerState,
};
