//! Thermostat state

use arduino_hal::{
    hal::{
        port::{Pin, PinOps},
        usart::UsartOps,
        Atmega,
    },
    port::mode::{OpenDrain, Output},
    prelude::_unwrap_infallible_UnwrapInfallible,
    Delay, Usart,
};
use avr_hal_generic::hal_v0::blocking::delay::DelayMs;

use super::{ds18b20::TempSensor, ow_error_str};

/// A macro to write a line to the serial port and increment the message counter
macro_rules! uart_writeln {
    ($self:expr, $fmt:literal, $( $arg:expr ),*) => {{
        ufmt::uwrite!($self.serial, "{:x} ", $self.message_count).unwrap_infallible();
        ufmt::uwriteln!($self.serial, $fmt, $($arg),*).unwrap_infallible();
        $self.message_count = $self.message_count.wrapping_add(1);
    }}
}

/// Low temperature threshold: 41 F = 5 C = 80 LSBs
const TEMP_THRESH_LO: i16 = 80;

/// High temperature threshold: 43 F = 6 1/9 C = 97 7/9 LSBs
const TEMP_THRESH_HI: i16 = 98;

/// Our sensor has about a 3 degree F offset as measured by
/// a calibrated thermometer.
const TEMP_OFFSET: i16 = -49;

/// Minimum on-time for the compressor is 2 minutes.
/// Each cycle is at least 1 second, enforced by timer0.
const MIN_ON_TIME: u8 = 120;

/// Minimum off-time for the compressor is 4 minutes.
/// Each cycle is at least 1 second, enforced by timer0.
const MIN_OFF_TIME: u8 = 240;

/// Number of temperature errors before we change state
const TEMP_ERR_THRESH: u8 = 8;

/// State of the thermostat
pub struct ThermostatState<'a, LED, RELAY, OWPIN, USART, RXPIN, TXPIN>
where
    LED: PinOps,
    RELAY: PinOps,
    OWPIN: PinOps,
    USART: UsartOps<Atmega, RXPIN, TXPIN>,
{
    temperature: Option<i16>,
    converting: bool,
    cooling: bool,
    dwell_count: u8,
    temp_err_count: u8,
    message_count: u32,
    led: &'a mut Pin<Output, LED>,
    relay: &'a mut Pin<OpenDrain, RELAY>,
    serial: &'a mut Usart<USART, RXPIN, TXPIN>,
    thermometer: &'a mut TempSensor<'a, OWPIN, Delay>,
}

impl<'a, LED, RELAY, OWPIN, USART, RXPIN, TXPIN>
    ThermostatState<'a, LED, RELAY, OWPIN, USART, RXPIN, TXPIN>
