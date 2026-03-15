use dfu_core::asynchronous::DfuASync;
use dfu_core::functional_descriptor::FunctionalDescriptor;
use dfu_core::memory_layout::MemoryLayout;
use dfu_core::{DfuProtocol, Error as DfuError};
use send_wrapper::SendWrapper;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
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
const WEBUSB_INTERFACE: u8 = 2; // Interface 0=WebUSB desc, 1=DFU runtime, 2=vendor bulk
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

// Wrapper for non-Send futures (safe in single-threaded WASM)
struct WasmFuture<F>(SendWrapper<F>);

impl<F: Future> Future for WasmFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: SendWrapper guarantees single-thread access
        unsafe {
            let inner = self.map_unchecked_mut(|s| &mut *s.0);
            inner.poll(cx)
        }
    }
}

// SAFETY: WASM is single-threaded
unsafe impl<F> Send for WasmFuture<F> {}

// Error type for DFU operations
struct FlashError(String);

impl std::fmt::Debug for FlashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<DfuError> for FlashError {
    fn from(e: DfuError) -> Self {
        FlashError(format!("{:?}", e))
    }
}

impl From<io::Error> for FlashError {
    fn from(e: io::Error) -> Self {
        FlashError(e.to_string())
    }
}

impl From<JsValue> for FlashError {
    fn from(e: JsValue) -> Self {
        FlashError(format!("{:?}", e))
    }
}

impl From<String> for FlashError {
    fn from(e: String) -> Self {
        FlashError(e)
    }
}

// WebUSB DFU IO adapter
struct WebUsbDfu {
    device: UsbDevice,
    descriptor: FunctionalDescriptor,
    protocol: DfuProtocol<MemoryLayout>,
    total_size: usize,
    bytes_written: std::cell::Cell<usize>,
}

impl dfu_core::asynchronous::DfuAsyncIo for WebUsbDfu {
    type Read = usize;
    type Write = usize;
    type Reset = ();
    type Error = FlashError;
    type MemoryLayout = MemoryLayout;

    fn read_control(
        &self,
        _request_type: u8,
        request: u8,
        value: u16,
        buffer: &mut [u8],
    ) -> impl Future<Output = Result<Self::Read, Self::Error>> + Send {
        let device = self.device.clone();
        let len = buffer.len();
        let buffer_ptr = buffer.as_mut_ptr();
        let buffer_len = buffer.len();

        WasmFuture(SendWrapper::new(async move {
            let params = UsbControlTransferParameters::new(
                0,
                UsbRecipient::Interface,
                request,
                UsbRequestType::Class,
                value,
            );

            let transfer: web_sys::UsbInTransferResult =
                JsFuture::from(device.control_transfer_in(&params, len as u16))
                    .await
                    .map_err(|e| FlashError(format!("{:?}", e)))?;

            if transfer.status() != UsbTransferStatus::Ok {
                return Err(FlashError("Control transfer IN failed".into()));
            }

            if let Some(data) = transfer.data() {
                let n = data.byte_length() as usize;
                // SAFETY: buffer_ptr is valid for buffer_len bytes, single-threaded WASM
                let buffer = unsafe { std::slice::from_raw_parts_mut(buffer_ptr, buffer_len) };
                for i in 0..n.min(buffer.len()) {
                    buffer[i] = data.get_uint8(i);
                }
                Ok(n)
            } else {
                Ok(0)
            }
        }))
    }

