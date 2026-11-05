//! Communicate with a DS18B20 temperature sensor via a one-wire bus.
//!
//! NOTE that this interface currently does not support parasite power mode.

use arduino_hal::port::{mode::OpenDrain, Pin, PinOps};
use avr_hal_generic::hal_v0::blocking::delay::{DelayMs, DelayUs};
use one_wire_bus::{crc::check_crc8, Address, OneWire, OneWireError};

#[repr(u8)]
#[derive(Default, Copy, Clone)]
pub enum TempSensorResolution {
    #[default]
    Bits9 = 0x1f,
    Bits10 = 0x3f,
    Bits11 = 0x5f,
    Bits12 = 0x7f,
}

impl TempSensorResolution {
    /// Delay for temperature reading given conversion resolution.
    /// These numbers come from the DS18B20 datasheet.
    fn delay_ms(&self) -> u16 {
        match self {
            Self::Bits9 => 94,
            Self::Bits10 => 188,
            Self::Bits11 => 375,
            Self::Bits12 => 750,
        }
    }
}

type OwError = OneWireError<core::convert::Infallible>;

// Commands are documented in the DS18B20 datasheet
const WRITE_SCRATCHPAD: u8 = 0x4e;
const READ_SCRATCHPAD: u8 = 0xbe;
const CONVERT_T: u8 = 0x44;

pub struct TempSensor<'a, PIN: PinOps, D: DelayUs<u16> + DelayMs<u16>> {
    ow_bus: &'a mut OneWire<Pin<OpenDrain, PIN>>,
    resolution: TempSensorResolution,
    addr: Option<&'a Address>,
    delay: &'a mut D,
}

impl<'a, PIN: PinOps, D: DelayMs<u16> + DelayUs<u16>> TempSensor<'a, PIN, D> {
    /// Construct a new TempSensor and configure the temperature sensor
    /// with the specified resolution.
    pub fn new(
        ow_bus: &'a mut OneWire<Pin<OpenDrain, PIN>>,
        resolution: TempSensorResolution,
        addr: Option<&'a Address>,
        delay: &'a mut D,
    ) -> Result<Self, OwError> {
        // configure the temperature sensor with the specified resolution
        let config = [0u8, 0u8, resolution as u8];
        ow_bus.send_command(WRITE_SCRATCHPAD, addr, delay)?;
        ow_bus.write_bytes(&config[..], delay)?;

        // read back the configuration and make sure it is correct
        let mut scratch = [0u8; 9];
        ow_bus.send_command(READ_SCRATCHPAD, addr, delay)?;
        ow_bus.read_bytes(&mut scratch, delay)?;
        check_crc8(&scratch[..])?;
        if scratch[2] != config[0] || scratch[3] != config[1] || scratch[4] != config[2] {
            return Err(OneWireError::UnexpectedResponse);
        }

        Ok(Self {
            ow_bus,
            resolution,
            addr,
            delay,
        })
    }

    /// Command a conversion and wait for the result. This function waits
    /// the amount of time specified in the datasheet
    pub fn convert_blocking(&mut self) -> Result<u16, OwError> {
        self.start_conversion()?;

        // NOTE could do this with DelayUs and avoid the DelayMs bound
        self.delay.delay_ms(self.resolution.delay_ms());

        self.read_temperature()
    }

    /// Command a conversion and wait for the result, polling the sensor for completion
    pub fn convert_polling(&mut self) -> Result<u16, OwError> {
        self.start_conversion()?;

        loop {
            // wait 1ms
            self.delay.delay_us(1000);

            // poll for completion
            if self.conversion_complete()? {
                break;
            }
        }

        self.read_temperature()
    }

    /// Initiate a conversion and do not wait for the result
    pub fn start_conversion(&mut self) -> Result<(), OwError> {
        self.ow_bus.send_command(CONVERT_T, self.addr, self.delay)
    }

    /// Check whether the temperature conversion is ready. This function can
    /// only be used after `start_conversion(..)` was issued. Once it returns
    /// `true`, the result can be retrieved with `read_temperature(..)`.
    pub fn conversion_complete(&mut self) -> Result<bool, OwError> {
        self.ow_bus.read_bit(self.delay)
    }

    /// Read the temperature data from the scratchpad
    pub fn read_temperature(&mut self) -> Result<u16, OwError> {
        // read the scratchpad data
        let mut scratch = [0u8; 9];
        self.ow_bus
            .send_command(READ_SCRATCHPAD, self.addr, self.delay)?;
        self.ow_bus.read_bytes(&mut scratch, self.delay)?;
        check_crc8(&scratch[..])?;

        // convert the temperature reading to a u16 and return it
        Ok(u16::from_le_bytes(
            scratch[0..2].try_into().unwrap_or_default(),
        ))
    }
}
