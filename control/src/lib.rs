use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    console, Document, HtmlButtonElement, HtmlInputElement, HtmlTextAreaElement, Navigator, Usb,
    UsbControlTransferParameters, UsbDevice, UsbDeviceFilter, UsbDeviceRequestOptions,
    UsbRecipient, UsbRequestType, UsbTransferStatus,
};

const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x0001;
const DFU_VENDOR_ID: u16 = 0x0483;  // STM32 DFU mode VID
const DFU_PRODUCT_ID: u16 = 0xDF11; // STM32 DFU mode PID
const WEBUSB_INTERFACE: u8 = 0;
const WEBUSB_ENDPOINT_IN: u8 = 1;
const WEBUSB_ENDPOINT_OUT: u8 = 1;

#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    console::log_1(&"Control panel initialized".into());
    Ok(())
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no global window exists")
}

fn document() -> Document {
    window().document().expect("no document exists")
}

fn navigator() -> Navigator {
    window().navigator()
}

fn usb() -> Result<Usb, JsValue> {
    navigator()
        .usb()
        .ok_or_else(|| JsValue::from_str("WebUSB not supported"))
}

#[wasm_bindgen]
pub async fn flash_firmware() -> Result<(), JsValue> {
    log("Starting DFU flash process...");
    log("Please put your device into DFU mode:");
    log("1. Hold BOOT0 button");
    log("2. Press and release RESET button");
    log("3. Release BOOT0 button");
    log("4. Click the button below to select the DFU device");

    // Request the DFU device
    let usb = usb()?;

    let filter = UsbDeviceFilter::new();
    filter.set_vendor_id(DFU_VENDOR_ID);
    filter.set_product_id(DFU_PRODUCT_ID);

    let filters = js_sys::Array::new();
    filters.push(&filter);

    let options = UsbDeviceRequestOptions::new(&filters);
    let device: UsbDevice = JsFuture::from(usb.request_device(&options))
        .await?
        .dyn_into()?;

    log(&format!(
        "Connected to DFU device: {} {}",
        device.manufacturer_name().unwrap_or_default(),
        device.product_name().unwrap_or_default()
    ));

    // Open the device
    JsFuture::from(device.open()).await?;
    log("Device opened");

    // Claim interface 0 (DFU interface)
    JsFuture::from(device.claim_interface(0)).await?;
    log("DFU interface claimed");

    // Get the firmware binary (embedded at build time)
    let firmware_bytes = include_bytes!("../../firmware/target/thumbv7em-none-eabihf/release/firmware.bin");
    log(&format!("Firmware size: {} bytes", firmware_bytes.len()));

    // DFU download process
    let block_size: usize = 2048;
    let mut block_num: u16 = 0;
    let mut offset: usize = 0;

    while offset < firmware_bytes.len() {
        let end = (offset + block_size).min(firmware_bytes.len());
        let chunk = &firmware_bytes[offset..end];

        // DFU_DNLOAD request
        let params = UsbControlTransferParameters::new(
            0,       // index (interface)
            21,      // DFU_DNLOAD request
            block_num, // value (block number)
        );
        params.set_recipient(UsbRecipient::Interface);
        params.set_request_type(UsbRequestType::Class);

        let data = js_sys::Uint8Array::from(chunk);
        JsFuture::from(device.control_transfer_out_with_buffer_source(&params, &data)).await?;

        // Wait for the device to be ready (poll DFU_GETSTATUS)
        loop {
            let status_params = UsbControlTransferParameters::new(0, 3, 0); // DFU_GETSTATUS
            status_params.set_recipient(UsbRecipient::Interface);
            status_params.set_request_type(UsbRequestType::Class);

            let result = JsFuture::from(device.control_transfer_in(&status_params, 6)).await?;
            let transfer: web_sys::UsbInTransferResult = result.dyn_into()?;

            if transfer.status() == UsbTransferStatus::Ok {
                if let Some(data) = transfer.data() {
                    let status = data.get_uint8(0);
                    let state = data.get_uint8(4);
                    if status == 0 && (state == 5 || state == 2) {
                        // dfuDNLOAD-IDLE or dfuIDLE
                        break;
                    } else if status != 0 {
                        return Err(JsValue::from_str(&format!("DFU error: status={}", status)));
                    }
                }
            }

            // Small delay
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                window()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 10)
                    .unwrap();
            });
            JsFuture::from(promise).await?;
        }

        offset = end;
        block_num += 1;

        let progress = (offset as f32 / firmware_bytes.len() as f32) * 100.0;
        log(&format!("Progress: {:.1}%", progress));
    }

    // Send zero-length packet to indicate end of download
    let params = UsbControlTransferParameters::new(0, 21, block_num);
    params.set_recipient(UsbRecipient::Interface);
    params.set_request_type(UsbRequestType::Class);

    let empty = js_sys::Uint8Array::new_with_length(0);
    JsFuture::from(device.control_transfer_out_with_buffer_source(&params, &empty)).await?;

    // Detach and reset
    let detach_params = UsbControlTransferParameters::new(0, 0, 0); // DFU_DETACH
    detach_params.set_recipient(UsbRecipient::Interface);
    detach_params.set_request_type(UsbRequestType::Class);
    let _ = device.control_transfer_out(&detach_params);

    log("Firmware flashed successfully!");
    log("Device will reset. Please wait a few seconds, then connect.");

    JsFuture::from(device.close()).await?;

    Ok(())
}