    fn write_control(
        &self,
        _request_type: u8,
        request: u8,
        value: u16,
        buffer: &[u8],
    ) -> impl Future<Output = Result<Self::Write, Self::Error>> + Send {
        let device = self.device.clone();
        let buffer = buffer.to_vec();
        let total_size = self.total_size;

        // Track progress for data blocks (value >= 2 means actual data in DfuSe)
        // DfuSe uses block 0 for commands, block 2+ for data
        let is_data_block = value >= 2 && !buffer.is_empty();
        let bytes_so_far = if is_data_block {
            let current = self.bytes_written.get();
            let new_total = current + buffer.len();
            self.bytes_written.set(new_total);
            Some(new_total)
        } else {
            None
        };

        WasmFuture(SendWrapper::new(async move {
            let params = UsbControlTransferParameters::new(
                0,
                UsbRecipient::Interface,
                request,
                UsbRequestType::Class,
                value,
            );

            let data = js_sys::Uint8Array::from(buffer.as_slice());
            let transfer: web_sys::UsbOutTransferResult = JsFuture::from(
                device
                    .control_transfer_out_with_buffer_source(&params, &data)
                    .map_err(|e| FlashError(format!("{:?}", e)))?,
            )
            .await
            .map_err(|e| FlashError(format!("{:?}", e)))?;

            if transfer.status() != UsbTransferStatus::Ok {
                return Err(FlashError("Control transfer OUT failed".into()));
            }

            // Update progress bar for data blocks
            if let Some(written) = bytes_so_far {
                let progress = (written as f32 / total_size as f32) * 100.0;
                update_progress(progress, written, total_size);
            }

            Ok(buffer.len())
        }))
    }

    fn usb_reset(&mut self) -> impl Future<Output = Result<Self::Reset, Self::Error>> + Send {
        // WebUSB doesn't have a reset method, device will reset itself
        WasmFuture(SendWrapper::new(async { Ok(()) }))
    }

    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        let ms = duration.as_millis() as i32;
        WasmFuture(SendWrapper::new(async move {
            let promise = js_sys::Promise::new(&mut |resolve, _| {
                web_sys::window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                    .unwrap();
            });
            let _ = JsFuture::from(promise).await;
        }))
    }

    fn protocol(&self) -> &DfuProtocol<Self::MemoryLayout> {
        &self.protocol
    }

    fn functional_descriptor(&self) -> &FunctionalDescriptor {
        &self.descriptor
    }
}

/// Prompt user to authorize the DFU bootloader device.
/// Call this during initial pairing while you have a user gesture.
#[wasm_bindgen]
pub async fn pair_bootloader() -> Result<(), JsValue> {
    log("Please select the DFU bootloader device to authorize it...");

    let usb = usb();

    let filter = UsbDeviceFilter::new();
    filter.set_vendor_id(DFU_VENDOR_ID);
    filter.set_product_id(DFU_PRODUCT_ID);

    let filters = [filter];
    let options = UsbDeviceRequestOptions::new(&filters);

    let _device: UsbDevice = JsFuture::from(usb.request_device(&options))
        .await?
        .dyn_into()?;

    log("Bootloader device authorized for future use.");
    Ok(())
}

/// Check if bootloader device is already authorized (without prompting)
#[wasm_bindgen]
pub async fn get_bootloader_device() -> Result<JsValue, JsValue> {
    let usb = usb();
    let devices: js_sys::Array = JsFuture::from(usb.get_devices()).await?.dyn_into()?;

    for i in 0..devices.length() {
        let device: UsbDevice = devices.get(i).dyn_into()?;
        if device.vendor_id() == DFU_VENDOR_ID && device.product_id() == DFU_PRODUCT_ID {
            return Ok(device.into());
        }
    }

    Err(JsValue::from_str("Bootloader not found"))
}