where
    LED: PinOps,
    RELAY: PinOps,
    OWPIN: PinOps,
    USART: UsartOps<Atmega, RXPIN, TXPIN>,
{
    pub fn new(
        led: &'a mut Pin<Output, LED>,
        relay: &'a mut Pin<OpenDrain, RELAY>,
        serial: &'a mut Usart<USART, RXPIN, TXPIN>,
        thermometer: &'a mut TempSensor<'a, OWPIN, Delay>,
        message_count: u32,
    ) -> Self {
        Self {
            temperature: None,
            converting: false,
            cooling: false,
            dwell_count: 0,
            temp_err_count: 0,
            message_count,
            led,
            relay,
            serial,
            thermometer,
        }
    }

    /// Record a temperature error
    fn increment_temp_err(&mut self) {
        self.temp_err_count = self.temp_err_count.saturating_add(1);
    }

    /// Clear the temperature error count
    fn clear_temp_err(&mut self) {
        self.temp_err_count = 0;
    }

    /// Check whether we are in the temperature error state
    fn is_temp_err(&self) -> bool {
        self.temp_err_count >= TEMP_ERR_THRESH
    }

    /// Increment the dwell counter
    pub fn increment_dwell_cnt(&mut self) {
        self.dwell_count = self.dwell_count.saturating_add(1);
    }

    /// Clear the dwell counter
    fn clear_dwell_cnt(&mut self) {
        self.dwell_count = 0;
    }

    /// Check whether we've dwelled long enough
    fn can_change_state(&self) -> bool {
        if self.cooling {
            self.dwell_count > MIN_ON_TIME
        } else {
            self.dwell_count > MIN_OFF_TIME
        }
    }

    /// Update temperature reading based on output of thermometer
    pub fn update_temperature(&mut self) {
        if self.converting {
            // if we keep getting errors here, we'll loop until the watchdog resets us
            loop {
                match self.thermometer.command_complete() {
                    Ok(complete) if complete => break,
                    Ok(_) => (),
                    Err(e) => uart_writeln!(self, "ERR THERM(WAIT) {}", ow_error_str(e)),
                };

                self.thermometer.delay.delay_ms(1u16);
            }

            match self.thermometer.read_temperature() {
                Err(e) => {
                    self.increment_temp_err();
                    self.temperature = None;
                    uart_writeln!(self, "ERR THERM(READ) {}", ow_error_str(e));
                }
                Ok(t) => {
                    // correct the temperature by applying the offset
                    self.temperature = Some(t - TEMP_OFFSET);
                }
            }
        }
    }

    /// Start a new temperature conversion
    pub fn start_conversion(&mut self) {
        if let Err(e) = self.thermometer.start_conversion() {
            self.increment_temp_err();
            self.converting = false;
            uart_writeln!(self, "ERR THERM(CONV) {}", ow_error_str(e));
        } else {
            self.converting = true;
        }
    }

    /// Update the cooling state based on the measured temperature, if available
    pub fn update_cooling_state(&mut self) {
        if let Some(t) = self.temperature.take() {
            // got a good temperature reading; clear temp sensor error count
            self.clear_temp_err();

            // check the hysteresis band
            let stop_commanded = self.should_stop_cooling(t);
            let cool_commanded = self.should_start_cooling(t);

            // decide whether to change cooling state
            #[allow(clippy::collapsible_if)]
            if self.cooling && stop_commanded {
                if self.thermostat_off() {
                    self.clear_dwell_cnt();
                }
            } else if !self.cooling && cool_commanded {
                if self.thermostat_on() {
                    self.clear_dwell_cnt();
                }
            }

            uart_writeln!(
                self,
                "OK temp={:x} dwell={:x} cool={} scmd={} ccmd={}",
                t,
                self.dwell_count,
                self.cooling as u8,
                stop_commanded as u8,
                cool_commanded as u8
            );
        } else if self.is_temp_err() {
            // if we're in temperature sensor error state, turn off thermostat,
            // bypassing the dwellr because this is a hard error...
            self.thermostat_off_unchecked();

            // ...then write status line to the serial port...
            uart_writeln!(self, "ERR THERM(ERRS) {}", self.temp_err_count);

            // ...then go into a busyloop until wdt resets us
            loop {
                self.thermometer.delay.delay_ms(1000u16);
            }
        }
    }

    /// Enable the thermostat if the dwellr allows it
    ///
    /// Returns true if the thermostat was actually enabled
    fn thermostat_on(&mut self) -> bool {
        if self.can_change_state() {
            self.cooling = true;
            self.led.set_high();
            self.relay.set_low();
            return true;
        }

        false
    }

    /// Disable the thermostat if the dwellr allows it
    ///
    /// Returns true if the thermostat was actually disabled
    fn thermostat_off(&mut self) -> bool {
        if self.can_change_state() {
            self.thermostat_off_unchecked();
            return true;
        }

        false
    }

    /// Disable the thermostat, bypassing the dwellr
    ///
    /// This function should not be called except after checking
    /// the dwell counter or in case of an error state.
    fn thermostat_off_unchecked(&mut self) {
        self.cooling = false;
        self.led.set_low();
        self.relay.set_high();
    }

    /// Check whether the given temperature is below the low threshold
    fn should_stop_cooling(&self, t: i16) -> bool {
        t <= TEMP_THRESH_LO
    }

    /// Check whether the given temperature is above the hi threshold
    fn should_start_cooling(&self, t: i16) -> bool {
        t >= TEMP_THRESH_HI
    }
}
