//! Length-prefixed streaming echo protocol over USB bulk endpoints.

use defmt::info;
use embassy_usb::driver::{EndpointError, EndpointIn, EndpointOut};
use heapless::Vec;

/// Ready signal for flow control (0x0000)
const READY_SIGNAL: [u8; 2] = [0x00, 0x00];

/// State machine for length-prefixed streaming echo protocol
enum State {
    /// Waiting for 2-byte little-endian length prefix
    WaitingForLength { buf: [u8; 2], pos: usize },
    /// Streaming payload bytes to output
    StreamingPayload { remaining: u16 },
}

impl State {
    fn new() -> Self {
        Self::WaitingForLength { buf: [0; 2], pos: 0 }
    }
}

/// Process incoming bytes through the echo state machine.
/// Returns true on success, false on write error.
async fn process_packet<E: EndpointIn>(
    data: &[u8],
    state: &mut State,
    out_buf: &mut Vec<u8, 64>,
    ep_in: &mut E,
) -> bool {
    let mut i = 0;
    while i < data.len() {
        match state {
            State::WaitingForLength { buf, pos } => {
                buf[*pos] = data[i];
                *pos += 1;
                i += 1;

                if *pos == 2 {
                    let len = u16::from_le_bytes(*buf);
                    info!("Echoing {} bytes of data...", len);

                    let response_len = len.saturating_add(5);
                    out_buf.clear();
                    out_buf.extend_from_slice(&response_len.to_le_bytes()).ok();
                    out_buf.extend_from_slice(b"ECHO ").ok();

                    *state = State::StreamingPayload { remaining: len };
                }
            }
            State::StreamingPayload { remaining } => {
                let bytes_to_copy = (data.len() - i).min(*remaining as usize);
                for &byte in &data[i..i + bytes_to_copy] {
                    if out_buf.is_full() {
                        if ep_in.write(out_buf).await.is_err() {
                            return false;
                        }
                        out_buf.clear();
                    }
                    let _ = out_buf.push(byte);
                }
                i += bytes_to_copy;
                *remaining -= bytes_to_copy as u16;

                if *remaining == 0 {
                    if !out_buf.is_empty() {
                        if ep_in.write(out_buf).await.is_err() {
                            return false;
                        }
                        out_buf.clear();
                    }
                    info!("... complete");
                    *state = State::new();
                }
            }
        }
    }
    true
}

/// Run the echo protocol on the given USB bulk endpoints.
/// Loops forever, handling USB disconnects and reconnects.
pub async fn run<I: EndpointIn, O: EndpointOut>(mut ep_in: I, mut ep_out: O) -> ! {
    info!("Echo task started, waiting for USB connection");
    let mut read_buf = [0u8; 64];
    let mut out_buf: Vec<u8, 64> = Vec::new();

    loop {
        ep_out.wait_enabled().await;
        info!("USB configured, ready for data");

        let mut state = State::new();

        loop {
            match ep_out.read(&mut read_buf).await {
                Ok(n) => {
                    if !process_packet(&read_buf[..n], &mut state, &mut out_buf, &mut ep_in).await {
                        info!("Write error");
                        break;
                    }
                    if ep_in.write(&READY_SIGNAL).await.is_err() {
                        info!("Write error sending ready");
                        break;
                    }
                }
                Err(EndpointError::BufferOverflow) => info!("Buffer overflow"),
                Err(EndpointError::Disabled) => {
                    info!("USB disconnected");
                    break;
                }
            }
        }
    }
}
