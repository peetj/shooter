use serde::{Deserialize, Serialize};

pub type ClientId = u32;

/// Client -> Server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum C2s {
    /// Input for a specific simulation tick.
    Input(Input),
}

/// Server -> Client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum S2c {
    Welcome { client_id: ClientId },
    /// Authoritative snapshot at a server tick.
    Snapshot(Snapshot),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Input {
    pub seq: u32,
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    /// Aim direction, quantized.
    pub aim_x: i16,
    pub aim_y: i16,
    pub shoot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u32,
    pub players: Vec<PlayerState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: ClientId,
    /// Position in millimeters (fixed-point), to keep messages small and deterministic.
    pub x_mm: i32,
    pub y_mm: i32,
    pub hp: u16,
    /// Last input sequence number received by the server for this player.
    pub last_input_seq: u32,
}

pub fn encode_c2s(msg: &C2s) -> Vec<u8> {
    // MVP: postcard over std Vec.
    postcard::to_stdvec(msg).expect("encode C2S")
}

pub fn decode_c2s(bytes: &[u8]) -> Result<C2s, postcard::Error> {
    postcard::from_bytes(bytes)
}

pub fn encode_s2c(msg: &S2c) -> Vec<u8> {
    postcard::to_stdvec(msg).expect("encode S2C")
}

pub fn decode_s2c(bytes: &[u8]) -> Result<S2c, postcard::Error> {
    postcard::from_bytes(bytes)
}
