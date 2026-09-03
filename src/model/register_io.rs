use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterReadRequest {
    pub id: u64,
    pub address: u64,
    pub size_bytes: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterReadResult {
    pub sequence: u64,
    pub value: Result<u64, String>,
}

pub type RegisterData = HashMap<u64, RegisterReadResult>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterWriteRequest {
    pub address: u64,
    pub size_bytes: u8,
    pub value: u64,
}
