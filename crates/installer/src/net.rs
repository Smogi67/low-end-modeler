//! Downloading over HTTPS with WinHTTP, which is part of Windows. It uses Windows's
//! own certificate checks and proxy settings, and follows GitHub's redirect to its
//! file servers.

use std::ptr::null;

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};

use crate::core::Download;

/// Refuses anything this big, which no expected file comes near.
const MOST_BYTES: usize = 64 * 1024 * 1024;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle came from WinHTTP and is closed once.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// Explains a WinHTTP failure in plain words.
fn failure(doing: &str) -> String {
    // SAFETY: no arguments.
    let code = unsafe { GetLastError() };
    let reason = match code {
        12002 => "the connection timed out",
        12007 => "the server name could not be found; is the internet connected?",
        12029 | 12030 => "the connection failed or was cut off",
        12175 | 12045 | 12057 | 12038 | 12037 => {
            "the secure connection could not be checked; is the computer's date and time right?"
        }
        _ => "Windows reported an error",
    };
    format!("Could not {doing}: {reason} (error {code}).")
}

/// Downloads the file, calling `progress` with the bytes so far and the total.
pub fn download(item: &Download, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let agent = wide(concat!("MixerSetup/", env!("CARGO_PKG_VERSION")));
    // SAFETY: each call gets valid zero-ended strings or null where allowed, and every
    // handle is checked before use and closed by `Handle`.
    unsafe {
        let mut session = Handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ));
        if session.0.is_null() {
            // Windows before 8.1 lacks automatic proxy detection.
            session = Handle(WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                null(),
                null(),
                0,
            ));
        }
        if session.0.is_null() {
            return Err(failure("start a download"));
        }
        WinHttpSetTimeouts(session.0, 30_000, 30_000, 30_000, 60_000);
        let host = wide(item.host);
        let connection = Handle(WinHttpConnect(session.0, host.as_ptr(), 443, 0));
        if connection.0.is_null() {
            return Err(failure(&format!("connect to {}", item.host)));
        }
        let (verb, path) = (wide("GET"), wide(item.path));
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(failure("prepare the download"));
        }
        if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, std::ptr::null_mut()) == 0
        {
            return Err(failure(&format!("reach {}", item.host)));
        }

        let number = |which: u32| -> Option<u32> {
            let mut value = 0u32;
            let mut size = std::mem::size_of::<u32>() as u32;
            let ok = WinHttpQueryHeaders(
                request.0,
                which | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut value as *mut u32).cast(),
                &mut size,
                std::ptr::null_mut(),
            );
            (ok != 0).then_some(value)
        };
        let status = number(WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(format!(
                "The server answered with code {status} instead of the file. The download \
                 address may have moved; please report this."
            ));
        }
        let total = number(WINHTTP_QUERY_CONTENT_LENGTH).map_or(item.size, u64::from);

        let mut body: Vec<u8> = Vec::with_capacity(total as usize);
        loop {
            let mut available = 0u32;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(failure("keep downloading"));
            }
            if available == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            if WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            ) == 0
            {
                return Err(failure("keep downloading"));
            }
            body.truncate(start + read as usize);
            if body.len() > MOST_BYTES {
                return Err("The download is far bigger than expected, so it was stopped.".into());
            }
            progress(body.len() as u64, total.max(body.len() as u64));
        }
        Ok(body)
    }
}
