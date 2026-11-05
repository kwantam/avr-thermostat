#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::{
    hal::wdt,
    prelude::{_embedded_hal_blocking_delay_DelayMs, _unwrap_infallible_UnwrapInfallible},
    Delay,
};
use core::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use one_wire_bus::{OneWire, OneWireError};
use panic_halt as _;
use ufmt::{uwrite, uwriteln};

pub mod ds18b20;
pub mod reset;

static ADC_VALUE: AtomicU16 = AtomicU16::new(0);
static ADC_COUNT: AtomicU8 = AtomicU8::new(0);

#[avr_device::interrupt(atmega328p)]
fn ADC() {
    // SAFETY: we are only using this to read the ADC value and clear the timer interrupt flag
    let dp = unsafe { arduino_hal::Peripherals::steal() };
    let val = dp.ADC.adc().read().bits();
    ADC_VALUE.store(val, Ordering::SeqCst);
    ADC_COUNT.store(
        ADC_COUNT.load(Ordering::SeqCst).wrapping_add(1),
        Ordering::SeqCst,
    );

    // clear the ocf1b flag (NOTE: write 1 to clear flag!)
    // we have to do this because the ADC requires a rising edge to sample
    dp.TC1.tifr1().write(|w| w.ocf1b().bit(true));
}

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    // check reset reason
    let mcusr = dp.CPU.mcusr().read().bits();
    dp.CPU.mcusr().reset();

    // set up serial port and send reset reason
    let mut serial = arduino_hal::default_serial!(dp, pins, 38400);
    match reset::ResetReason::try_from(mcusr) {
        Err(e) => uwriteln!(&mut serial, "RESET unknown({:x})", e),
        Ok(r) => uwriteln!(&mut serial, "RESET {}", <&'static str>::from(r)),
    }.unwrap_infallible();

    //  digital output pins
    let mut led = pins.d13.into_output();
    let mut relay = pins.d8.into_opendrain_high();

    // set up the one-wire thermometer
    let ow_pin = pins.d2.into_opendrain_high();
    let mut ow_bus = OneWire::new(ow_pin).unwrap();
    let mut delay = Delay::new();
    delay.delay_ms(10u16); // allow one-wire bus to settle
    let mut thermometer = match ds18b20::TempSensor::new(
        &mut ow_bus,
        ds18b20::TempSensorResolution::Bits12,
        None,
        &mut delay,
    ) {
        Err(e) => {
            uwriteln!(&mut serial, "THERM: ERR(INIT): {}", ow_error_str(e)).unwrap_infallible();
            panic!();
        }
        Ok(t) => t,
    };

    // set up the watchdog timer
    let mut watchdog = wdt::Wdt::new(dp.WDT, dp.CPU.mcusr());
    watchdog.start(wdt::Timeout::Ms2000).unwrap_or_default();

    // ADC and timer setup for auto conversion of adc0 at 1.2 kHz, interrupting when finished
    adc_timer_setup(&dp.ADC, &dp.TC1);
    unsafe { avr_device::interrupt::enable() };

    let mut count = 0u8;
    let mut delay = Delay::new();
    loop {
        watchdog.feed();

        led.toggle();
        uwrite!(&mut serial, "hello ").unwrap_infallible();
        delay.delay_ms(100u16);

        led.toggle();
        uwrite!(&mut serial, "world\n").unwrap_infallible();
        delay.delay_ms(100u16);

        // flip the relay every 5 seconds
        if count == 50u8 {
            // thermometer
            if let Err(e) = thermometer.start_conversion() {
                uwriteln!(&mut serial, "THERM: ERR(CONV): {}", ow_error_str(e)).unwrap_infallible();
            }

            // read the ADC value
            let adc_value = ADC_VALUE.load(Ordering::SeqCst);
            let adc_count = ADC_COUNT.load(Ordering::SeqCst);
            uwriteln!(&mut serial, "ADC: {:x} {}", adc_value, adc_count).unwrap_infallible();

            // wait for thermometer to be ready
            while !thermometer.conversion_complete().unwrap_or_default() {
                delay.delay_ms(1u16);
            }
            match thermometer.read_temperature() {
                Err(e) => uwriteln!(&mut serial, "THERM: ERR(READ): {}", ow_error_str(e)),
                Ok(t) => uwriteln!(&mut serial, "THERM: {:x}", t),
            }
            .unwrap_infallible();

            // relay
            count = 0;
            relay = if relay.is_low() {
                relay.into_opendrain_high()
            } else {
                relay.into_opendrain()
            };
        }
        count = count.wrapping_add(1u8);
    }
}

fn ow_error_str<E>(e: OneWireError<E>) -> &'static str {
    match e {
        OneWireError::BusNotHigh => "OWE BusNotHigh",
        OneWireError::PinError(_) => "OWE PinError",
        OneWireError::UnexpectedResponse => "OWE UnexpectedResponse",
        OneWireError::FamilyCodeMismatch => "OWE FamilyCodeMismatch",
        OneWireError::CrcMismatch => "OWE CrcMismatch",
        OneWireError::Timeout => "OWE Timeout",
    }
}

fn adc_timer_setup(adc: &avr_device::atmega328p::ADC, tc1: &avr_device::atmega328p::TC1) {
    // set up ADC for automatic conversion of the adc0 input at 1.2 kHz
    // - vref = 1.1v
    // - mux = adc8 (temperature sensor)
    // - right aligned output
    adc.admux().write(|w| unsafe { w.bits(0xc8) });
    // - adc auto trigger source = timer1 compare match b
    adc.adcsrb().write(|w| unsafe { w.bits(0x05) });
    // - disable digital input buffer in a0
    adc.didr0().write(|w| unsafe { w.bits(0x01) });
    // prescaler = 128 (16000000 / 128 = 125000 Hz ADC clock)
    // adc interrupt enabled
    // adc auto trigger enabled
    // adc enabled
    adc.adcsra().write(|w| unsafe { w.bits(0xaf) });

    // set up timer1
    // ocr1a = 13332; 16 MHz / 13332 = 1.2 kHz
    tc1.ocr1a().write(|w| w.set(13332u16));
    // ocr1b = 6666; compare will fire when timer crosses this value
    tc1.ocr1b().write(|w| w.set(6666u16));
    // clear tccr1a
    tc1.tccr1a().reset();
    // set up CTC mode (auto clear counter when it reaches ocr1a value)
    // unscaled clock as input to the timer (timer is now enabled)
    tc1.tccr1b().write(|w| unsafe { w.bits(0x09) });
    // clear the interrupt flag (NOTE: write 1 to clear flag!)
    tc1.tifr1().write(|w| w.ocf1b().bit(true));
}
