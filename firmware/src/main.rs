#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::usb::Driver;
use embassy_stm32::{bind_interrupts, peripherals, usb, Config};
use embassy_usb::class::web_usb::{Config as WebUsbConfig, State as WebUsbState, Url, WebUsb};
use embassy_usb::driver::EndpointError;
use embassy_usb::driver::{Endpoint, EndpointIn, EndpointOut};
use embassy_usb::msos::{self, windows_version};
use embassy_usb::Builder;
use heapless::Vec;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});

const LANDING_PAGE_URL: &str = "localhost:8080";
const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x0001;
const MANUFACTURER: &str = "Hactar";
const PRODUCT: &str = "STM-USB Echo";
const SERIAL: &str = "12345678";

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
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
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
        config.rcc.sys = Sysclk::PLL1_P;
    }

    let p = embassy_stm32::init(config);
    info!("USB echo device starting");

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
    config.device_class = 0xFF; // Vendor-specific
    config.device_sub_class = 0x00;
    config.device_protocol = 0x00;

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
        landing_url: Some(Url::new(LANDING_PAGE_URL)),
    });
    let webusb_state = WEBUSB_STATE.init(WebUsbState::new());
    WebUsb::configure(&mut builder, webusb_state, webusb_config);

    // Add MS OS 2.0 descriptors for Windows compatibility
    builder.msos_descriptor(windows_version::WIN8_1, 2);
    builder.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));

    // Create vendor-specific function with bulk endpoints
    let (mut ep_out, mut ep_in) = {
        let mut function = builder.function(0xFF, 0x00, 0x00);
        let mut interface = function.interface();
        let mut alt = interface.alt_setting(0xFF, 0x00, 0x00, None);
        let ep_out = alt.endpoint_bulk_out(64);
        let ep_in = alt.endpoint_bulk_in(64);
        (ep_out, ep_in)
    };

    let mut usb = builder.build();
    let usb_fut = usb.run();

    let echo_fut = async {
        loop {
            // Wait for USB to be configured
            ep_out.wait_enabled().await;
            info!("USB configured, ready for data");

            let mut line_buf: Vec<u8, 256> = Vec::new();
            let mut read_buf = [0u8; 64];

            loop {
                match ep_out.read(&mut read_buf).await {
                    Ok(n) => {
                        for &byte in &read_buf[..n] {
                            if byte == b'\n' || byte == b'\r' {
                                if !line_buf.is_empty() {
                                    if let Ok(s) = core::str::from_utf8(&line_buf) {
                                        info!("Received: {}", s);
                                    }
                                    // Echo the line back with newline
                                    if ep_in.write(&line_buf).await.is_err() {
                                        info!("Write error");
                                        break;
                                    }
                                    if ep_in.write(b"\r\n").await.is_err() {
                                        info!("Write error");
                                        break;
                                    }
                                    line_buf.clear();
                                }
                            } else if line_buf.push(byte).is_err() {
                                info!("Line buffer overflow");
                                line_buf.clear();
                            }
                        }
                    }
                    Err(EndpointError::BufferOverflow) => {
                        info!("Buffer overflow");
                    }
                    Err(EndpointError::Disabled) => {
                        info!("USB disconnected");
                        break;
                    }
                }
            }
        }
    };

    embassy_futures::join::join(usb_fut, echo_fut).await;
}
