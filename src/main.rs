#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]

use arduino_hal::{
    hal::wdt,
    prelude::{_embedded_hal_blocking_delay_DelayMs, _unwrap_infallible_UnwrapInfallible},
    Delay,
};
use core::sync::atomic::{AtomicU8, Ordering};
use one_wire_bus::{OneWire, OneWireError};
use panic_halt as _;
use ufmt::uwriteln;

#[macro_use]
pub mod ds18b20;
#[macro_use]
pub mod reset;
pub mod state;

type OwError = OneWireError<core::convert::Infallible>;

/// The seconds counter, which is written by the timer0_compa interrupt
/// and read by the main loop to sync to approximately 1 second
static SECONDS: AtomicU8 = AtomicU8::new(0);

/// Tick divider for timer0. Timer0 is configured to interrupt at 125 Hz,
/// so after 125 ticks we update the seconds counter.
const TIMER0_TICK_COUNT: u8 = 125;

#[avr_device::interrupt(atmega328p)]
fn TIMER0_COMPA() {
    static CYCLES: AtomicU8 = AtomicU8::new(0);

    // update the tick count
    let next = CYCLES.load(Ordering::SeqCst).saturating_add(1);
    if next < TIMER0_TICK_COUNT {
        CYCLES.store(next, Ordering::SeqCst);
        return;
    }

    // on rollover, update the seconds count
    CYCLES.store(0, Ordering::SeqCst);
    let seconds_next = SECONDS.load(Ordering::SeqCst).wrapping_add(1);
    SECONDS.store(seconds_next, Ordering::SeqCst);
}

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    // check reset reason
    let mcusr = dp.CPU.mcusr().read().bits();
    dp.CPU.mcusr().reset();

    // set up the watchdog timer
    let mut watchdog = wdt::Wdt::new(dp.WDT, dp.CPU.mcusr());
    // make sure that the watchdog timer is off. if we just had a watchdog reset, it will be
    // enabled with a 16ms timeout at this point, so we have to do this early in startup
    watchdog.stop();

    // set up serial port and send reset reason
    let mut serial = arduino_hal::default_serial!(dp, pins, 38400);
    write_reset_reason!(serial, mcusr);

    // status LED
    let mut led = pins.d13.into_output();
    // relay
    //
    // NOTE: we call `into_opendrain()` first to ensure that the output buffer
    // is in the correct state. We do this here because the `set_low()` function
    // on an open-drain output does not actually write to the output buffer, it just
    // enables the output. This shouldn't matter since the reset state of the buffer
    // is low, but this makes that state explicit. It could cause a sub-microsecond
    // pulse on the relay output, but a pulse that short has no effect on the relay.
    let mut relay = pins.d8.into_opendrain().into_opendrain_high();

    // one-wire bus
    let ow_pin = pins.d2.into_opendrain_high();
    let mut ow_bus = OneWire::new(ow_pin).unwrap();
    let mut delay = Delay::new();

    // initialize the temperature sensor
    let mut count = 1u32;
    let mut thermometer = temp_sensor!(
        ow_bus,
        ds18b20::TempSensorResolution::Bits12,
        None,
        delay,
        serial,
        &mut count
    );

    // construct the state machine controller
    let mut state =
        state::ThermostatState::new(&mut led, &mut relay, &mut serial, &mut thermometer, count);

    // now start the watchdog timer
    watchdog.start(wdt::Timeout::Ms4000).unwrap_or_default();

    // finally, configure the timer and enable timer interrupts
    timer0_setup(&dp.TC0);
    unsafe { avr_device::interrupt::enable() };

    let mut seconds = SECONDS.load(Ordering::SeqCst);
    let mut delay = Delay::new();
    loop {
        // reset the watchdog timer
        watchdog.feed();

        // if we're converting, wait until conversion is complete and then read the value
        state.update_temperature();

        // immediately start a new conversion
        state.start_conversion();

        // update compressor state if indicated
        state.update_cooling_state();

        // increment the dwell counter for min on- and off-time
        state.increment_dwell_cnt();

        // wait until the SECONDS timer rolls over before continuing
        loop {
            let current = SECONDS.load(Ordering::SeqCst);
            if current.wrapping_sub(seconds) > 0 {
                seconds = current;
                break;
            }
            delay.delay_ms(1u16);
        }
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

// see the Atmega328p datasheet for config register specs
fn timer0_setup(tc0: &avr_device::atmega328p::TC0) {
    // set up timer0 for CTC mode with 1024 prescaler and compA interrupt

    // ocr0a = 124; 16 MHz / (1024 * (1 + 124)) = 125 Hz
    // datasheet page 117
    tc0.ocr0a().write(|w| w.set(124));
    // tccr0a = 0x02, CTC mode
    // datasheet page 113
    tc0.tccr0a().write(|w| unsafe { w.bits(0x02) });
    // tccr0b = 0x05, CTC mode and 1024 prescaler
    // datasheet page 116
    tc0.tccr0b().write(|w| unsafe { w.bits(0x05) });
    // timsk0 = 0x02, compare match A interrupt enable
    // datasheet page 118
    tc0.timsk0().write(|w| unsafe { w.bits(0x02) });
    // clear the interrupt flag (NOTE: write 1 to clear flag!)
    tc0.tifr0().write(|w| w.ocf0a().bit(true));
}
