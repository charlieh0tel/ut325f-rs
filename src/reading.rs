use anyhow::{anyhow, Result};
use std::mem;
use std::time::SystemTime;

use crate::utils::system_time_to_unix_seconds;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum HoldType {
    Current = 0,
    Maximum = 1,
    Minimum = 2,
    Average = 3,
}

impl TryFrom<u8> for HoldType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Current),
            1 => Ok(Self::Maximum),
            2 => Ok(Self::Minimum),
            3 => Ok(Self::Average),
            _ => Err(()),
        }
    }
}

#[derive(Debug)]
pub struct Reading {
    pub timestamp: SystemTime,
    pub current_temps_c: [f32; 4],
    pub held_temps_c: [f32; 4],
    pub hold_type: HoldType,
    pub meter_temp_c: f32,
}

impl Reading {
    pub const N_BYTES: usize = 56;
    pub const SYNC: [u8; 5] = [0xaa, 0x55, 0x00, 0x34, 0x01];
    pub const N_SYNC_BYTES: usize = Self::SYNC.len();

    fn unpack_f32(buf: &[u8], offset: &mut usize) -> Result<f32> {
        let size = mem::size_of::<f32>();
        if *offset + size > buf.len() {
            return Err(anyhow!("Read beyond buffer"));
        }
        let bytes = &buf[*offset..*offset + size];
        let value = f32::from_le_bytes(bytes.try_into().unwrap());
        *offset += size;
        Ok(value)
    }

    fn unpack_u8(buf: &[u8], offset: &mut usize) -> Result<u8> {
        let size = mem::size_of::<u8>();
        if *offset + size > buf.len() {
            return Err(anyhow!("Read beyond buffer"));
        }
        let value = buf[*offset];
        *offset += size;
        Ok(value)
    }

    fn unpack_u16(buf: &[u8], offset: &mut usize) -> Result<u16> {
        let size = mem::size_of::<u16>();
        if *offset + size > buf.len() {
            return Err(anyhow!("Read beyond buffer"));
        }
        let bytes = &buf[*offset..*offset + size];
        let value = u16::from_le_bytes(bytes.try_into().unwrap());
        *offset += size;
        Ok(value)
    }

    fn unpack_u32(buf: &[u8], offset: &mut usize) -> Result<u32> {
        let size = mem::size_of::<u32>();
        if *offset + size > buf.len() {
            return Err(anyhow!("Read beyond buffer"));
        }
        let bytes = &buf[*offset..*offset + size];
        let value = u32::from_le_bytes(bytes.try_into().unwrap());
        *offset += size;
        Ok(value)
    }

    pub fn parse(buf: &[u8; Self::N_BYTES]) -> Result<Self> {
        if buf.len() != Self::N_BYTES {
            return Err(anyhow!("Incorrect buffer size"));
        }
        if buf[..Self::N_SYNC_BYTES] != Self::SYNC {
            return Err(anyhow!("Bad sync header"));
        }

        let mut offset = Self::N_SYNC_BYTES;
        let timestamp = SystemTime::now();
        let mut current_temps_c = [0.0; 4];
        for temp in current_temps_c.iter_mut() {
            *temp = Self::unpack_f32(buf, &mut offset)?;
        }
        for temp in current_temps_c.iter_mut() {
            let error = Self::unpack_u8(buf, &mut offset)?;
            if error != 0 {
                *temp = f32::NAN;
            }
        }
        let mut held_temps_c = [0.0; 4];
        for temp in held_temps_c.iter_mut() {
            *temp = Self::unpack_f32(buf, &mut offset)?;
        }
        for temp in held_temps_c.iter_mut() {
            let error = Self::unpack_u8(buf, &mut offset)?;
            if error != 0 {
                *temp = f32::NAN;
            }
        }
        let meter_temp_c = Self::unpack_f32(buf, &mut offset)?;
        Self::unpack_u32(buf, &mut offset)?; // unknown
        let hold_type_raw = Self::unpack_u8(buf, &mut offset)?;
        let hold_type =
            HoldType::try_from(hold_type_raw).map_err(|_| anyhow!("Invalid HoldType"))?;
        Self::unpack_u16(buf, &mut offset)?; // checksum??

        if offset == Self::N_BYTES {
            Ok(Self {
                timestamp,
                current_temps_c,
                held_temps_c,
                hold_type,
                meter_temp_c,
            })
        } else {
            Err(anyhow!("Failed to parse all bytes"))
        }
    }

    pub fn print_current_temps(&self) {
        print!(
            "{:.3}",
            system_time_to_unix_seconds(self.timestamp).unwrap()
        );
        for temp in self.current_temps_c.iter() {
            print!(" {:7.3}", temp);
        }
        println!();
    }

    pub fn print_all(&self) {
        print!(
            "{:.3}",
            system_time_to_unix_seconds(self.timestamp).unwrap()
        );
        for temp in &self.current_temps_c {
            print!(" {:7.3}", temp);
        }
        print!(" {:?}", self.hold_type);
        for temp in &self.held_temps_c {
            print!(" {:7.3}", temp);
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reading_from_bytes() -> Result<()> {
        #[rustfmt::skip]
        let test_bytes: [u8; Reading::N_BYTES] = [
            0xaa, 0x55, 0x00, 0x34, 0x01, 0x98, 0x94, 0xd5,
            0x41, 0x00, 0x00, 0x00, 0x00, 0x2d, 0x02, 0xd5,
            0x41, 0x6c, 0x25, 0x85, 0x42, 0x00, 0x30, 0x30,
            0x30, 0x98, 0x94, 0xd5, 0x41, 0x00, 0x00, 0x00,
            0x00, 0x2d, 0x02, 0xd5, 0x41, 0x6c, 0x25, 0x85,
            0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xd2,
            0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0d, 0x15,
        ];

        let reading_result = Reading::parse(&test_bytes)?;

        assert_eq!(reading_result.current_temps_c[0], 26.697556);
        assert!(reading_result.current_temps_c[1].is_nan());
        assert!(reading_result.current_temps_c[2].is_nan());
        assert!(reading_result.current_temps_c[3].is_nan());

        assert_eq!(reading_result.held_temps_c[0], 26.697556);
        assert_eq!(reading_result.held_temps_c[1], 0.0);
        assert_eq!(reading_result.held_temps_c[2], 26.626062);
        assert_eq!(reading_result.held_temps_c[3], 66.57309);

        assert_eq!(reading_result.meter_temp_c, 26.3125);
        assert_eq!(reading_result.hold_type, HoldType::Current);

        Ok(())
    }

    #[test]
    fn test_parse_bad_sync() -> Result<()> {
        let mut buffer = [0u8; Reading::N_BYTES];
        buffer[0] = 0x00; // Corrupt the sync header
        let reading_result = Reading::parse(&buffer);
        assert!(reading_result.is_err());
        assert_eq!(reading_result.unwrap_err().to_string(), "Bad sync header");
        Ok(())
    }

    #[test]
    fn test_parse_invalid_hold_type() -> Result<()> {
        let mut buffer = [0u8; Reading::N_BYTES];
        buffer[..Reading::N_SYNC_BYTES].copy_from_slice(&Reading::SYNC);
        buffer[Reading::N_BYTES - 3] = 0xff; // Invalid HoldType value
        let reading_result = Reading::parse(&buffer);
        assert!(reading_result.is_err());
        assert_eq!(reading_result.unwrap_err().to_string(), "Invalid HoldType");
        Ok(())
    }
}