/// Flash firmware to an already-obtained DFU device
#[wasm_bindgen]
pub async fn flash_to_device(device: &UsbDevice, firmware_data: &[u8]) -> Result<(), JsValue> {
    log("Starting DFU flash...");

    JsFuture::from(device.open()).await?;
    log("Device opened");

    JsFuture::from(device.claim_interface(0)).await?;
    log("DFU interface claimed");

    // STM32 DfuSe functional descriptor
    let descriptor = FunctionalDescriptor {
        can_download: true,
        can_upload: true,
        manifestation_tolerant: false,
        will_detach: true,
        detach_timeout: 255,
        transfer_size: 2048,
        dfu_version: (0x01, 0x1a), // DfuSe
    };

    // STM32F4 internal flash layout
    let memory_layout = MemoryLayout::try_from("04*016Kg,01*064Kg,03*128Kg")
        .map_err(|e| JsValue::from_str(&format!("Memory layout error: {:?}", e)))?;

    let protocol = DfuProtocol::Dfuse {
        address: 0x0800_0000,
        memory_layout,
    };

    log(&format!("Firmware size: {} bytes", firmware_data.len()));

    let io = WebUsbDfu {
        device: device.clone(),
        descriptor,
        protocol,
        total_size: firmware_data.len(),
        bytes_written: std::cell::Cell::new(0),
    };

    let mut dfu = DfuASync::new(io);

    log("Starting DfuSe download...");
    let result = dfu.download_from_slice(firmware_data).await;

    hide_progress();

    result.map_err(|e| JsValue::from_str(&format!("DFU error: {:?}", e)))?;

    log("Firmware flashed successfully!");
    log("Device will reset. Reconnect when ready.");

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
    JsFuture::from(device.select_alternate_interface(WEBUSB_INTERFACE.into(), 0)).await?;
    log("Ready to communicate");

    Ok(device.into())
}

/// Helper to check if a USB transfer is a ready signal (0-length or 0x0000)
fn is_ready_signal(transfer: &web_sys::UsbInTransferResult) -> bool {
    match transfer.data() {
        None => true,
        Some(data) => {
            let len = data.byte_length();
            len == 0 || (len >= 2 && data.get_uint8(0) == 0 && data.get_uint8(1) == 0)
        }
    }
}

/// Read from device until we get a ready signal, accumulating any response data
async fn read_until_ready(device: &UsbDevice, buffer: &mut Vec<u8>) -> Result<(), JsValue> {
    loop {
        let transfer: web_sys::UsbInTransferResult =
            JsFuture::from(device.transfer_in(WEBUSB_ENDPOINT_IN, 64))
                .await?
                .dyn_into()?;

        if transfer.status() != UsbTransferStatus::Ok {
            return Err(JsValue::from_str("Transfer failed"));
        }

        if is_ready_signal(&transfer) {
            return Ok(());
        }

        // Not a ready signal - it's response data, accumulate it
        if let Some(data) = transfer.data() {
            for i in 0..data.byte_length() {
                buffer.push(data.get_uint8(i as usize));
            }
        }
    }
}

/// Send text with length-prefix framing and flow control
#[wasm_bindgen]
pub async fn send_text(device: &UsbDevice, text: &str) -> Result<String, JsValue> {
    let performance = window()
        .performance()
        .ok_or("Performance API not available")?;

    let payload = text.as_bytes();
    let len = payload.len() as u16;

    // Build length-prefixed message
    let mut data = Vec::with_capacity(2 + payload.len());
    data.extend_from_slice(&len.to_le_bytes());
    data.extend_from_slice(payload);

    // Start timing from first byte sent
    let start_time = performance.now();

    // Buffer to accumulate response data as we send chunks
    let mut buffer: Vec<u8> = Vec::new();

    // Send data in chunks, collecting response data after each
    const CHUNK_SIZE: usize = 64;
    let mut offset = 0;

    while offset < data.len() {
        let end = (offset + CHUNK_SIZE).min(data.len());
        let chunk = &data[offset..end];
        let bytes = js_sys::Uint8Array::from(chunk);
        JsFuture::from(device.transfer_out_with_buffer_source(WEBUSB_ENDPOINT_OUT, &bytes)?).await?;
        offset = end;

        // After each chunk, read response data until we get a ready signal
        read_until_ready(device, &mut buffer).await?;
    }

    log(&format!("Sent ({} bytes): {}", len, text));

    // Parse the response from accumulated buffer
    let response = if buffer.len() >= 2 {
        let response_len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
        let total_expected = 2 + response_len;
        if buffer.len() >= total_expected {
            String::from_utf8_lossy(&buffer[2..total_expected]).to_string()
        } else {
            return Err(JsValue::from_str(&format!(
                "Incomplete response: got {} bytes, expected {}",
                buffer.len(),
                total_expected
            )));
        }
    } else {
        return Err(JsValue::from_str("No response received"));
    };

    // End timing when last byte received
    let end_time = performance.now();
    let elapsed_ms = end_time - start_time;

    // Calculate throughput: payload bytes sent, round-trip time
    let bytes_per_sec = if elapsed_ms > 0.0 {
        (len as f64) / (elapsed_ms / 1000.0)
    } else {
        0.0
    };

    log(&format!(
        "Received ({} bytes) in {:.2}ms ({:.0} bytes/sec): {}",
        response.len(),
        elapsed_ms,
        bytes_per_sec,
        response
    ));

    Ok(response)
}

