#![no_std]
#![no_main]
#![allow(static_mut_refs)] // DFU_FLAG access is safe: single-threaded pre_init context

mod echo;

use core::mem::MaybeUninit;
use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::pac;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_time::{Duration, Timer};
use embassy_usb::class::web_usb::{Config as WebUsbConfig, State as WebUsbState, WebUsb};
use embassy_usb::msos::{self, windows_version};
use embassy_usb::Builder;
use embassy_usb_dfu::consts::DfuAttributes;
use embassy_usb_dfu::{usb_dfu, Control, DfuMarker, Reset};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// Magic value for DFU reset - unlikely to occur naturally in uninitialized RAM
const DFU_MAGIC: u32 = 0xDEAD_BEEF;

// STM32F4 system bootloader address - see AN2606
const SYSTEM_MEMORY_BASE: u32 = 0x1FFF_0000;

// Placed in .uninit so startup code does not zero it
#[link_section = ".uninit.DFU_MAGIC"]
static mut DFU_FLAG: MaybeUninit<u32> = MaybeUninit::uninit();

/// Runs before RAM initialization on every boot.
/// If the magic value is present, remaps system memory and jumps to the ST ROM bootloader.
#[cortex_m_rt::pre_init]
unsafe fn check_bootloader_magic() {
    if DFU_FLAG.assume_init() == DFU_MAGIC {
        // Clear magic so we don't loop
        DFU_FLAG.as_mut_ptr().write(0);

        // The ROM bootloader expects its own vector table mapped at 0x00000000.
        // Without this remap it crashes silently before USB initialises.

        // Enable SYSCFG clock
        pac::RCC.apb2enr().modify(|w| w.set_syscfgen(true));

        // Remap system memory to 0x00000000 (mem_mode = 1)
        pac::SYSCFG.memrm().write(|w| w.set_mem_mode(1));

        // Jump to ROM bootloader (reads SP and PC from vector table)
        cortex_m::asm::bootload(SYSTEM_MEMORY_BASE as *const u32);
    }
}

/// DFU marker that writes magic to .uninit RAM
struct RomBootloaderMarker;

impl DfuMarker for RomBootloaderMarker {
    fn mark_dfu(&mut self) {
        info!("DFU detach requested, writing magic");
        unsafe {
            DFU_FLAG.as_mut_ptr().write(DFU_MAGIC);
        }
    }
}

/// Custom reset that disables USB before resetting.
/// This gives the host time to see the disconnect before bootloader re-enumerates.
struct ResetToBootloader;

impl Reset for ResetToBootloader {
    fn sys_reset(&self) {
        info!("Disabling USB and resetting to bootloader");

        // Disable USB OTG FS clock - this drops D+ low, signaling disconnect to host
        pac::RCC.ahb2enr().modify(|w| w.set_usb_otg_fsen(false));

        // Busy-wait ~5 ms at 84 MHz to give the host time to register the disconnect
        cortex_m::asm::delay(84_000 * 5);

        cortex_m::peripheral::SCB::sys_reset()
    }
}

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

#[embassy_executor::task]
async fn blink_led(mut led: Output<'static>) {
    loop {
        led.set_high();
        Timer::after_millis(500).await;
        led.set_low();
        Timer::after_millis(500).await;
    }
}

const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x0001;
const MANUFACTURER: &str = "Hactar";
const PRODUCT: &str = "STM-USB Echo";
const SERIAL: &str = "12345678";

