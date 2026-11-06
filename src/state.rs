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
use ufmt::uwriteln;

use super::{ds18b20::TempSensor, ow_error_str};

/// Low temperature threshold: 41 F = 5 C = 80 LSBs
const TEMP_THRESH_LO: u16 = 80;

/// High temperature threshold: 43 F = 6 1/9 C = 97 7/9 LSBs
const TEMP_THRESH_HI: u16 = 98;

/// Minimum on-time for the compressor
///
/// The thermometer ADC takes about 750ms to convert.
/// 160 cycles is 120 seconds.
const MIN_ON_TIME: u16 = 160;

/// Minimum off-time for the compressor
///
/// The thermometer ADC takes about 750ms to convert.
/// 320 cycles is 240 seconds.
const MIN_OFF_TIME: u16 = 320;

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
    temperature: Option<u16>,
    converting: bool,
    cooling: bool,
    debounce_count: u16,
    temp_err_count: u8,
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
    ) -> Self {
        Self {
            temperature: None,
            converting: false,
            cooling: false,
            debounce_count: 0,
            temp_err_count: 0,
            led,
            relay,
            serial,
            thermometer,
        }
    }

    /// Record a temperature error
    pub fn increment_temp_err(&mut self) {
        self.temp_err_count = self.temp_err_count.saturating_add(1);
    }

    /// Clear the temperature error count
    pub fn clear_temp_err(&mut self) {
        self.temp_err_count = 0;
    }

    /// Check whether we are in the temperature error state
    pub fn is_temp_err(&self) -> bool {
        self.temp_err_count >= TEMP_ERR_THRESH
    }

    /// Increment the debounce counter
    pub fn increment_debounce_cnt(&mut self) {
        self.debounce_count = self.debounce_count.saturating_add(1);
    }

    /// Clear the debounce counter
    pub fn clear_debounce_cnt(&mut self) {
        self.debounce_count = 0;
    }

    /// Check whether we're debounced
    pub fn is_debounced(&self) -> bool {
        if self.cooling {
            self.debounce_count > MIN_ON_TIME
        } else {
            self.debounce_count > MIN_OFF_TIME
        }
    }

    /// Update temperature reading based on output of thermometer
    pub fn update_temperature(&mut self) {
        if self.converting {
            while !self.thermometer.conversion_complete().unwrap_or_default() {
                self.thermometer.delay.delay_ms(1u16);
            }

            match self.thermometer.read_temperature() {
                Err(e) => {
                    self.increment_temp_err();
                    self.temperature = None;
                    uwriteln!(self.serial, "ERR THERM(CONV) {}", ow_error_str(e))
                        .unwrap_infallible();
                }
                Ok(t) => {
                    self.temperature = Some(t);
                }
            }
        }
    }

    /// Start a new temperature conversion
    pub fn start_conversion(&mut self) {
        if let Err(e) = self.thermometer.start_conversion() {
            self.increment_temp_err();
            self.converting = false;
            uwriteln!(self.serial, "ERR THERM(CONV) {}", ow_error_str(e)).unwrap_infallible();
        } else {
            self.converting = true;
        }
    }

    /// Take the temperature value from the state
    pub fn take_temperature(&mut self) -> Option<u16> {
        self.temperature.take()
    }

    /// Write the temperature to the serial port
    pub fn write_temperature(&mut self, t: u16) {
        uwriteln!(self.serial, "THERM {:x}", t).unwrap_infallible();
    }

    /// Get the `cooling` bit.
    pub fn cooling(&self) -> bool {
        self.cooling
    }

    /// Enable the thermostat
    ///
    /// Returns true if the thermostat was actually enabled
    pub fn thermostat_on(&mut self) -> bool {
        if self.is_debounced() {
            self.cooling = true;
            self.led.set_high();
            self.relay.set_low();
            return true;
        }

        false
    }

    /// Disable the thermostat
    ///
    /// Returns true if the thermostat was actually disabled
    pub fn thermostat_off(&mut self) -> bool {
        if self.is_debounced() {
            unsafe {
                self.thermostat_off_unchecked();
            }
            return true;
        }

        false
    }

    /// Disable the thermostat without checking the debouncer
    ///
    /// # Safety
    ///
    /// This function should not be called except after checking
    /// the debouncer or in case of an error state.
    pub unsafe fn thermostat_off_unchecked(&mut self) {
        self.cooling = false;
        self.led.set_low();
        self.relay.set_high();
    }

    /// Check whether the given temperature is below the low threshold
    pub fn should_stop_cooling(&self, t: u16) -> bool {
        t < TEMP_THRESH_LO
    }

    /// Check whether the given temperature is above the hi threshold
    pub fn should_start_cooling(&self, t: u16) -> bool {
        t > TEMP_THRESH_HI
    }
}