#[wasm_bindgen]
pub async fn disconnect_device(device: &UsbDevice) -> Result<(), JsValue> {
    JsFuture::from(device.release_interface(WEBUSB_INTERFACE.into())).await?;
    JsFuture::from(device.close()).await?;
    log("Device disconnected");
    Ok(())
}

/// Send DFU_DETACH to trigger device reset into DFU bootloader mode.
/// Releases all interfaces and closes the device.
#[wasm_bindgen]
pub async fn detach_to_dfu(device: &UsbDevice) -> Result<(), JsValue> {
    log("Sending DFU_DETACH command...");

    // DFU runtime interface is at index 1 (0=WebUSB desc, 1=DFU, 2=vendor bulk)
    const DFU_INTERFACE: u8 = 1;
    const DFU_DETACH: u8 = 0;
    const DETACH_TIMEOUT: u16 = 1000;

    // Claim the DFU interface
    JsFuture::from(device.claim_interface(DFU_INTERFACE.into())).await?;

    // Send DFU_DETACH control transfer
    let params = UsbControlTransferParameters::new(
        DFU_INTERFACE.into(),
        UsbRecipient::Interface,
        DFU_DETACH,
        UsbRequestType::Class,
        DETACH_TIMEOUT,
    );

    // DFU_DETACH has no data
    let empty = js_sys::Uint8Array::new_with_length(0);
    let _ = JsFuture::from(device.control_transfer_out_with_buffer_source(&params, &empty)?).await;

    // Release all interfaces and close - device will reset
    let _ = JsFuture::from(device.release_interface(DFU_INTERFACE.into())).await;
    let _ = JsFuture::from(device.release_interface(WEBUSB_INTERFACE.into())).await;
    let _ = JsFuture::from(device.close()).await;

    log("DFU_DETACH sent. Device resetting to bootloader...");

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

fn update_progress(percent: f32, bytes_written: usize, total_bytes: usize) {
    console::log_1(&format!("Progress: {:.1}% ({}/{})", percent, bytes_written, total_bytes).into());

    // Update HTML progress bar
    if let Some(container) = document().get_element_by_id("progressContainer") {
        let _ = container.class_list().add_1("active");
    }
    if let Some(bar) = document().get_element_by_id("progressBar") {
        let _ = bar.dyn_ref::<web_sys::HtmlElement>()
            .map(|el| el.style().set_property("width", &format!("{}%", percent)));
    }
    if let Some(text) = document().get_element_by_id("progressText") {
        text.set_text_content(Some(&format!("{:.0}%", percent)));
    }
}

fn hide_progress() {
    if let Some(container) = document().get_element_by_id("progressContainer") {
        let _ = container.class_list().remove_1("active");
    }
    if let Some(bar) = document().get_element_by_id("progressBar") {
        let _ = bar.dyn_ref::<web_sys::HtmlElement>()
            .map(|el| el.style().set_property("width", "0%"));
    }
}
