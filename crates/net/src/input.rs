//! Per-player inputs on the wire.
//!
//! GGRS wants `Copy + PartialEq + Default + Serialize + DeserializeOwned`
//! inputs. The sim crate supplies a plain `bytemuck::Pod` struct instead;
//! [`Wire`] adapts it: the bytes of the struct are the wire format (a fixed
//! number of bytes, no length prefix), `Default` is all zeroes and equality is
//! byte equality. All-zero therefore means "no input", which is also what
//! GGRS hands out for a disconnected player.

use bytemuck::{Pod, Zeroable};
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::marker::PhantomData;

/// The inputs of one player for one frame, as the network sees them.
///
/// Implemented for every `Pod` type. Keep it small (GGRS sends the last few
/// frames of every local player in each packet) and state-based (buttons held,
/// not pressed): the default predictor repeats the last input.
pub trait NetInput: Pod + Send + 'static {}
impl<T: Pod + Send + 'static> NetInput for T {}

/// Placeholder input until `rounds-sim` defines its own: 6 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PlayerInput {
    /// Movement stick, -127..=127 (quantise analog sticks: fewer mispredictions).
    pub move_x: i8,
    pub move_y: i8,
    /// Aim direction, 0..65535 = 0..360 degrees.
    pub aim: u16,
    /// Held buttons, see the `BTN_*` constants.
    pub buttons: u16,
}

impl PlayerInput {
    pub const BTN_JUMP: u16 = 1 << 0;
    pub const BTN_FIRE: u16 = 1 << 1;
    pub const BTN_BLOCK: u16 = 1 << 2;
    pub const BTN_RELOAD: u16 = 1 << 3;
    pub const BTN_MENU_OK: u16 = 1 << 4;
    pub const BTN_MENU_BACK: u16 = 1 << 5;
}

/// GGRS input wrapper around a `Pod` input.
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Wire<I: NetInput>(pub I);

impl<I: NetInput> Default for Wire<I> {
    fn default() -> Self {
        Wire(I::zeroed())
    }
}

impl<I: NetInput> PartialEq for Wire<I> {
    fn eq(&self, other: &Self) -> bool {
        bytemuck::bytes_of(&self.0) == bytemuck::bytes_of(&other.0)
    }
}

impl<I: NetInput> fmt::Debug for Wire<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wire(")?;
        for b in bytemuck::bytes_of(&self.0) {
            write!(f, "{b:02x}")?;
        }
        write!(f, ")")
    }
}

impl<I: NetInput> Serialize for Wire<I> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let bytes = bytemuck::bytes_of(&self.0);
        let mut t = s.serialize_tuple(bytes.len())?;
        for b in bytes {
            t.serialize_element(b)?;
        }
        t.end()
    }
}

impl<'de, I: NetInput> Deserialize<'de> for Wire<I> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<I>(PhantomData<I>);
        impl<'de, I: NetInput> Visitor<'de> for V<I> {
            type Value = Wire<I>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{} input bytes", std::mem::size_of::<I>())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Wire<I>, A::Error> {
                let mut v = I::zeroed();
                let bytes = bytemuck::bytes_of_mut(&mut v);
                for (i, b) in bytes.iter_mut().enumerate() {
                    *b = seq
                        .next_element()?
                        .ok_or_else(|| de::Error::invalid_length(i, &self))?;
                }
                Ok(Wire(v))
            }
        }
        d.deserialize_tuple(std::mem::size_of::<I>(), V(PhantomData))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_size_roundtrip() {
        let i = Wire(PlayerInput {
            move_x: -5,
            move_y: 127,
            aim: 0xBEEF,
            buttons: 3,
        });
        let b = bincode::serialize(&i).unwrap();
        assert_eq!(b.len(), 6, "no length prefix");
        let back: Wire<PlayerInput> = bincode::deserialize(&b).unwrap();
        assert_eq!(back, i);
        assert_eq!(
            bincode::serialized_size(&Wire::<PlayerInput>::default()).unwrap(),
            6
        );
        assert!(bincode::deserialize::<Wire<PlayerInput>>(&b[..5]).is_err());
    }
}
