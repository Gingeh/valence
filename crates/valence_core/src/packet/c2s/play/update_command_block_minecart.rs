use crate::packet::var_int::VarInt;
use crate::packet::{Decode, Encode};

#[derive(Copy, Clone, Debug, Encode, Decode)]
pub struct UpdateCommandBlockMinecartC2s<'a> {
    pub entity_id: VarInt,
    pub command: &'a str,
    pub track_output: bool,
}
