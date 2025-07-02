#[cfg(test)]
mod tests;

use alloc::borrow::Cow;
use core::fmt::Formatter;
use anyhow::{anyhow, Error};
use bendy::value::Value;
use bincode::{Decode, Encode};
use rand::RngCore;
use sha1::{Digest, Sha1};

#[derive(Clone, Eq, PartialEq, Encode, Decode)]
pub struct NodeId([u8; 20]);

impl NodeId {
    pub fn new(id: [u8; 20]) -> Self {
        Self(id)
    }
    
    pub fn distance(&self, other: &Self) -> [u8; 20] {
        let mut res = [0u8; 20];
        for i in 0..20 {
            res[i] = self[i] ^ other[i];
        }
        res
    }
    
    pub fn to_value_bytes(&self) -> Value {
        Value::Bytes(Cow::from(&self.0))
    }
}

impl TryFrom<&[u8]> for NodeId {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != 20 {
            return Err(anyhow!("Invalid node ID length: {}", value.len()));
        }
        Ok(Self::new(value.try_into()?))
    }
}

impl From<[u8; 20]> for NodeId {
    fn from(value: [u8; 20]) -> Self {
        Self::new(value)
    }
}

impl std::ops::Index<usize> for NodeId {
    type Output = u8;
    fn index(&self, index: usize) -> &u8 {
        &self.0[index]
    }
}

impl std::ops::IndexMut<usize> for NodeId {
    fn index_mut(&mut self, index: usize) -> &mut u8 {
        &mut self.0[index]
    }
}

impl std::fmt::Debug for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeId({})", hex::encode(&self.0))
    }
}

impl std::str::FromStr for NodeId {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|e| anyhow!("Invalid hex string: {}", e))?;
        if bytes.len() == 20 {
            Ok(Self(bytes.try_into().unwrap()))
        } else {
            Err(anyhow!("Invalid node ID length: {}", bytes.len()))
        }
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

pub fn generate_node_id() -> NodeId {
    let mut bytes = [0u8; 20];
    rand::rng().fill_bytes(&mut bytes);
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    NodeId(hasher.finalize().into())
}