#[wasm_bindgen]
pub async fn connect_device() -> Result<JsValue, JsValue> {
    log("Connecting to device...");

    let usb = usb()?;

    let filter = UsbDeviceFilter::new();
    filter.set_vendor_id(VENDOR_ID);
    filter.set_product_id(PRODUCT_ID);

    let filters = js_sys::Array::new();
    filters.push(&filter);

    let options = UsbDeviceRequestOptions::new(&filters);
    let device: UsbDevice = JsFuture::from(usb.request_device(&options))
        .await?
        .dyn_into()?;

    log(&format!(
        "Selected: {} {}",
        device.manufacturer_name().unwrap_or_default(),
        device.product_name().unwrap_or_default()
    ));

    JsFuture::from(device.open()).await?;
    log("Device opened");

    if device.configuration().is_none() {
        JsFuture::from(device.select_configuration(1)).await?;
    }

    JsFuture::from(device.claim_interface(WEBUSB_INTERFACE.into())).await?;
    log("Interface claimed, ready to communicate");

    Ok(device.into())
}

#[wasm_bindgen]
pub async fn send_text(device: &UsbDevice, text: &str) -> Result<(), JsValue> {
    let data = format!("{}\n", text);
    let bytes = js_sys::Uint8Array::from(data.as_bytes());

    JsFuture::from(device.transfer_out_with_buffer_source(WEBUSB_ENDPOINT_OUT, &bytes)).await?;
    log(&format!("Sent: {}", text));

    Ok(())
}

#[wasm_bindgen]
pub async fn receive_text(device: &UsbDevice) -> Result<String, JsValue> {
    let result = JsFuture::from(device.transfer_in(WEBUSB_ENDPOINT_IN, 64)).await?;
    let transfer: web_sys::UsbInTransferResult = result.dyn_into()?;

    if transfer.status() != UsbTransferStatus::Ok {
        return Err(JsValue::from_str("Transfer failed"));
    }

    let data = transfer.data().ok_or("No data received")?;
    let mut bytes = vec![0u8; data.byte_length() as usize];
    for i in 0..bytes.len() {
        bytes[i] = data.get_uint8(i as u32);
    }

    let text = String::from_utf8_lossy(&bytes).to_string();
    if !text.is_empty() {
        log(&format!("Received: {}", text.trim()));
    }

    Ok(text)
}

#[wasm_bindgen]
pub async fn disconnect_device(device: &UsbDevice) -> Result<(), JsValue> {
    JsFuture::from(device.release_interface(WEBUSB_INTERFACE.into())).await?;
    JsFuture::from(device.close()).await?;
    log("Device disconnected");
    Ok(())
}

fn log(msg: &str) {
    console::log_1(&msg.into());

    if let Ok(output) = document().get_element_by_id("output").ok_or(()) {
        if let Ok(textarea) = output.dyn_into::<HtmlTextAreaElement>() {
            let current = textarea.value();
            textarea.set_value(&format!("{}{}\n", current, msg));
            textarea.set_scroll_top(textarea.scroll_height());
        }
    }
}
