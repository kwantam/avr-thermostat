//! Communicate with a DS18B20 temperature sensor via a one-wire bus.
//!
//! NOTE that this interface currently does not support parasite power mode.

use arduino_hal::port::{mode::OpenDrain, Pin, PinOps};
use avr_hal_generic::hal_v0::blocking::delay::{DelayMs, DelayUs};
use one_wire_bus::{crc::check_crc8, Address, OneWire, OneWireError};

use super::OwError;

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

// Commands are documented in the DS18B20 datasheet
const RECALL_EEPROM: u8 = 0xb8;
const WRITE_SCRATCHPAD: u8 = 0x4e;
const READ_SCRATCHPAD: u8 = 0xbe;
const CONVERT_T: u8 = 0x44;

pub struct TempSensor<'a, PIN: PinOps, D: DelayUs<u16> + DelayMs<u16>> {
    ow_bus: &'a mut OneWire<Pin<OpenDrain, PIN>>,
    resolution: TempSensorResolution,
    config: [u8; 3],
    addr: Option<&'a Address>,
    pub(crate) delay: &'a mut D,
}

macro_rules! temp_sensor {
    ($ow_bus:expr, $resolution:expr, $addr:expr, $delay:expr, $serial:expr, $msg_count:expr) => {
        loop {
            // let the one-wire bus settle
            $delay.delay_ms(10u16);

            // attempt to configure the temperature sensor
            match ds18b20::TempSensor::new(&mut $ow_bus, $resolution, $addr, &mut $delay) {
                Err(e) => ufmt::uwriteln!(
                    &mut $serial,
                    "{} ERR THERM(INIT) {}",
                    *$msg_count,
                    ow_error_str(e)
                )
                .unwrap_infallible(),
                Ok(t) => break t,
            }

            *$msg_count = $msg_count.wrapping_add(1);
        }
    };
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
        let mut sensor = Self {
            ow_bus,
            resolution,
            config: [0u8; 3],
            addr,
            delay,
        };
        sensor.initialize()?;
        Ok(sensor)
    }

    fn initialize(&mut self) -> Result<(), OwError> {
        let mut scratch = [0u8; 9];

        // tell the temperature sensor to read values out of the EEPROM
        self.ow_bus
            .send_command(RECALL_EEPROM, self.addr, self.delay)?;
        // wait until the recall_eeprom command is done
        loop {
            self.delay.delay_us(1000);
            if self.command_complete()? {
                break;
            }
        }
        // now read the power-on values
        self.ow_bus
            .send_command(READ_SCRATCHPAD, self.addr, self.delay)?;
        self.ow_bus.read_bytes(&mut scratch, self.delay)?;
        check_crc8(&scratch[..])?;

        // configure the temperature sensor.
        // Resolution is set to the value specified at construction.
        // Alarm registers are set to arbitrary values that are different
        // from the ones that are read from the EEPROM. This lets us detect
        // a brownout: if the scratch register comes back with the wrong
        // configuration bytes, we know that something has gone wrong.
        self.config = [scratch[2] ^ 0xff, scratch[3] ^ 0xff, self.resolution as u8];
        self.ow_bus
            .send_command(WRITE_SCRATCHPAD, self.addr, self.delay)?;
        self.ow_bus.write_bytes(&self.config[..], self.delay)?;

        // read back configuration and make sure it's correct
        self.ow_bus
            .send_command(READ_SCRATCHPAD, self.addr, self.delay)?;
        self.ow_bus.read_bytes(&mut scratch, self.delay)?;
        check_crc8(&scratch[..])?;
        self.check_config(&scratch)?;

        Ok(())
    }

    /// Check that the response received from the sensor has the expected config values.
    /// If it does not, that likely means that it experienced a brownout.
    fn check_config(&self, scratch: &[u8; 9]) -> Result<(), OwError> {
        if scratch[2] != self.config[0]
            || scratch[3] != self.config[1]
            || scratch[4] != self.config[2]
        {
            return Err(OneWireError::UnexpectedResponse);
        }
        Ok(())
    }

    /// Command a conversion and wait for the result. This function waits
    /// the amount of time specified in the datasheet
    pub fn convert_blocking(&mut self) -> Result<i16, OwError> {
        self.start_conversion()?;

        // NOTE could do this with DelayUs and avoid the DelayMs bound
        self.delay.delay_ms(self.resolution.delay_ms());

        self.read_temperature()
    }

    /// Command a conversion and wait for the result, polling the sensor for completion
    pub fn convert_polling(&mut self) -> Result<i16, OwError> {
        self.start_conversion()?;

        loop {
            // wait 1ms
            self.delay.delay_us(1000);

            // poll for completion
            if self.command_complete()? {
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
    pub fn command_complete(&mut self) -> Result<bool, OwError> {
        self.ow_bus.read_bit(self.delay)
    }

    /// Read the temperature data from the scratchpad
    pub fn read_temperature(&mut self) -> Result<i16, OwError> {
        // read the scratchpad data
        let mut scratch = [0u8; 9];
        self.ow_bus
            .send_command(READ_SCRATCHPAD, self.addr, self.delay)?;
        self.ow_bus.read_bytes(&mut scratch, self.delay)?;
        check_crc8(&scratch[..])?;

        // if the response contains an erroneous configuration, reinitialize and return an error
        if let Err(e) = self.check_config(&scratch) {
            self.initialize()?;
            return Err(e);
        }

        // convert the temperature reading to a u16 and return it
        Ok(i16::from_le_bytes([scratch[0], scratch[1]]))
    }
}