#[embassy_executor::main]
async fn main(#[allow(unused)] spawner: Spawner) {
    // Configure clocks
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        #[cfg(feature = "stm32f411")]
        {
            // Black Pill board: 25 MHz HSE crystal
            config.rcc.hse = Some(Hse {
                freq: embassy_stm32::time::Hertz(25_000_000),
                mode: HseMode::Oscillator,
            });
            config.rcc.pll_src = PllSource::HSE;
            config.rcc.pll = Some(Pll {
                prediv: PllPreDiv::DIV25,
                mul: PllMul::MUL336,
                divp: Some(PllPDiv::DIV4),
                divq: Some(PllQDiv::DIV7),
                divr: None,
            });
        }
        #[cfg(feature = "stm32f412")]
        {
            // Nucleo-144 board: 8 MHz from ST-LINK MCO
            config.rcc.hse = Some(Hse {
                freq: embassy_stm32::time::Hertz(8_000_000),
                mode: HseMode::Bypass,
            });
            config.rcc.pll_src = PllSource::HSE;
            config.rcc.pll = Some(Pll {
                prediv: PllPreDiv::DIV8,
                mul: PllMul::MUL336,
                divp: Some(PllPDiv::DIV4),
                divq: Some(PllQDiv::DIV7),
                divr: None,
            });
        }
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
        config.rcc.sys = Sysclk::PLL1_P;
    }

    let p = embassy_stm32::init(config);
    info!("USB echo device starting");

    // Spawn a task to blink an LED to show we're alive
    #[cfg(feature = "stm32f411")]
    {
        let led = Output::new(p.PC13, Level::Low, Speed::Low);
        spawner.spawn(blink_led(led)).unwrap();
    }
    #[cfg(feature = "stm32f412")]
    {
        let led = Output::new(p.PB14, Level::Low, Speed::Low); // Green LED
        spawner.spawn(blink_led(led)).unwrap();
    }

    // Configure USB
    static EP_OUT_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    let ep_out_buffer = EP_OUT_BUFFER.init([0u8; 256]);

    let mut usb_config = embassy_stm32::usb::Config::default();
    usb_config.vbus_detection = false;

    let driver = Driver::new_fs(
        p.USB_OTG_FS,
        Irqs,
        p.PA12,
        p.PA11,
        ep_out_buffer,
        usb_config,
    );

    let mut config = embassy_usb::Config::new(VENDOR_ID, PRODUCT_ID);
    config.manufacturer = Some(MANUFACTURER);
    config.product = Some(PRODUCT);
    config.serial_number = Some(SERIAL);
    config.device_class = 0xEF; // Miscellaneous (required for IAD composite)
    config.device_sub_class = 0x02; // Common Class
    config.device_protocol = 0x01; // Interface Association Descriptor

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        MSOS_DESC.init([0; 256]),
        CONTROL_BUF.init([0; 128]),
    );

    // Add WebUSB capability
    static WEBUSB_STATE: StaticCell<WebUsbState> = StaticCell::new();
    static WEBUSB_CONFIG: StaticCell<WebUsbConfig> = StaticCell::new();
    let webusb_config = WEBUSB_CONFIG.init(WebUsbConfig {
        max_packet_size: 64,
        vendor_code: 1,
        landing_url: None, // Allow any origin
    });
    let webusb_state = WEBUSB_STATE.init(WebUsbState::new());
    WebUsb::configure(&mut builder, webusb_state, webusb_config);

    // Add MS OS 2.0 descriptors for Windows compatibility
    builder.msos_descriptor(windows_version::WIN8_1, 2);
    builder.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));

    // Add DFU runtime interface - handles DFU_DETACH command from host
    static DFU_CONTROL: StaticCell<Control<RomBootloaderMarker, ResetToBootloader>> = StaticCell::new();
    let dfu_control = DFU_CONTROL.init(Control::new(
        RomBootloaderMarker,
        DfuAttributes::CAN_DOWNLOAD | DfuAttributes::WILL_DETACH,
        ResetToBootloader,
    ));
    usb_dfu(&mut builder, dfu_control, Duration::from_millis(2500), |_| {});

    // Create vendor-specific function with bulk endpoints
    let (ep_out, ep_in) = {
        let mut function = builder.function(0xFF, 0x00, 0x00);
        let mut interface = function.interface();
        let mut alt = interface.alt_setting(0xFF, 0x00, 0x00, None);
        let ep_out = alt.endpoint_bulk_out(None, 64);
        let ep_in = alt.endpoint_bulk_in(None, 64);
        (ep_out, ep_in)
    };

    let mut usb = builder.build();
    info!("USB device built, starting USB task");

    // Spawn tasks to run the USB protocol itself and the echo logic
    info!("Starting USB and echo tasks");
    embassy_futures::join::join(usb.run(), echo::run(ep_in, ep_out)).await;
}
