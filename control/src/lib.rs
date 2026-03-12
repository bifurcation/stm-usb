use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    console, Document, HtmlTextAreaElement, Navigator, Usb, UsbControlTransferParameters,
    UsbDevice, UsbDeviceFilter, UsbDeviceRequestOptions, UsbRecipient, UsbRequestType,
    UsbTransferStatus,
};

const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x0001;
const DFU_VENDOR_ID: u16 = 0x0483;
const DFU_PRODUCT_ID: u16 = 0xDF11;
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

fn usb() -> Usb {
    navigator().usb()
}

#[wasm_bindgen]
pub async fn flash_firmware(firmware_data: &[u8]) -> Result<(), JsValue> {
    log("Starting DFU flash process...");

    let usb = usb();

    let filter = UsbDeviceFilter::new();
    filter.set_vendor_id(DFU_VENDOR_ID);
    filter.set_product_id(DFU_PRODUCT_ID);

    let filters = [filter];
    let options = UsbDeviceRequestOptions::new(&filters);

    let device: UsbDevice = JsFuture::from(usb.request_device(&options))
        .await?
        .dyn_into()?;

    log(&format!(
        "Connected to DFU device: {} {}",
        device.manufacturer_name().unwrap_or_default(),
        device.product_name().unwrap_or_default()
    ));

    JsFuture::from(device.open()).await?;
    log("Device opened");

    JsFuture::from(device.claim_interface(0)).await?;
    log("DFU interface claimed");

    let firmware_bytes = firmware_data;
    log(&format!("Firmware size: {} bytes", firmware_bytes.len()));

    let block_size: usize = 2048;
    let mut block_num: u16 = 0;
    let mut offset: usize = 0;

    while offset < firmware_bytes.len() {
        let end = (offset + block_size).min(firmware_bytes.len());
        let chunk = &firmware_bytes[offset..end];

        let params = UsbControlTransferParameters::new(
            0,
            UsbRecipient::Interface,
            21,
            UsbRequestType::Class,
            block_num,
        );

        let data = js_sys::Uint8Array::from(chunk);
        JsFuture::from(
            device
                .control_transfer_out_with_buffer_source(&params, &data)?
        )
        .await?;

        loop {
            let status_params = UsbControlTransferParameters::new(
                0,
                UsbRecipient::Interface,
                3,
                UsbRequestType::Class,
                0,
            );

            let transfer: web_sys::UsbInTransferResult =
                JsFuture::from(device.control_transfer_in(&status_params, 6)).await?.dyn_into()?;

            if transfer.status() == UsbTransferStatus::Ok {
                if let Some(data) = transfer.data() {
                    let status: u8 = data.get_uint8(0);
                    let state: u8 = data.get_uint8(4);
                    if status == 0 && (state == 5 || state == 2) {
                        break;
                    } else if status != 0 {
                        return Err(JsValue::from_str(&format!("DFU error: status={}", status)));
                    }
                }
            }

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

    let params = UsbControlTransferParameters::new(
        0,
        UsbRecipient::Interface,
        21,
        UsbRequestType::Class,
        block_num,
    );

    let empty = js_sys::Uint8Array::new_with_length(0);
    JsFuture::from(device.control_transfer_out_with_buffer_source(&params, &empty)?).await?;

    let detach_params = UsbControlTransferParameters::new(
        0,
        UsbRecipient::Interface,
        0,
        UsbRequestType::Class,
        0,
    );
    let _ = device.control_transfer_out(&detach_params);

    log("Firmware flashed successfully!");
    log("Device will reset. Please wait a few seconds, then connect.");

    JsFuture::from(device.close()).await?;

    Ok(())
}

#[wasm_bindgen]
pub async fn connect_device() -> Result<JsValue, JsValue> {
    log("Connecting to device...");

    let usb = usb();

    let filter = UsbDeviceFilter::new();
    filter.set_vendor_id(VENDOR_ID);
    filter.set_product_id(PRODUCT_ID);

    let filters = [filter];
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

    JsFuture::from(device.transfer_out_with_buffer_source(WEBUSB_ENDPOINT_OUT, &bytes)?).await?;
    log(&format!("Sent: {}", text));

    Ok(())
}

#[wasm_bindgen]
pub async fn receive_text(device: &UsbDevice) -> Result<String, JsValue> {
    let transfer: web_sys::UsbInTransferResult =
        JsFuture::from(device.transfer_in(WEBUSB_ENDPOINT_IN, 64)).await?.dyn_into()?;

    if transfer.status() != UsbTransferStatus::Ok {
        return Err(JsValue::from_str("Transfer failed"));
    }

    let data = transfer.data().ok_or("No data received")?;
    let mut bytes = vec![0u8; data.byte_length() as usize];
    for i in 0..bytes.len() {
        bytes[i] = data.get_uint8(i);
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

    if let Some(output) = document().get_element_by_id("output") {
        if let Ok(textarea) = output.dyn_into::<HtmlTextAreaElement>() {
            let current = textarea.value();
            textarea.set_value(&format!("{}{}\n", current, msg));
            textarea.set_scroll_top(textarea.scroll_height() as f64);
        }
    }
}
